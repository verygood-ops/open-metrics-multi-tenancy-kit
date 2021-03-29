use kube_metrics_mutli_tenancy_lib as kube_lib;
use serde_yaml;

use log::{debug,error,info,trace,warn};
use std::collections::HashSet;
use std::iter::FromIterator;
use std::time::Duration;

use reqwest::Client as RClient;
use kube::{Api,Client,api::ListParams};
use prometheus::Counter;
use tokio::time::interval;

use crate::rules::rules;


// Periodically discover rules from Cortex.
// Update Rules CRD in k8s
pub async fn tracker(k8s_client: Client,
                     ruler_client: RClient,
                     ruler_api_url: String,
                     num_rules: Box<Counter>,
                     num_tenants: Box<Counter>,
                     ms: u64,
                     skip_resource_removal: bool) {

    let mut interval = interval(Duration::from_millis(ms));
    let namespace = std::env::var("OPEN_METRICS_INFORMER_NAMESPACE")
        .unwrap_or("default".into());

    loop {
        interval.tick().await;

        match kube_lib::get_tenant_ids(
            k8s_client.clone(), &namespace).await {
            Ok(tenants_rules) => {
                debug!("Going to discover rules for {} tenants.", tenants_rules.len());
                let tenant_vec: Vec<String> = Vec::from_iter(tenants_rules);
                match rules::discover_ruler_rules(
                    &tenant_vec,
                    ruler_client.clone(),
                    &ruler_api_url.clone(),
                    &namespace,
                ).await {
                    Ok(r) => {
                        let mut tenants_found: u32 = 0;
                        let mut groups_found: u32 = 0;
                        let mut rules_found: u32 = 0;
                        for k in r.keys().into_iter() {
                            tenants_found += 1;
                            num_tenants.inc();
                            let vg = r.get(k).unwrap();
                            for g in vg {
                                groups_found += 1;
                                for r in g.rules.iter().cloned().into_iter() {
                                    rules_found += 1;
                                    num_rules.inc();
                                }
                            }
                        };
                        info!("tracker: discovered {} rules in {} groups for {} tenants",
                              rules_found, groups_found, tenants_found);

                        match kube_lib::discover_open_metrics_rules(k8s_client.clone(), &namespace).await {
                            Ok(k8s_rules) => {
                                let tenant_k8s_specs = rules::get_tenant_map_from_rules_list(k8s_rules);
                                let (updates, removals) =
                                    rules::diff_rule_groups(tenant_k8s_specs, r);

                                for (tenant_id, update_groups) in updates.clone().into_iter() {
                                    for group in update_groups {
                                        info!("TRACKER: Going to ADD {:?} to k8s {} tenant", group, tenant_id);
                                    }
                                };

                                if !skip_resource_removal {
                                    for (tenant_id, remove_groups) in removals.clone().into_iter() {
                                        for group in remove_groups {
                                            info!("TRACKER: Going to REMOVE {:?} from k8s {} tenant", group, tenant_id);
                                        }
                                    };
                                };
                            },
                            Err(msg) => {
                                error!("tracker: failed to discover k8s rules: {}", msg);
                            }
                        }

                    },
                    Err(msg) => {
                        error!("tracker: failed to discover ruler rules: {}", msg)
                    }
                };
            },
            Err(msg) => {
                error!("tracker: tenants acquire from k8s failed: {}", msg);
            }
        };


        debug!("Done tracker tick!");

    };
}
