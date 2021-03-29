use kube_metrics_mutli_tenancy_lib as kube_lib;
use serde_yaml;

use log::{debug,error,info,trace,warn};
use std::collections::HashMap;

use reqwest::Client as RClient;
use std::panic::resume_unwind;
use std::borrow::BorrowMut;


// For the sake of simplicity, this discovers rules only for clients in
pub async fn discover_ruler_rules(
    tenants: &Vec<String>,
    ruler_client: RClient,
    ruler_api_url: &String,
    namespace: &String
) -> Result<HashMap<String, Vec<kube_lib::GroupSpec>>,String> {

    let r_client = ruler_client.clone();
    let mut result: HashMap<String, Vec<kube_lib::GroupSpec>> = HashMap::new();

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


fn find_group_named(group_list: Vec<kube_lib::GroupSpec>, group_name: &String) -> Option<kube_lib::GroupSpec> {
    for group in group_list {
        if &group.name == group_name {
            return Some(group)
        }
    }
    None
}

fn map_eq(a: HashMap<String,String>, b: HashMap<String,String>) -> bool {
    let mut neq = false;
    for k in a.keys() {
        if !(b.contains_key(k) && b.get(k) == a.get(k)) {
            neq = true;
            break;
        }
    }
    for k in b.keys() {
        if !(a.contains_key(k) && a.get(k) == b.get(k)) {
            neq = true;
            break;
        }
    }
    return !neq;
}

fn find_rule_named(rule_list: Vec<kube_lib::RuleSpec>, compare_to: kube_lib::RuleSpec) -> bool {
    let mut found = false;
    for rule in rule_list {

        let compare_rule = compare_to.clone();

        let labels_eq = if rule.labels.is_some() {
            if compare_rule.labels.is_some() {
                map_eq(rule.labels.unwrap(), compare_rule.labels.unwrap())
            } else {false}
        } else {compare_rule.labels.is_none()};

        let anns_eq = if rule.annotations.is_some() {
            if compare_rule.annotations.is_some() {
                map_eq(rule.annotations.unwrap(), compare_rule.annotations.unwrap())
            } else {false}
        } else {compare_rule.annotations.is_none()};

        if !labels_eq && anns_eq && rule.expr == compare_rule.expr &&
            rule.for_.as_deref() == compare_rule.for_.as_deref() &&
            rule.alert.as_deref() == compare_rule.alert.as_deref() &&
            rule.record.as_deref() == compare_rule.record.as_deref() {
            found = true;
            break;
        }
    }
    return found;
}


fn are_groups_equal(original: &kube_lib::GroupSpec, copy: &kube_lib::GroupSpec) -> bool {
    if original.name == copy.name &&
        original.interval == copy.interval {
        let mut found_diff = false;
        for rule in original.rules.iter().cloned().into_iter() {
            found_diff = !find_rule_named(copy.rules.to_vec(), rule);
            if found_diff { break; }
        };
        for rule in copy.rules.iter().cloned().into_iter() {
            found_diff = !find_rule_named(original.rules.to_vec(), rule);
            if found_diff { break; }
        }
        found_diff
    } else {false}

}

fn get_or_create_action<'ret, 'src:'ret, 'a>(tenant_id: &'a String, groups_tenant_hashmap: &'src mut HashMap<String,Vec<kube_lib::GroupSpec>>) -> Vec<kube_lib::GroupSpec> {
    if groups_tenant_hashmap.contains_key(tenant_id) {
        groups_tenant_hashmap.get_mut(tenant_id).unwrap().to_owned()
    } else {
        Vec::new().to_owned()
    }
}


// Calculate rule updates for each tenant
pub fn diff_rule_groups(
    target_rules: HashMap<String,Vec<kube_lib::GroupSpec>>,
    origin_rules: HashMap<String,
    Vec<kube_lib::GroupSpec>>
) -> (HashMap<String, Vec<kube_lib::GroupSpec>>, HashMap<String, Vec<kube_lib::GroupSpec>>) {
    let mut updates = HashMap::new();
    let mut removals = HashMap::new();

    // Calculate updates: all stuff different in origin
    for tenant_id in origin_rules.keys().into_iter() {
        debug!("origin tenant {}", tenant_id);
        let origin_rule_groups_list = origin_rules.get(tenant_id).unwrap().iter().cloned().into_iter();
        debug!("num rules in origin for tenant {}: {}", tenant_id, origin_rule_groups_list.len());
        let mut updates_vec = get_or_create_action(tenant_id, &mut updates);
        if !target_rules.contains_key(tenant_id) {
            // all rules go to updates
            updates_vec.extend(origin_rule_groups_list.into_iter());
        } else {
            let target_rule_groups_list = target_rules.get(tenant_id).unwrap();
            for origin_group in origin_rule_groups_list.into_iter() {
                match find_group_named(target_rule_groups_list.to_owned(), &origin_group.name) {
                    Some(g) => {
                        if !are_groups_equal(&origin_group, &g) {
                            debug!("group named {} for tenant {} are different in target and origin",
                                origin_group.name, tenant_id);
                            updates_vec.push(origin_group);
                        } else {
                            debug!("group named {} for tenant {} already synced",
                                   origin_group.name, tenant_id);
                        }
                    },
                    None => {
                        debug!("no group with name {} exists in target", origin_group.name);
                        updates_vec.push(origin_group);
                    }
                };
            };
        };
        if updates_vec.len() > 0 {
            updates.insert(tenant_id.clone(), updates_vec.to_owned());
        };
    };

    // Calculate removals: all stuff in target but not in main
    for tenant_id in target_rules.keys().into_iter() {
        debug!("target tenant {}", tenant_id);
        let target_rule_groups_list = target_rules.get(tenant_id).unwrap().iter().cloned().into_iter();
        let mut removals_vec = get_or_create_action(tenant_id, &mut removals);
        if !origin_rules.contains_key(tenant_id) {
            debug!("no groups at all for tenant {} in origin", tenant_id);
            removals_vec.extend(target_rule_groups_list);
        } else {
            let origin_rule_groups_list = origin_rules.get(tenant_id).unwrap();
            for target_group in target_rule_groups_list {
                match find_group_named(origin_rule_groups_list.to_owned(), &target_group.name) {
                    Some(g) => {
                        debug!("group with name {} exists in both target and origin", g.name);
                    },
                    None => {
                        debug!("no group named {} for tenant {}", target_group.name, tenant_id);
                        removals_vec.push(target_group);
                    }
                }
            };
        };
        if removals_vec.len() > 0 {
            removals.insert(tenant_id.clone(), removals_vec.to_owned());
        }
    };

    return (HashMap::from(updates), HashMap::from(removals))
}

pub fn get_tenant_map_from_rules_list(open_metrics_rules: Vec<kube_lib::OpenMetricsRule>) -> HashMap<String, Vec<kube_lib::GroupSpec>> {
    let mut tenant_specs_k8s: HashMap<String, Vec<kube_lib::GroupSpec>> = HashMap::new();
    for tenant_rule in open_metrics_rules.iter().cloned().into_iter() {
        for tenant_id in tenant_rule.spec.tenants {
            if !tenant_specs_k8s.contains_key(&tenant_id) {
                let mut vec = Vec::new();
                tenant_specs_k8s.insert(tenant_id.clone(), vec);
            }
            let mut spec_vec =  tenant_specs_k8s.get_mut(&tenant_id).unwrap();
            for group in tenant_rule.spec.groups.iter().cloned().into_iter() {
                spec_vec.push(group);
            }
        }
    }
    return tenant_specs_k8s
}