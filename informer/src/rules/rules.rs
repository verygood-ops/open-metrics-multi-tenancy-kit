use std::collections::HashMap;

use kube_metrics_mutli_tenancy_lib as kube_lib;
use log::{debug,error,trace,warn};
use reqwest::Client as RClient;


// This method is complex, since it is instruments both k8s and cortex.
// 1. Get tenants list
pub async fn discover_ruler_rules(
    tenants: &Vec<String>,
    ruler_client: RClient,
    ruler_api_url: &String,
    namespace: &String
) -> Result<HashMap<String, Vec<(kube_lib::GroupSpec, i64)>>,String> {

    let r_client = ruler_client.clone();
    let mut result: HashMap<String, Vec<(kube_lib::GroupSpec, i64)>> = HashMap::new();

    let url = ruler_api_url.clone() + "api/v1/rules/" + &namespace.clone();

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
                    let g: Result<HashMap<String, Vec<kube_lib::GroupSpec>>,
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
                                    // we use zero as i64 here, because ruler updates
                                    // do not carry k8s updates together
                                    result_groups.push((group.clone(), -1));
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

// Find group with given name in the vector of groups, and return it together with index
pub fn find_group_named<'a>(group_list: &'a Vec<(kube_lib::GroupSpec, i64)>, group_name: &String) -> (u32, Option<&'a kube_lib::GroupSpec>, i64) {
    let mut idx: u32 = 0;
    let s = -1;
    for (group, s) in group_list {
        if &group.name == group_name {
            return (idx.into(), Some(group), s.clone())
        } else {
            idx += 1;
        }
    }
    (0, None, s)
}

// Equality check for group specs
fn are_groups_equal(original: &kube_lib::GroupSpec, copy: &kube_lib::GroupSpec) -> bool {
    return kube_lib::GroupSpec::eq(original, copy);
}

// given tenant id and group tenant hashmap, either get existing tenant group vector,
// or create new vector
fn get_or_create_action<'ret, 'src:'ret, 'a>(
    tenant_id: &'a String,
    groups_tenant_hashmap: &'src mut HashMap<String,Vec<(kube_lib::GroupSpec, i64)>>
) -> Vec<(kube_lib::GroupSpec, i64)> {
    if groups_tenant_hashmap.contains_key(tenant_id) {
        groups_tenant_hashmap.get_mut(tenant_id).unwrap().to_owned()
    } else {
        Vec::new().to_owned()
    }
}


// Calculate rule updates for each tenant
pub fn diff_rule_groups(
    target_rules: HashMap<String,Vec<(kube_lib::GroupSpec, i64)>>,
    origin_rules: HashMap<String,
    Vec<(kube_lib::GroupSpec, i64)>>
) -> (HashMap<String, Vec<(kube_lib::GroupSpec, i64)>>, HashMap<String, Vec<(kube_lib::GroupSpec, i64)>>) {
    let mut updates = HashMap::new();
    let mut removals = HashMap::new();

    // Calculate updates: all stuff different in origin
    for tenant_id in origin_rules.keys().into_iter() {
        debug!("origin tenant {}", tenant_id);
        // it is safe to unwrap, since tenant_id had appeared iterating from origin_rules
        let origin_rule_groups_list = origin_rules.get(tenant_id)
            .unwrap().iter().cloned().into_iter();
        debug!("num rules in origin for tenant {}: {}", tenant_id, origin_rule_groups_list.len());
        let mut updates_vec =
            get_or_create_action(tenant_id, &mut updates);
        if !target_rules.contains_key(tenant_id) {
            // all rules go to updates
            updates_vec.extend(origin_rule_groups_list.into_iter());
        } else {
            // safe to unwrap, since checked for !contains_key() above
            let target_rule_groups_list = target_rules.get(tenant_id).unwrap();
            for (origin_group, k8s_index) in origin_rule_groups_list.into_iter() {
                // find whether group from origin already exists in target
                match find_group_named(
                    &target_rule_groups_list,
                    &origin_group.name
                ) {
                    (_i, Some(g), _k_i) => {
                        if !are_groups_equal(&origin_group, &g) {
                            // a group exists but it is not equal
                            debug!("group named {} for tenant {} are different in target and origin",
                                origin_group.name, tenant_id);
                            updates_vec.push((origin_group, k8s_index));
                        } else {
                            // a group exists and equal
                            debug!("group named {} for tenant {} already synced",
                                   origin_group.name, tenant_id);
                        }
                    },
                    (_i, None, _k_i) => {
                        // no group exists yet, need to create
                        debug!("no group with name {} exists in target", origin_group.name);
                        updates_vec.push((origin_group, k8s_index));
                    }
                };
            };
        };
        if updates_vec.len() > 0 {
            // Prepare updates
            updates.insert(tenant_id.clone(), updates_vec.to_owned());
        };
    };

    // Calculate removals: all stuff in target but not in main
    for tenant_id in target_rules.keys().into_iter() {
        debug!("target tenant {}", tenant_id);
        // it is safe to unwrap, since tenant_id is sourced from target_rules keys
        let target_rule_groups_list =
            target_rules.get(tenant_id).unwrap().iter().cloned().into_iter();
        let mut removals_vec =
            get_or_create_action(tenant_id, &mut removals);

        if !origin_rules.contains_key(tenant_id) {
            // tenant have been removed, should be safe to remove his rules
            debug!("no groups at all for tenant {} in origin", tenant_id);
            removals_vec.extend(target_rule_groups_list);
        } else {
            // safe to unwrap, since checked for contains_key() above
            let origin_rule_groups_list = origin_rules.get(tenant_id).unwrap();
            for (target_group, k_i) in target_rule_groups_list {
                match find_group_named(
                    &origin_rule_groups_list,
                    &target_group.name) {
                    (_i, Some(g), _k_i) => {
                        debug!("group with name {} exists in both target and origin", g.name);
                    },
                    (_i, None, _k_i) => {
                        debug!("no group named {} for tenant {}", target_group.name, tenant_id);
                        removals_vec.push((target_group, k_i));
                    }
                }
            };
        };
        if removals_vec.len() > 0 {
            // Prepare removals
            removals.insert(tenant_id.clone(), removals_vec.to_owned());
        }
    };

    return (HashMap::from(updates), HashMap::from(removals))
}

// Given list of OpenMetricsRule, get hash map of (tenant_id) -> (list of egligible rules)
pub fn get_tenant_map_from_rules_list(open_metrics_rules: Vec<kube_lib::OpenMetricsRule>)
    -> HashMap<String, Vec<(kube_lib::GroupSpec, i64)>> {
    let mut tenant_specs_k8s: HashMap<String, Vec<(kube_lib::GroupSpec, i64)>> = HashMap::new();
    let mut ctr = 0;
    for tenant_rule in open_metrics_rules.iter().cloned().into_iter() {
        for tenant_id in tenant_rule.spec.tenants {
            if !tenant_specs_k8s.contains_key(&tenant_id) {
                let vec = Vec::new();
                tenant_specs_k8s.insert(tenant_id.clone(), vec);
            }
            let spec_vec =  tenant_specs_k8s.get_mut(&tenant_id).unwrap();
            for group in tenant_rule.spec.groups.iter().cloned().into_iter() {
                spec_vec.push((group, ctr));
            }
        }
        ctr += 1;
    }
    return tenant_specs_k8s
}
