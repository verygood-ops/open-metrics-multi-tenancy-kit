use std::collections::HashSet;
use std::time::Duration;

use kube::Client;
use log::{debug,error,info,warn};
use tokio::sync::RwLock;
use tokio::time::interval;
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
    namespace: String
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
            namespace: std::env::var("OPEN_METRICS_PROXY_NAMESPACE").unwrap_or("default".into())
        }
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
    pub async fn init_k8s(&mut self) {
        if self.k8s_poll_ms > 0 {
            // Initialize k8s client
            self.k8s_client = match Client::try_default().await {
                Ok(v) => Some(v),
                Err(e) => {
                    error!("Failed to instantiate k8s client: {}", e.to_string());
                    None
                }
            };
        };
    }
}

// Controller worker logic.
// On start, acquire write lock
pub async fn worker() {
    let mut c = CONTROLLER.write().await;
    c.init_k8s().await;
    drop(c);

    let ctrl = CONTROLLER.read().await;

    if ctrl.k8s_client.is_some() {
        let ms = ctrl.k8s_poll_ms;
        drop(ctrl);
        // Poll every two minutes
        let mut interval = interval(Duration::from_millis(ms));
        loop {
            interval.tick().await;

            // Acquire read lock.
            let ctrl = CONTROLLER.read().await;
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
        };
    };
}
