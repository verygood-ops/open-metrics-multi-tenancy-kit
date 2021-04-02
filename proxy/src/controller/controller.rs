use std::collections::HashSet;
use std::time::Duration;

use kube::Client;
use log::{debug,error,info,warn};
use tokio::sync::RwLock;
use tokio::time::sleep;
use once_cell::sync::Lazy;

use kube_metrics_mutli_tenancy_lib as kube_lib;

// An ingestion controller singleton
// It is protected by global rw lock, which is acquired for writers
// when doing k8s state change,
// and for readers on every proxy request.
pub static CONTROLLER: Lazy<RwLock<IngestionTenantController>> =
    Lazy::new(|| RwLock::new(IngestionTenantController::new()));

// Ingestion controller state
pub struct IngestionTenantController {
    k8s_poll_ms: u64,
    k8s_client: Option<Client>,
    initial_tenants: HashSet<String>,
    tenants: HashSet<String>,
    tenants_vec: Vec<String>,
    namespace: String,
    stopping: Option<bool>
}


// An ingestion controller informer trait
impl IngestionTenantController {

    // Instantiate with empty values.
    pub fn new() -> IngestionTenantController   {
        return IngestionTenantController{
            k8s_poll_ms: 0,
            initial_tenants: HashSet::new(),
            k8s_client: None,
            tenants: HashSet::new(),
            tenants_vec: Vec::new(),
            namespace: std::env::var("OPEN_METRICS_PROXY_NAMESPACE").unwrap_or("default".into()),
            stopping: None
        }
    }

    // Clean helper
    pub fn clean(&mut self) {
        self.initial_tenants = HashSet::new();
        self.tenants = HashSet::new();
        self.tenants_vec = Vec::new();
        self.stopping = None;
    }

    pub fn set_stopping(&mut self) {
        self.stopping = Some(true);

    }

    // Initialize poll seconds parameter.
    pub fn set_k8s_poll_delay(&mut self, k8s_poll_ms: u64) -> &mut IngestionTenantController {
        self.k8s_poll_ms = k8s_poll_ms;
        return self
    }

    // Initialize tenants from command line.
    pub fn set_initial_allowed_tenants(&mut self, initial_allowed_tenants: Vec<String>) -> &mut IngestionTenantController {
        for tenant_id in  initial_allowed_tenants.iter().cloned().into_iter() {
            self.initial_tenants.insert(tenant_id);
        };

        for tenant_id in self.initial_tenants.iter().cloned().into_iter() {
            self.tenants_vec.push(tenant_id);
        }
        return self
    }

    // Add tenant to list of observed ones.
    // This method is not thread safe!
    fn add_tenant(&mut self, tenant_id: &String) {
        // Check that tenant have not been specified via command line
        if !self.initial_tenants.contains(tenant_id) {
            // Check that tenant have not been added already
            if !self.tenants.contains(tenant_id) {

                debug!(
                    "Going to insert tenant ID {}",
                    tenant_id
                );

                // Add value to vector.
                self.tenants_vec.push(tenant_id.clone());
                // Add value to hashset
                self.tenants.insert(tenant_id.clone());

                info!(
                    "Tenant ID {} had been inserted",
                    tenant_id
                )
            } else {
                warn!(
                    "Tenant ID {} is already being ingested",
                    tenant_id
                )
            }
        } else {
            warn!(
                "Tenant ID {} is already being ingested",
                tenant_id
            )
        }
    }

    // Remove tenant from a set of being observed.
    // This method is not thread safe!
    fn remove_tenant(&mut self, tenant_id: &String) {
        if self.initial_tenants.contains(tenant_id) {
            info!(
                "Tenant ID {} is a part of initial tenant set",
                tenant_id
            )
        } else {

            if !self.tenants.contains(tenant_id) {
                warn!(
                    "tenant ID {} was not being observed before",
                    tenant_id
                )
            } else {
                let tid: String = tenant_id.clone();

                let mut idx: Option<u32> = None;
                let mut ctr: u32 = 0;

                debug!(
                    "checking tenant vec of length {} ",
                    self.tenants_vec.len()
                );


                for tid_inner in self.tenants_vec.iter().cloned().into_iter() {
                    if tid_inner.eq(&tid) {
                        idx = Some(ctr);
                    }
                    ctr = ctr + 1;
                };

                if idx.is_some() {
                    // It is safe to do unwrap since is_some() was checked
                    let usz_idx = idx.unwrap() as usize;
                    self.tenants_vec.remove(usz_idx);

                    info!(
                        "Tenant ID {} stopped observation",
                        tenant_id
                    )
                } else {
                    warn!(
                        "Tenant ID {} already removed from vector",
                        tenant_id
                    )
                }
                self.tenants.remove(tenant_id);

            }

        }
    }

    // Calculate difference between k8s and internal state
    // Update tenants
    pub fn observe(&mut self, found_tenants: HashSet<String>) {

        debug!("number of ingestion tenants found : {}", found_tenants.len());

        for tenant_id in found_tenants.iter().cloned().into_iter() {
            // Check if tenant already present
            if !self.initial_tenants.contains(&tenant_id) {
                if !self.tenants.contains(&tenant_id) {
                    self.add_tenant(&tenant_id);
                };
            };
        };

        // Check for removed tenants
        let existing_tenants = self.tenants.clone();
        for tenant_id in existing_tenants.into_iter() {
            if !found_tenants.contains(&tenant_id) {
                self.remove_tenant(&tenant_id)
            };
        }

        debug!("--- Done observe(), got {} tenants ---", self.tenants_vec.len());

    }

    // Get tenants vector to use.
    pub fn get_tenants(&self) -> &Vec<String> {
        return &self.tenants_vec;
    }

    // Initialize k8s if necessary
    pub fn init_k8s(&mut self, cli: Option<Client>) {

        if cli.is_some() {
            self.k8s_client = cli;
        }
    }
}

// Controller worker logic.
// On start, acquire write lock
pub async fn worker(k8s_client: Option<Client>) {
    let mut c = CONTROLLER.write().await;
    c.init_k8s(k8s_client);
    drop(c);
    debug!("initialized k8s client");

    let ctrl = CONTROLLER.read().await;

    if ctrl.k8s_client.is_some() {
        debug!("starting k8s poll loop");
        let ms = ctrl.k8s_poll_ms;
        drop(ctrl);
        // Poll every two minutes
        loop {
            sleep(Duration::from_millis(ms)).await;
            debug!("next tick start");
            // Acquire read lock.
            let ctrl = CONTROLLER.read().await;
            if ctrl.stopping.is_some() {
                break;
            } else {
                let cli = ctrl.k8s_client.clone().unwrap();
                match kube_lib::get_tenant_ids(
                    cli.clone(), &ctrl.namespace).await {
                    Ok(found_tenants) => {
                        debug!("preparing to write tenants");
                        // Drop read lock for current thread.
                        drop(ctrl);

                        // Acquire write lock for current thread..
                        let mut ctrl = CONTROLLER.write().await;
                        // Compute in memory state change.
                        ctrl.observe(found_tenants);
                        // Drop write lock.
                        drop(ctrl);
                    },
                    Err(msg) => {
                        error!("failed to acquire tenants, will not observe(): {}", msg);
                    }
                };
            }

        };
        let mut ctrl = CONTROLLER.write().await;
        ctrl.stopping = None;
        drop(ctrl);
        debug!("restored controller");
    } else {
        info!("kubernetes controller is not started due to poll interval being zero");
    };
}

#[cfg(test)]
mod tests {
    use kube::{Client, Service};

    use futures::pin_mut;
    use http::{Request, Response};
    use hyper::Body;
    use env_logger;
    use log::debug;
    use tower_test::mock;

    use std::collections::HashSet;
    use std::iter::FromIterator;
    //use crate::{discover_open_metrics_rules, discover_tenant_ids, get_tenant_ids};
    use k8s_openapi::serde_json::Value;
    use tokio::time::sleep;
    use std::time::Duration;
    use crate::controller::controller::worker;
    use std::borrow::Borrow;
    use serial_test::serial;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn test_fixture_1() -> Value {
        serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "List",
                    "metadata": {
                        "resourceVersion": "",
                        "selfLink": ""
                    },
                    "items": [
                        // First test rule
                        {
                            "apiVersion": "open-metrics.vgs.io",
                            "kind": "OpenMetricsRule",
                            "metadata": {
                                "name": "test1",
                                "namespace": "default",
                                "annotations": { "kube-rs": "test-2" },
                            },
                            "spec": {
                                "tenants": [
                                    "tenant7",
                                    "tenant8"
                                ]
                            }
                        },
                        // Second test rule
                        {
                            "apiVersion": "open-metrics.vgs.io",
                            "kind": "OpenMetricsRule",
                            "metadata": {
                                "name": "test19",
                                "namespace": "default",
                                "annotations": { "kube-rs": "test-2" },
                            },
                            "spec": {
                                "tenants": [
                                    "tenant1",
                                    "tenant2"
                                ]
                            }
                        }
                    ]
                })
    }

    fn test_fixture_2() -> Value {
        serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "List",
                    "metadata": {
                        "resourceVersion": "",
                        "selfLink": ""
                    },
                    "items": [
                        // First test rule
                        {
                            "apiVersion": "open-metrics.vgs.io",
                            "kind": "OpenMetricsRule",
                            "metadata": {
                                "name": "test1",
                                "namespace": "default",
                                "annotations": { "kube-rs": "test-2" },
                            },
                            "spec": {
                                "tenants": [
                                    "tenant7",
                                    "tenant9"
                                ]
                            }
                        }
                    ]
                })
    }

    #[tokio::test]
    #[serial]
    pub async fn test_controller_tenant_added() {
        init();
        let (mock_service, handle) = mock::pair::<Request<Body>, Response<Body>>();

        let spawned = tokio::spawn(async move {
            // Receive a request for pod and respond with some data
            pin_mut!(handle);
            let (request, send) = handle.next_request().await.expect("service not called");
            assert_eq!(request.method(), http::Method::GET);
            assert_eq!(request.uri().to_string(), "/apis/open-metrics.vgs.io/v1/namespaces/default/openmetricsrules?");
            let l = test_fixture_1();
            let rules = serde_json::to_string(&l).unwrap();
            send.send_response(
                Response::builder()
                    .body(Body::from(rules))
                    .unwrap(),
            );
        });

        // `kube::Service` takes `tower::Service` with request/response of `hyper::Body`
        let service = Service::new(mock_service);
        let k8s_client = Client::new(service);

        // Set controller iteration time
        let mut controller = crate::CONTROLLER.write().await;
        controller.set_k8s_poll_delay(800);
        drop(controller);

        let worker_handle = tokio::spawn(worker(Some(k8s_client.clone())));
        // Wait some time for tenant updates to propagate
        sleep(Duration::from_secs(1)).await;
        debug!("ticked!");
        // Acquire controller and make sure all tenants are seen
        let controller = crate::CONTROLLER.read().await;

        let expected_tenants: HashSet<String> = HashSet::from_iter(
            vec![
                String::from("tenant1"),
                String::from("tenant2"),
                String::from("tenant7"),
                String::from("tenant8"),
            ]
        );

        let found_tenants: HashSet<String> = HashSet::from_iter(controller.get_tenants().iter().cloned().into_iter());

        assert_eq!(expected_tenants, found_tenants);

        spawned.await.unwrap();
        drop(controller);

        // Clean up global state
        let mut controller = crate::CONTROLLER.write().await;
        controller.set_stopping();
        sleep(Duration::from_secs(1)).await;
        controller.clean();
        worker_handle.abort();
        drop(controller);
    }

    #[tokio::test]
    #[serial]
    pub async fn test_controller_tenant_removed() {
        init();
        let (mock_service, handle) = mock::pair::<Request<Body>, Response<Body>>();

        let spawned = tokio::spawn(async move {
            // Receive a request for pod and respond with some data
            pin_mut!(handle);
            let (request, send) = handle.next_request().await.expect("service not called");
            assert_eq!(request.method(), http::Method::GET);
            assert_eq!(request.uri().to_string(), "/apis/open-metrics.vgs.io/v1/namespaces/default/openmetricsrules?");
            let l = test_fixture_2();
            let rules = serde_json::to_string(&l).unwrap();
            send.send_response(
                Response::builder()
                    .body(Body::from(rules))
                    .unwrap(),
            );
        });

        // `kube::Service` takes `tower::Service` with request/response of `hyper::Body`
        let service = Service::new(mock_service);
        let k8s_client = Client::new(service);

        // Set controller iteration time
        let mut controller = crate::CONTROLLER.write().await;
        controller.set_k8s_poll_delay(800);
        for tenant in vec![
            String::from("tenant1"),
            String::from("tenant2"),
            String::from("tenant7"),
            String::from("tenant8"),
        ] {
            controller.add_tenant(tenant.borrow());
        };

        // Make sure all tenants have been initialized by add_tenant()

        let expected_tenants: HashSet<String> = HashSet::from_iter(
            vec![
                String::from("tenant1"),
                String::from("tenant2"),
                String::from("tenant7"),
                String::from("tenant8"),
            ]
        );

        let found_tenants: HashSet<String> = HashSet::from_iter(controller.get_tenants().iter().cloned().into_iter());

        assert_eq!(expected_tenants, found_tenants);
        drop(controller);

        let worker_handle = tokio::spawn(worker(Some(k8s_client.clone())));
        // Wait some time for tenant updates to propagate
        sleep(Duration::from_secs(1)).await;
        debug!("ticked!");
        // Acquire controller and make sure all tenants are seen
        let controller = crate::CONTROLLER.read().await;

        let expected_tenants_2: HashSet<String> = HashSet::from_iter(
            vec![
                String::from("tenant7"),
                String::from("tenant9"),
            ]
        );

        let found_tenants_2: HashSet<String> = HashSet::from_iter(controller.get_tenants().iter().cloned().into_iter());

        assert_eq!(expected_tenants_2, found_tenants_2);

        spawned.await.unwrap();
        drop(controller);

        // Clean up global state
        let mut controller = crate::CONTROLLER.write().await;
        controller.set_stopping();
        sleep(Duration::from_secs(1)).await;
        controller.clean();
        worker_handle.abort();
        drop(controller);
    }

}
