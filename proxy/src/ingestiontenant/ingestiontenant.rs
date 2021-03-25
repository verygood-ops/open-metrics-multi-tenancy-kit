#[macro_use]
use crate::controller;
extern crate serde_derive;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use apiexts::CustomResourceDefinition;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1 as apiexts;

use kube::{
    api::{Api, ListParams, Meta, Patch, PatchParams, WatchEvent},
    Client, CustomResource
};


#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "open-metrics.vgs.io", version = "v1", kind = "IngestionTenant", namespaced)]
pub struct IngestionTenantSpec {
    // A tenant identifier, value to be sent with X-Scope-OrgID header
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<String>,
    // A tenant description, optional
    pub description: Option<String>
}
