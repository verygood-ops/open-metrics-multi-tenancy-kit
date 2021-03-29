use kube_metrics_mutli_tenancy_lib as kube_lib;
use log::{debug,info,error};

use kube::Client;
use reqwest::Client as RClient;
use prometheus::Counter;
use tokio::time::interval;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::time::Duration;

use crate::crud::crud;
use crate::rules::rules;

// Periodically discover rules from Cortex.
// Update Rules CRD in k8s
pub async fn updater(k8s_client: Client,
                     ruler_client: RClient,
                     ruler_api_url: String,
                     num_rules: Box<Counter>,
                     num_tenants: Box<Counter>,
                     ms: u64,
                     skip_ruler_group_removal: bool) {
    let mut interval = interval(Duration::from_millis(ms));
    let namespace = std::env::var("OPEN_METRICS_INFORMER_NAMESPACE")
        .unwrap_or("default".into());

    loop {
        interval.tick().await;

        match kube_lib::discover_open_metrics_rules(
            k8s_client.clone(),
            &namespace.clone()
        ).await {
            Ok(k8s_rules) => {
                let rules_clone = Vec::from_iter(k8s_rules.iter().cloned().into_iter());
                let tenants = kube_lib::discover_tenant_ids(k8s_rules);
                match rules::discover_ruler_rules(
                    &Vec::from_iter(tenants),
                    ruler_client.clone(),
                    &ruler_api_url.clone(),
                    &namespace.clone()
                ).await {
                    Ok(tenant_specs_ruler) => {
                        let mut tenant_k8s_specs =
                            rules::get_tenant_map_from_rules_list(rules_clone);

                        let (
                            rule_updates_add,
                            rule_updates_remove
                        ) = rules::diff_rule_groups(
                            tenant_specs_ruler,
                            tenant_k8s_specs);

                        for (tenant_id, update_groups)
                                in rule_updates_add.clone().into_iter() {
                            for group in update_groups {
                                info!("UPDATER: Going to ADD {:?} to {} tenant", group, tenant_id);
                                crud::update_ruler_rule(
                                    ruler_client.clone(),
                                    &ruler_api_url.clone(),
                                    &tenant_id.clone(),
                                    &namespace.clone(),
                                    group
                                ).await;
                            }
                        };

                        if !skip_ruler_group_removal {
                            for (tenant_id, remove_groups)
                                    in rule_updates_remove.clone().into_iter() {
                                for group in remove_groups {
                                    info!(
                                        "UPDATER: Going to REMOVE {:?} from {} tenant",
                                        group, tenant_id);
                                    crud::remove_ruler_rule(
                                        ruler_client.clone(),
                                        &ruler_api_url.clone(),
                                        &tenant_id.clone(),
                                        &namespace.clone(),
                                        group
                                    );
                                };
                            };
                        };


                    },
                    Err(msg) => {
                        error!("failed to discover ruler rules");
                    }
                }
            },
            Err(msg) => {
                error!("failed to discover k8s rules");
            }
        };

        debug!("Done updater tick");

    };
}
