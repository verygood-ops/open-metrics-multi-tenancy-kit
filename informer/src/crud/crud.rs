use kube::{Api,Client,api::{Patch,PatchParams}};
use kube::api::DeleteParams;
use log::{debug,error,info,warn};
use reqwest::Client as RClient;
use reqwest::{Response,Error};
use serde::Deserialize;
use sha1::{Sha1, Digest};

use kube_metrics_mutli_tenancy_lib as kube_lib;
use crate::rules::rules;


// Ruler rule modification actions should return 202 on success.
async fn check_response_202(response: Result<Response,Error>) {
    match response {
        Ok(r) => {
            let s = r.status();
            match r.text().await {
                Ok(t) => {
                    info!("received ruler response {}, text {}", s.to_string(), t);
                    if s == 202 {
                        debug!("successfully updated rule")
                    } else {
                        warn!("ruler response is not 202; text: {}", t)
                    }
                },
                Err(e) => {
                    error!("failed to receive ruler response body: {}", e);
                }
            }
        },
        Err(e) => {
            error!("failed to update ruler, abort: {}", e);
        }
    }
}

#[derive(Deserialize)]
struct UserStat {
    #[serde(rename(deserialize = "userID"))]
    user_id: String,
}

// Extracting tenant list from UserStats. "zero" tenant excluded.
async fn extract_tenants(response: Result<Response, Error>) -> Result<Vec<String>, Error> {
    match response {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Vec<UserStat>>().await {
                Ok(user_stats) => {
                    debug!("successfully parsed user stats");
                    let tenant_vec: Vec<String> = user_stats.iter()
                        .map(|user_stat| user_stat.user_id.clone())
                        .filter(|id| id != "0")
                        .collect();
                    info!("received distributor response: {}, parsed tenants: {:?}", status.to_string(), tenant_vec);
                    Ok(tenant_vec)
                }
                Err(e) => {
                    error!("failed to parse distributor response body: {}", e);
                    Err(e)
                }
            }
        }
        Err(e) => {
            error!("failed to read data from distributor, abort: {}", e);
            Err(e)
        }
    }
}

// get tenants from a Distributor
pub async fn load_tenants_from_distributor(
    client: RClient,
    distributor_api_url: &String) -> Result<Vec<String>, Error> {
    let url = distributor_api_url.clone() + "distributor/all_user_stats";
    debug!("distributor URL is {}, going to read data", url);


    let response = client.get(&url)
        .send()
        .await;

    extract_tenants(response).await
}

// update ruler inside rule
pub async fn update_ruler_rule(
    client: RClient,
    ruler_api_url: &String,
    tenant_id: &String,
    namespace: &String,
    rule_group: kube_lib::GroupSpec) {
    let url = ruler_api_url.clone() + "api/v1/rules/" + &namespace.clone();
    debug!("ruler URL is {}, going to insert data", url);
    match serde_yaml::to_string(&rule_group) {
        Ok(body) => {
            debug!("ruler req body is {}", body);
            let response = client.post(&url)
                .header("X-Scope-OrgID", tenant_id)
                .body(body)
                .send()
                .await;
            check_response_202(response).await;

        },
        Err(e) => {
            error!("failed to encode rule group, abort: {}", e);
        }
    };
}

// remove rule from a Ruler
pub async fn remove_ruler_rule(
    client: RClient,
    ruler_api_url: &String,
    tenant_id: &String,
    namespace: &String,
    rule_group: kube_lib::GroupSpec) {
    let url = ruler_api_url.clone() + "api/v1/rules/"
        + &namespace.clone() + "/" + &rule_group.name;
    debug!("ruler URL is {}, going to delete group", url);
    let response = client.delete(&url)
        .header("X-Scope-OrgID", tenant_id)
        .send()
        .await;
    check_response_202(response).await;
}

// create or update resource in k8s
pub async fn create_or_update_k8s_resource(
    k8s_client: Client,
    tenant_id: &String,
    namespace: &String,
    rule_group: kube_lib::GroupSpec
) {
    let cli = k8s_client.clone();
    match kube_lib::discover_open_metrics_rules(cli.clone(), namespace).await {
        Ok(rule_vec) => {
            let mut found: Option<kube_lib::OpenMetricsRule> = None;
            let rule_group_name = rule_group.name.clone();
            for mut rule in rule_vec.iter().cloned().into_iter() {
                if rule.spec.tenants.contains(tenant_id) {
                    let mut groups_with_idx = Vec::new();
                    for g in rule.spec.groups.iter().cloned().into_iter() {
                        groups_with_idx.push((g, -1));
                    };
                    let (idx, group_named, _k8s_idx) =
                        rules::find_group_named(&groups_with_idx, &rule_group.name);
                    if group_named.is_some() {
                        let index = idx as usize;
                        rule.spec.groups[index] = rule_group.clone();
                        found = Some(rule);
                        break;
                    };
                };
            };
            let not_found = found.clone().is_none();
            let api : Api<kube_lib::OpenMetricsRule> = Api::namespaced(
                cli.clone(), namespace);

            let resource_name = if not_found {
                let hash = Sha1::digest(&rule_group_name.replace("_", "-"));
                format!("{}-{:x}", tenant_id, hash)
            } else {
                // It is safe to unwrap, since we just found this rule via discover_open_metrics()
                let rule = found.clone().unwrap();
                rule.metadata.name.unwrap()
            };

            let open_metrics_rule = if not_found {
                // Group not exist before.
                 kube_lib::OpenMetricsRule::new(
                     &resource_name,
                     kube_lib::OpenMetricsRuleSpec {
                         tenants: vec![tenant_id.clone()],
                         description: Some(String::from("open-metrics-multi-tenancy-kit-sourced-rule")),
                         groups: vec![rule_group.clone()]
                     }
                 )
            } else {
                found.clone().unwrap()
            };
            // Mark recent status update.
            // Since group was sourced from ruler, it is safe to assume
            // it is identical.
            resource_updated(&api, &resource_name, open_metrics_rule).await;
        },
        Err(msg) => {
            error!("failed to discover current rule set: {}", msg);
        }
    };
}

// remove or update resource in k8s
pub async fn remove_k8s_resource(
    k8s_client: Client,
    tenant_id: &String,
    namespace: &String,
    rule_group: kube_lib::GroupSpec
) {
    let cli = k8s_client.clone();
    match kube_lib::discover_open_metrics_rules(cli.clone(), namespace).await {
        Ok(rule_vec) => {
            let rule_group_name = rule_group.name.clone();
            for mut rule in rule_vec.iter().cloned().into_iter() {
                if rule.spec.tenants.contains(tenant_id) {
                    let mut groups_with_idx = Vec::new();
                    for g in rule.spec.groups.iter().cloned().into_iter() {
                        groups_with_idx.push((g, -1));
                    };
                    let (idx, group_named, _k8s_idx) =
                        rules::find_group_named(&groups_with_idx, &rule_group.name);
                    if group_named.is_some() {
                        let resource_name = rule.metadata.name.clone().unwrap();
                        let api : Api<kube_lib::OpenMetricsRule> = Api::namespaced(
                            cli.clone(), namespace);

                        if groups_with_idx.len() == 1 {
                            remove_resource(&api, &resource_name).await;
                        } else {
                            rule.spec.groups.remove(idx as usize);
                            resource_updated(&api, &resource_name, rule.clone()).await;
                        }
                        return;
                    };
                };
            };

            warn!("Rule group name [{}] not found", rule_group_name);
        },
        Err(msg) => {
            error!("failed to discover current rule set: {}", msg);
        }
    };
}

// renew resource status
pub async fn resource_updated(api: &Api<kube_lib::OpenMetricsRule>, resource_name: &String, mut open_metrics_rule: kube_lib::OpenMetricsRule) {
    let status = kube_lib::OpenMetricsRuleStatus {
        ruler_updated: true
    };
    // managed fields might be set when querying
    open_metrics_rule.metadata.managed_fields = None;
    open_metrics_rule.status = Some(status);
    let ssapply = PatchParams::apply("openmetricsrule").force();
    match api.patch(
        &resource_name.clone(),
        &ssapply,
        &Patch::Apply(&open_metrics_rule)
    ).await {
        Ok(resp) => {
            // safe to unwrap since yaml received from k8s
            info!("patched k8s resource: {}", serde_yaml::to_string(&resp).unwrap());
        },
        Err(e) => {
            error!("failed to apply rule: {} because of {:?}", &resource_name, e);
        }
    };
}

// remove resource
pub async fn remove_resource(api: &Api<kube_lib::OpenMetricsRule>, resource_name: &String) {


    match api.delete(
        &resource_name.clone(),
        &DeleteParams::default()
    ).await {
        Ok(result) => {
            result
                .map_left(|o| println!("Deleting rule CRD: {:?}", o.status))
                .map_right(|s| println!("Deleted rule CRD: {:?}", s));
        },
        Err(e) => {
            error!("failed to delete rule: {} because of {:?}", &resource_name, e);
        }
    };
}

#[cfg(test)]
mod tests {
    use log::debug;
    use serde_json::Value;

    use crate::crud::crud::load_tenants_from_distributor;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn client() -> reqwest::Client {
        reqwest::ClientBuilder::new()
            .http1_title_case_headers()
            .build()
            .unwrap()
    }

    fn test_fixture() -> Value {
        let s = r#"[
{"userID":"0","ingestionRate":19897.548317860346,"numSeries":4033570,"APIIngestionRate":16398.35078884816,"RuleIngestionRate":3499.1975290121827},
{"userID":"tntimulpey0","ingestionRate":258.3845283139957,"numSeries":4479,"APIIngestionRate":258.3845283139957,"RuleIngestionRate":0},
{"userID":"tnt5ihx8vw2","ingestionRate":94.72447272610466,"numSeries":2115,"APIIngestionRate":87.21010958247322,"RuleIngestionRate":7.514363143631437},
{"userID":"tntgwgis7t4","ingestionRate":49.4930170232766,"numSeries":730,"APIIngestionRate":49.4930170232766,"RuleIngestionRate":0}
]"#;

        serde_json::from_str(&s).unwrap()
    }

    fn test_fixture_zero_tenant() -> Value {
        let s = r#"[
{"userID":"0","ingestionRate":19897.548317860346,"numSeries":4033570,"APIIngestionRate":16398.35078884816,"RuleIngestionRate":3499.1975290121827}
]"#;

        serde_json::from_str(&s).unwrap()
    }

    fn test_fixture_empty() -> Value {
        let s = r#"[
{"userID":"0","ingestionRate":19897.548317860346,"numSeries":4033570,"APIIngestionRate":16398.35078884816,"RuleIngestionRate":3499.1975290121827}
]"#;

        serde_json::from_str(&s).unwrap()
    }

    #[tokio::test]
    async fn test_load_tenants() {
        init();

        let y1 = test_fixture();
        let r1 = serde_json::to_string(&y1).unwrap();

        let _m1 = mockito::mock("GET", "/distributor/all_user_stats")
            .with_status(200)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_body(r1.clone())
            .create();

        let c = client();

        let distributor_api_url = mockito::server_url() + &String::from("/");

        let tenant_vec = load_tenants_from_distributor(c.clone(), &distributor_api_url).await.unwrap();

        debug!("{:?}", tenant_vec);

        assert!(tenant_vec.contains(&"tntimulpey0".to_owned()));
        assert!(tenant_vec.contains(&"tnt5ihx8vw2".to_owned()));
        assert!(tenant_vec.contains(&"tntgwgis7t4".to_owned()));
        assert_eq!(tenant_vec.len(), 3);
    }

    #[tokio::test]
    async fn test_load_tenants_zero_tenant() {
        init();

        let y1 = test_fixture_zero_tenant();
        let r1 = serde_json::to_string(&y1).unwrap();

        let _m1 = mockito::mock("GET", "/distributor/all_user_stats")
            .with_status(200)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_body(r1.clone())
            .create();

        let c = client();

        let distributor_api_url = mockito::server_url() + &String::from("/");

        let tenant_vec = load_tenants_from_distributor(c.clone(), &distributor_api_url).await.unwrap();

        debug!("{:?}", tenant_vec);

        assert_eq!(tenant_vec.len(), 0);
    }

    #[tokio::test]
    async fn test_load_tenants_empty() {
        init();

        let y1 = test_fixture_empty();
        let r1 = serde_json::to_string(&y1).unwrap();

        let _m1 = mockito::mock("GET", "/distributor/all_user_stats")
            .with_status(200)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_body(r1.clone())
            .create();

        let c = client();

        let distributor_api_url = mockito::server_url() + &String::from("/");

        let tenant_vec = load_tenants_from_distributor(c.clone(), &distributor_api_url).await.unwrap();

        debug!("{:?}", tenant_vec);

        assert_eq!(tenant_vec.len(), 0);
    }
}