#![deny(warnings)]
use std::collections::{HashMap,HashSet};

use kube::{Api, Client, CustomResource, api::ListParams};
use log::{debug, error, info};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};


// Get rules and tenants from k8s.
// Return a tuple containing
// bool -- status whether successfully refreshed tenants, or there was an error
// Vec<String> -- list of tenants ID to be ingested
// Option<String> -- optional continue token value
pub async fn refresh_open_metrics_rules(k8s_client: Client,
                                        continue_token: Option<String>,
                                        namespace:&String) -> (bool, Vec<OpenMetricsRule>, Option<String>) {
    // It is safe to do unwrap() since this function is called only when k8s client was inited
    let client = k8s_client.clone();
    // Call Kubernetes to check ingestion tenant resources.
    let api : Api<OpenMetricsRule> = Api::namespaced(client, namespace);
    let lp = if continue_token.is_some() {
        // It is safe to do unwrap() since is_some() was checked
        ListParams::default().continue_token(&continue_token.unwrap().clone())
    } else {
        ListParams::default()
    };

    let mut rules: Vec<OpenMetricsRule> = Vec::new();
    let mut continue_ = None;

    let rules_acquired = match api.list(&lp).await {
        Ok(tl) => {
            for rule in tl.items.iter().cloned().into_iter() {
                rules.push(rule.clone());
            };
            continue_ = tl.metadata.continue_;
            true
        },
        Err(_i) => {
            error!("Failed to get k8s API response.");
            false
        }
    };

    return (rules_acquired, rules, continue_);
}


// Retrieve ingestion tenants to be used for metrics discovery
pub async fn discover_open_metrics_rules(k8s_client: Client, namespace: &String)
    -> Result<Vec<OpenMetricsRule>,String> {

    let mut found_rules = Vec::new();
    let client = k8s_client.clone();
    // Call Kubernetes to check ingestion tenant resources.

    let (tenants_acquired, rules_portion, next_token) = refresh_open_metrics_rules(
            client.clone(),
            None,
            &namespace.clone()).await;

    for rule in rules_portion {
        found_rules.push(rule);
    }
    match next_token {
        Some(token_init) => {
            // Retrieve rest of data
            if token_init.len() > 0 {
                info!("Token: {}", token_init);
                let mut t = token_init.clone();
                loop {
                    let token = t.clone();
                    let (_tenants_acquired_inner,
                        rules_portion, _t) = refresh_open_metrics_rules(
                        client.clone(), Some(token), &namespace.clone()).await;
                    for rule in rules_portion {
                        found_rules.push(rule);
                    };
                    if !_t.is_some() {
                        debug!("Finished acquiring tenants from k8s");
                        break
                    } else {
                        t = _t.unwrap().clone();
                    };
                };
            };
        },
        None => {
            debug!("no more data for k8s rules");
        }
    }

    if tenants_acquired {
        Ok(found_rules)
    } else {
        Err(String::from("k8s communication failure"))
    }

}

// Extract tenants set from rules
pub fn discover_tenant_ids(tenants_rules: Vec<OpenMetricsRule>) -> HashSet<String> {
    let mut tenant_ids = HashSet::new();
    for tenant_rule in tenants_rules.into_iter() {
        for tenant_id in tenant_rule.spec.tenants {
            tenant_ids.insert(tenant_id);
        }
    };
    return tenant_ids;
}

//  Discover all tenant IDs necessary for ingestion
pub async fn get_tenant_ids(k8s_client: Client, namespace: &String) -> Result<HashSet<String>,String> {
    match discover_open_metrics_rules(k8s_client.clone(), namespace).await {
        Ok(tenants_rules) => {
            Ok(discover_tenant_ids(tenants_rules))
        },
        Err(msg) => Err(msg)
    }
}


// A MetricsIngestionTenant CRD. Lists tenants eligible for metrics ingestion.
#[derive(CustomResource, Deserialize, Serialize, Clone, PartialEq, Eq, Debug, JsonSchema)]
#[kube(status = "OpenMetricsRuleStatus")]
#[kube(group = "open-metrics.vgs.io", version = "v1", kind = "OpenMetricsRule", namespaced)]
pub struct OpenMetricsRuleSpec {
    // A tenant identifier, value to be sent with X-Scope-OrgID header
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<String>,
    // A rule description, optional
    pub description: Option<String>,
    // Rule groups list

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupSpec>
}

// A specification for a rule status
#[derive(Serialize, Clone,PartialEq, Eq, Debug,  Deserialize, JsonSchema)]
pub struct OpenMetricsRuleStatus {
    pub ruler_updated: bool,
}


// A specification for alerting or recording rule.
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Deserialize, JsonSchema)]
pub struct RuleSpec {
    #[serde(skip_serializing_if="Option::is_none")]
    pub alert: Option<String>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub annotations: Option<HashMap<String,String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub labels: Option<HashMap<String,String>>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub record: Option<String>,
    #[serde(rename="for", skip_serializing_if="Option::is_none")]
    pub for_: Option<String>,
    pub expr: String,
}

// A specification for a group of rules.
#[derive(Serialize, Clone,PartialEq, Eq, Debug,  Deserialize, JsonSchema)]
pub struct GroupSpec {
    pub name: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub interval: Option<String>,
    pub rules: Vec<RuleSpec>,
}

#[cfg(test)]
mod tests {

    use kube::{Client, Service};

    use futures::pin_mut;
    use http::{Request, Response};
    use hyper::Body;
    use env_logger;
    use tower_test::mock;
    use std::collections::HashSet;
    use std::iter::FromIterator;
    use crate::{discover_open_metrics_rules, discover_tenant_ids, get_tenant_ids};
    use k8s_openapi::serde_json::Value;

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
                                "namespace": "test",
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
                                "namespace": "test",
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

    #[tokio::test]
    async fn test_discover_open_metrics_rules() {
        init();
        let (mock_service, handle) = mock::pair::<Request<Body>, Response<Body>>();
        let spawned = tokio::spawn(async move {
            // Receive a request for pod and respond with some data
            pin_mut!(handle);
            let (request, send) = handle.next_request().await.expect("service not called");
            assert_eq!(request.method(), http::Method::GET);
            assert_eq!(request.uri().to_string(), "/apis/open-metrics.vgs.io/v1/namespaces/test/openmetricsrules?");
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

        // Verify discovery works for tenants without rules
        let discovered_rules = discover_open_metrics_rules(
            Client::new(service), &String::from("test")).await.unwrap();

        assert_eq!(discovered_rules.len(), 2);

        // Verify tenant discovery works
        let discovered_tenant_ids = discover_tenant_ids(discovered_rules);

        let expected_tenants = vec![
            String::from("tenant1"),
            String::from("tenant2"),
            String::from("tenant7"),
            String::from("tenant8"),
        ];
        let expected_tenant_ids: HashSet<String> = HashSet::from_iter(expected_tenants.iter().cloned().into_iter());

        assert_eq!(discovered_tenant_ids, expected_tenant_ids);

        spawned.await.unwrap();
    }

    #[tokio::test]
    async fn test_get_tenant_ids() {
        init();
        let (mock_service, handle) = mock::pair::<Request<Body>, Response<Body>>();
        let spawned = tokio::spawn(async move {
            // Receive a request for pod and respond with some data
            pin_mut!(handle);
            let (request, send) = handle.next_request().await.expect("service not called");
            assert_eq!(request.method(), http::Method::GET);
            assert_eq!(request.uri().to_string(), "/apis/open-metrics.vgs.io/v1/namespaces/test/openmetricsrules?");
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

        // Verify tenant ID discovery works
        let discovered_tenant_ids = get_tenant_ids(Client::new(service), &String::from("test")).await.unwrap();

        let expected_tenants = vec![
            String::from("tenant1"),
            String::from("tenant2"),
            String::from("tenant7"),
            String::from("tenant8"),
        ];
        let expected_tenant_ids: HashSet<String> = HashSet::from_iter(expected_tenants.iter().cloned().into_iter());

        assert_eq!(discovered_tenant_ids, expected_tenant_ids);

        spawned.await.unwrap();
    }

}
