use log::error;

use serde_yaml::Value;

use kube::{Api, Client, CustomResource, api::ListParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;


// Get tenants from k8s.
// Return a tuple containing
// bool -- status whether successfully refreshed tenants, or there was an error
// Vec<String> -- list of tenants ID to be ingested
// Option<String> -- optional continue token value
pub async fn refresh_ingestion_tenants(k8s_client: Client,
                                       continue_token: Option<String>,
                                       namespace:&String) -> (bool, Vec<String>, Option<String>) {
    // It is safe to do unwrap() since this function is called only when k8s client was inited
    let client = k8s_client.clone();
    // Call Kubernetes to check ingestion tenant resources.
    let api : Api<MetricsIngestionTenant> = Api::namespaced(client, namespace);
    let lp = if continue_token.is_some() {
        // It is safe to do unwrap() since is_some() was checked
        ListParams::default().continue_token(&continue_token.unwrap().clone())
    } else {
        ListParams::default()
    };

    let mut tenants: Vec<String> = Vec::new();
    let mut continue_ = None;

    let tenants_acquired = match api.list(&lp).await {
        Ok(tl) => {
            for i_t in tl.items.into_iter() {
                for tenant in i_t.spec.tenants {
                    tenants.push(tenant.clone());
                };
            };
            continue_ = tl.metadata.continue_;
            true
        },
        Err(_) => {
            error!("Failed to get k8s API response!");
            false
        }
    };

    (tenants_acquired, tenants, continue_)
}


// A MetricsIngestionTenant CRD. Lists tenants eligible for metrics ingestion.
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "open-metrics.vgs.io", version = "v1", kind = "MetricsIngestionTenant", namespaced)]
pub struct MetricsIngestionTenantSpec {
    // A tenant identifier, value to be sent with X-Scope-OrgID header
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<String>,
    // A tenant description, optional
    pub description: Option<String>
}


// A specification for alerting or recording rule.
#[derive(Debug, PartialEq, Serialize, Clone, Deserialize, JsonSchema)]
pub struct RuleSpec {
    pub alert: Option<String>,
    pub annotations: Option<HashMap<String,String>>,
    pub labels: Option<HashMap<String,String>>,
    pub record: Option<String>,
    pub expr: String,
}

// A specification for a group of rules.
#[derive(Debug, PartialEq, Serialize, Clone, Deserialize, JsonSchema)]
pub struct GroupSpec {
    pub name: String,
    pub interval: String,
    pub rules: Vec<RuleSpec>,
}

// A MetricsTenantRule CRD, representing either recording or alerting kind of rule
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "open-metrics.vgs.io", version = "v1", kind = "MetricsTenantRule", namespaced)]
pub struct MetricsTenantRuleSpec {
    // A tenant identifier list, value to be sent with X-Scope-OrgID header
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupSpec>
}
