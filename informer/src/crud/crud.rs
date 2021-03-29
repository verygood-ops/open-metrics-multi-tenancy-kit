use kube_metrics_mutli_tenancy_lib as kube_lib;
use log::{debug,error,info};

use reqwest::Client;

pub async fn update_ruler_rule(
    client: Client,
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

            match response {
                Ok(r) => {
                    let s = r.status().to_string();
                    match r.text().await {
                        Ok(t) => {
                            info!("received ruler response {}, text {}", s, t);
                        },
                        Err(e) => {
                            error!("failed to receive ruler response body");
                        }
                    }
                },
                Err(e) => {
                    error!("failed to update ruler, abort!");
                }
            }
        },
        Err(e) => {
            error!("failed to encode rule group, abort!");
        }
    };

}
