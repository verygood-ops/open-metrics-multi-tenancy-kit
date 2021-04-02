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
                    let body_text = body.unwrap();

                    let g: Result<HashMap<String, Vec<kube_lib::GroupSpec>>,
                        serde_yaml::Error> = match serde_yaml::from_str(&body_text) {
                        Ok(y) => Ok(y),
                        Err(e) => {
                            error!("failed to deserialize input yaml: {}", e.to_string());
                            Err(e)
                        }
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
                        error!("received invalid YAML from ruler: {}", body_text);
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


#[cfg(test)]
mod tests {
    //use kube::{Client, Service};

    //use futures::pin_mut;
    //use http::{Request, Response};
    //use hyper::Body;
    use env_logger;
    use mockito;
    use kube_metrics_mutli_tenancy_lib as kube_lib;
    use crate::rules::rules::{discover_ruler_rules, diff_rule_groups};
    use serde_yaml;
    use serde_yaml::Value;
    use std::collections::HashMap;
    //use std::collections::HashSet;
    //use std::iter::FromIterator;
    //use k8s_openapi::serde_json::Value;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn test_fixture_1() -> Value {
        let s = "
---
test:
  - name: cortexrulegroup1
    rules:
        - expr: histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))
          record: p99:proxy_processing_duration_ms:5m
        - expr: histogram_quantile(0.95, rate(proxy_processing_duration_ms_bucket[5m]))
          record: p95:proxy_processing_duration_ms:5m
        ";
       serde_yaml::from_str(&s).unwrap()
    }

    fn test_fixture_2() -> Value {
        let s = "---
test:
  - name: cortexrulegroup1
    rules:
        - expr: histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))
          record: p99:proxy_processing_duration_ms:5m
        - expr: histogram_quantile(0.95, rate(proxy_processing_duration_ms_bucket[5m]))
          record: p95:proxy_processing_duration_ms:5m
  - name: cortexrulegroup2
    rules:
        - expr: histogram_quantile(0.5, rate(proxy_processing_duration_ms_bucket[5m]))
          record: p50:proxy_processing_duration_ms:5m
        - expr: sum without(code) (proxy_processing_duration_ms)
          record: proxyallcodes
        ";
        serde_yaml::from_str(&s).unwrap()
    }

    fn client() -> reqwest::Client {
        reqwest::ClientBuilder::new()
            .http1_title_case_headers()
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_discover_ruler_rules() {

        init();

        let y1 = test_fixture_1();
        let y2 = test_fixture_2();
        let r1 = serde_yaml::to_string(&y1).unwrap();
        let r2 = serde_yaml::to_string(&y2).unwrap();

        let c = client();

        let _m1 = mockito::mock("GET", "/api/v1/rules/test")
            .with_status(200)
            .with_header("content-type", "application/yaml")
            .with_body(r1)
            .create();

        let _m2 = mockito::mock("GET", "/api/v1/rules/test")
            .with_status(200)
            .with_header("content-type", "application/yaml")
            .with_body(r2)
            .create();

        let ruler_api_url = mockito::server_url() + &String::from("/");

        let found_rule_groups = discover_ruler_rules(
            &vec![String::from("tnt1"), String::from("tnt2")],
            c.clone(),
            &ruler_api_url,
            &String::from("test")

        ).await.unwrap();

        let specs_expected_tenant_1 = vec![
            (kube_lib::GroupSpec {
                name: String::from("cortexrulegroup1"),
                interval: None,
                rules: vec![
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))"),
                        record: Some(String::from("p99:proxy_processing_duration_ms:5m")),
                        labels: None,
                        for_: None
                    },
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("histogram_quantile(0.95, rate(proxy_processing_duration_ms_bucket[5m]))"),
                        record: Some(String::from("p95:proxy_processing_duration_ms:5m")),
                        labels: None,
                        for_: None
                    }
                ]
            }, -1)
        ];
        let specs_expected_tenant_2 = vec![
            (kube_lib::GroupSpec {
                name: String::from("cortexrulegroup1"),
                interval: None,
                rules: vec![
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))"),
                        record: Some(String::from("p99:proxy_processing_duration_ms:5m")),
                        labels: None,
                        for_: None
                    },
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("histogram_quantile(0.95, rate(proxy_processing_duration_ms_bucket[5m]))"),
                        record: Some(String::from("p95:proxy_processing_duration_ms:5m")),
                        labels: None,
                        for_: None
                    }
                ]}, -1),
            (kube_lib::GroupSpec {
                name: String::from("cortexrulegroup2"),
                interval: None,
                rules: vec![
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("histogram_quantile(0.5, rate(proxy_processing_duration_ms_bucket[5m]))"),
                        record: Some(String::from("p50:proxy_processing_duration_ms:5m")),
                        labels: None,
                        for_: None
                    },
                    kube_lib::RuleSpec {
                        alert: None,
                        annotations: None,
                        expr: String::from("sum without(code) (proxy_processing_duration_ms)"),
                        record: Some(String::from("proxyallcodes")),
                        labels: None,
                        for_: None
                    }
                ]}, -1)
        ];

        let mut expected_map = HashMap::new();
        expected_map.insert(String::from("tnt1"), specs_expected_tenant_1);
        expected_map.insert(String::from("tnt2"), specs_expected_tenant_2);

        assert_eq!(found_rule_groups.keys().len(), 2);
        assert_eq!(found_rule_groups, expected_map)

    }

    fn test_fixture_3() -> HashMap<String,Vec<(kube_lib::GroupSpec, i64)>> {
        let mut m = HashMap::new();

        m.insert(
            String::from("tnt1"),
            vec![
                (kube_lib::GroupSpec {
                    name: String::from("cortexrulegroup1"),
                    interval: None,
                    rules: vec![
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p99:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        },
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.95, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p95:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        }
                    ]}, (0 as i64)),
            ]
        );
        m
    }

    fn test_fixture_4() -> HashMap<String,Vec<(kube_lib::GroupSpec, i64)>> {
        let mut m = HashMap::new();

        m.insert(
            String::from("tnt1"),
            vec![
                (kube_lib::GroupSpec {
                    name: String::from("cortexrulegroup1"),
                    interval: None,
                    rules: vec![
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p99:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        },
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.98, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p95:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        }
                    ]}, (1 as i64)),
            ]
        );
        m
    }

    fn test_fixture_5() -> HashMap<String,Vec<(kube_lib::GroupSpec, i64)>> {
        let mut m = HashMap::new();

        m.insert(
            String::from("tnt1"),
            vec![
                (kube_lib::GroupSpec {
                    name: String::from("cortexrulegroup2"),
                    interval: None,
                    rules: vec![
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.99, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p99:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        },
                        kube_lib::RuleSpec {
                            alert: None,
                            annotations: None,
                            expr: String::from("histogram_quantile(0.98, rate(proxy_processing_duration_ms_bucket[5m]))"),
                            record: Some(String::from("p95:proxy_processing_duration_ms:5m")),
                            labels: None,
                            for_: None
                        }
                    ]}, (1 as i64)),
            ]
        );
        m
    }

    #[tokio::test]
    async fn test_diff_rule_groups_1_no_diff() {
        let groups1 = test_fixture_3();
        let groups2 = test_fixture_3();

        let (adds, removes) = diff_rule_groups(groups1, groups2);

        // No diff found
        assert_eq!(adds.len(), 0);
        assert_eq!(removes.len(), 0);
    }

    #[tokio::test]
    async fn test_diff_rule_groups_2_small_diff() {
        let groups1 = test_fixture_3();
        let groups2 = test_fixture_4();

        let (adds, removes) = diff_rule_groups(groups1, groups2);

        // Only one addition exists
        assert_eq!(adds.len(), 1);
        assert_eq!(removes.len(), 0);

        // A group named "adds" from tenant at position "1" should be added
        let gv = adds.get("tnt1").unwrap();
        let (g, i) = gv.get(0).unwrap();
        assert_eq!(g.name, "cortexrulegroup1");
        assert_eq!(i, &(1 as i64));
    }

    #[tokio::test]
    async fn test_diff_rule_groups_3_remove_group() {
        let groups1 = test_fixture_3();
        let groups2 = HashMap::new();

        let (adds, removes) = diff_rule_groups(groups1, groups2);

        // Only one removal exists
        assert_eq!(adds.len(), 0);
        assert_eq!(removes.len(), 1);

        // A group named "cortexrulegroup1" from tenant at position "0" should be removed
        let gv = removes.get("tnt1").unwrap();
        let (g, i) = gv.get(0).unwrap();
        assert_eq!(g.name, "cortexrulegroup1");
        assert_eq!(i, &(0 as i64));
    }

    #[tokio::test]
    async fn test_diff_rule_groups_4_replace_group() {
        let groups1 = test_fixture_3();
        let groups2 = test_fixture_5();

        let (adds, removes) = diff_rule_groups(groups1, groups2);

        // No diff found
        assert_eq!(adds.len(), 1);
        assert_eq!(removes.len(), 1);

       // One group to be removed, while other to be added.
        let gv_a = adds.get("tnt1").unwrap();
        let (g_a, i_a) = gv_a.get(0).unwrap();

        let gv_r = removes.get("tnt1").unwrap();
        let (g_r, i_r) = gv_r.get(0).unwrap();

        // group to be added
        assert_eq!(g_a.name, "cortexrulegroup2");
        assert_eq!(i_a, &(1 as i64));

        // group to be removed
        assert_eq!(g_r.name, "cortexrulegroup1");
        assert_eq!(i_r, &(0 as i64));
    }
}
