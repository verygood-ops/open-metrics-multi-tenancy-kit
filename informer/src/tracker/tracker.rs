use kube_metrics_mutli_tenancy_lib as kube_lib;
use serde_yaml;

use log::{debug,error,info,trace,warn};
use std::collections::{HashMap,HashSet};
use std::time::Duration;

use reqwest::Client as RClient;
use kube::{Api,Client,api::ListParams};
use prometheus::Counter;
use tokio::time::interval;
use kube_metrics_mutli_tenancy_lib::GroupSpec;


// Retrieve ingestion tenants to be used for metrics discovery
pub async fn discover_ingestion_tenants(k8s_client: Client, namespace: &String) -> HashSet<String> {

    let mut found_tenants = HashSet::new();
    let client = k8s_client.clone();
    // Call Kubernetes to check ingestion tenant resources.

    let (tenants_acquired, tenants_portion, next_token) =
        kube_lib::refresh_ingestion_tenants(
            client.clone(),
            None,
            &namespace.clone()).await;

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
            let (tenants_acquired_inner,
                tenants_portion, _t) = kube_lib::refresh_ingestion_tenants(
                client.clone(), Some(token), &namespace.clone()).await;
            if !tenants_acquired_inner {
                error!("Failed to acquire tenants from k8s, will retry later.");
                continue;
            } else {
                //
                for tenant in tenants_portion {
                    found_tenants.insert(tenant.clone());
                }
                if !_t.is_some() {
                    debug!("Finished acquiring tenants from k8s");
                    break
                } else {
                    t = _t.unwrap().clone();
                }
            };
            // End inner loop.
        }
    };
    return found_tenants;

}


// For the sake of simplicity, this discovers rules only for clients in
pub async fn discover_cortex_rules(
    tenants: &Vec<String>,
    ruler_client: RClient,
    ruler_api_url: &String,
) -> Result<HashMap<String, Vec<GroupSpec>>,String> {

    let failed = false;
    let r_client = ruler_client.clone();
    let mut result: HashMap<String, Vec<GroupSpec>> = HashMap::new();

    let url = ruler_api_url.clone() + "api/v1/rules";

    debug!("Cortex URL is {}", url);
    let mut failed = false;
    for tenant_id in tenants {

        let response = r_client.get(&url)
        .header("X-Scope-OrgID", tenant_id)
        .send()
        .await;

        let resp = match response {
            Ok(r) => Some(r),
            Err(f) => {
                error!("failed to communicate with ruler: {}", f);
                None
            }
        };
        if resp.is_some() {
            let r = resp.unwrap();
            let s = r.status();
            if s == 200 {
                let body = match r.text().await {
                    Ok(s) => {
                        Some(s)
                    },
                    Err(_) => {
                        warn!("failed to retrieve response body on 200!");
                        None
                    }
                };
                if body.is_some() {
                    let g: Result<HashMap<String, Vec<GroupSpec>>,
                        serde_yaml::Error> = match serde_yaml::from_str(&body.unwrap()) {
                            Ok(y) => Ok(y),
                            Err(e) => Err(e)
                    };
                    if g.is_ok() {
                        let groups = g.unwrap();
                        if groups.len() > 0 {
                            for namespace_groups_vec in groups
                                .values().into_iter() {
                                    let mut result_groups = Vec::new();
                                    for group in namespace_groups_vec.into_iter() {
                                        debug!("found group named {} with {} rules inside",
                                              group.name, group.rules.len());
                                        result_groups.push(group.clone());
                                    };
                                    result.insert(tenant_id.clone(), result_groups.clone());
                            }
                        };

                    } else {
                        error!("received invalid YAML from ruler");
                        failed = true;
                        break;
                    }
                };
                trace!("received ruler reponse")
            } else if s == 404 {
                let text = match r.text().await {
                    Ok(s) => s,
                    Err(_) => {
                        warn!("failed to retrieve response body");
                        String::from("")
                    }
                };
                if text.trim().eq("no rule groups found") {
                    debug!("response code is 404, perhaps there are no rules: {}, text: {}",
                          s, text);
                } else {
                    debug!("response code is 404, perhaps you call a wrong url: {}, text: {}",
                          s, text);
                    failed = true;
                    break;
                };
            } else {
                warn!("unknown ruler status code");
                failed = true;
                break;
            };
        };

    };

    if !failed {
        Ok(result)
    } else {
        Err(String::from("failed to retrieve rules"))
    }
}


// Periodically discover rules from Cortex.
// Update Rules CRD in k8s
pub async fn tracker(k8s_client: Client,
                     ruler_client: RClient,
                     ruler_api_url: String,
                     num_rules: Box<Counter>,
                     num_tenants: Box<Counter>, ms: u64) {

    let mut interval = interval(Duration::from_millis(ms));
    let namespace = std::env::var("OPEN_METRICS_INFORMER_NAMESPACE")
        .unwrap_or("default".into());

    loop {
        interval.tick().await;
        debug!("Done tracker tick!");
        let tenants: Vec<String> = discover_ingestion_tenants(k8s_client.clone(),
                                                              &namespace).await.into_iter().collect();
        let num_tenants = tenants.len();

        debug!("Going to discover rules for {} tenants.", num_tenants);

        match discover_cortex_rules(
            &tenants,
            ruler_client.clone(),
            &ruler_api_url.clone(),
            &namespace,
        ).await {
            Ok(r) => {
                let mut tenants_found = 0;
                let mut groups_found = 0;
                let mut rules_found = 0;
                for k in r.keys().into_iter() {
                    tenants_found += 1;
                    let vg = r.get(k).unwrap();
                    for g in vg {
                        groups_found += 1;
                        for r in g.rules.iter().cloned().into_iter() {
                            rules_found += 1;
                        }
                    }
                };
                info!("Discovered {} rules in {} groups for {} tenants",
                      rules_found, groups_found, tenants_found);
                // TODO: create rules that not exist yet, sync manual changes into k8s
            },
            Err(e) => {
                error!("Failed to discover rules: {}", e)
            }
        };

    };
}
