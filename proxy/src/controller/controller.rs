use crate::ingestiontenant;
use std::collections::HashSet;
use std::time::Duration;

use kube::{Api,Client};
use kube::api::ListParams;
use log::{debug,error,info,warn};
use tokio::sync::RwLock;
use tokio::time::interval;
use once_cell::sync::Lazy;

// ingestion controller singleton
pub static CONTROLLER: Lazy<RwLock<IngestionTenantController>> = Lazy::new(|| RwLock::new(IngestionTenantController::new()));


pub struct IngestionTenantController {
    k8s_poll_ms: u64,
    k8s_client: Option<Client>,
    initial_tenants: HashSet<String>,
    tenants: HashSet<String>,
    tenants_vec: Vec<String>,
    namespace: String
}


// An informer trait
impl IngestionTenantController {

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

    // Initialize poll seconds parameter
    pub fn set_k8s_poll_delay(&mut self, k8s_poll_ms: u64) -> &mut IngestionTenantController {
        self.k8s_poll_ms = k8s_poll_ms;
        return self
    }

    // Initialize tenants from command line
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
                // Achieve barrier

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

    // Get tenants
    pub async fn refresh_ingestion_tenants(&self,
                                       continue_token: Option<String>) -> (Vec<String>, Option<String>) {


        let client = self.k8s_client.clone().unwrap();
        // Call Kubernetes to check ingestion tenant resources.
        let api : Api<ingestiontenant::ingestiontenant::IngestionTenant> = Api::namespaced(client, &self.namespace);
        let lp = if continue_token.is_some() {
            ListParams::default().continue_token(&continue_token.unwrap().clone())
        } else {
            ListParams::default()
        };

        let ingestion_tenants_list = api.list(&lp).await.unwrap();

        let mut tenants: Vec<String> = Vec::new();

        for i_t in ingestion_tenants_list.items.into_iter() {
            for tenant in i_t.spec.tenants {
                tenants.push(tenant.clone());
            };
        };

        (tenants, ingestion_tenants_list.metadata.continue_)
    }

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

            let mut found_tenants: HashSet<String> = HashSet::new();

            let ctrl = CONTROLLER.read().await;
            let (tenants_portion, next_token) = ctrl.refresh_ingestion_tenants(None).await;
            drop(ctrl);

            for tenant in tenants_portion {
                found_tenants.insert(tenant.clone());
            }
            let token_init = next_token.unwrap().clone();

            // Retrieve rest of data
            if token_init.len() > 0 {
                info!("Token: {}", token_init);
                let mut t = token_init.clone();

                loop {
                    let token = t.clone();
                    let ctrl = CONTROLLER.read().await;
                    let (tenants_portion, _t) = ctrl.refresh_ingestion_tenants(Some(token)).await;
                    drop(ctrl);

                    for tenant in tenants_portion {
                        found_tenants.insert(tenant.clone());
                    }
                    if !_t.is_some() {
                        break
                    } else {
                        t = _t.unwrap().clone();
                    }
                }
            }
            debug!("preparing to write tenants");
            let mut ctrl = CONTROLLER.write().await;
            ctrl.observe(found_tenants);
            drop(ctrl);
        }
    };

}
