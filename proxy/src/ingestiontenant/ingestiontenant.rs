use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};


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
