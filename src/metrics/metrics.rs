#![deny(warnings)]
use std::collections::HashMap;

use crate::proto;
use proto::prometheus::{TimeSeries,WriteRequest};


// processes single time serie
// aggregate data over tenant
// populate hashmap with writerequests
// return number of processed tenants and labels
pub fn process_time_serie(
    time_series: &TimeSeries,
    tenant_labels: &Vec<String>,
    allow_listed_tenants: &Vec<String>,
    does_allow_list: bool,
    replicate_to: &Vec<String>,
    tenant_data: &mut HashMap::<String,WriteRequest>
) -> (u16,u16) {

    let mut label_tenants: Vec<String> = vec![];
    let mut tenants_detected = 0 as u16;
    let mut labels_detected = 0 as u16;

    for label in &time_series.labels {

        // find out if label identifies tenant
        for tenant_label in tenant_labels {
            labels_detected += 1;
            if tenant_label.as_str() == label.name.as_str() {
                // remember tenant id
                label_tenants.push(label.value.clone());
                tenants_detected += 1;
            };
        };
    };

    // remember visited tenants to avoid duplicate requests
    let mut visited_tenants:Vec<String> = vec![];

    let tenants = if does_allow_list {
        replicate_to.clone().into_iter()
            .chain(label_tenants.into_iter()).into_iter()
    } else {
        // chain empty vector for type matching
        allow_listed_tenants.clone().into_iter()
            .chain(vec![].into_iter()).into_iter()
    };
    // cycle tenant id vectors

    for tenant_id in tenants {

        // create single request for each tenant.
        if !visited_tenants.contains(&tenant_id) {

            // setdefault
            if !tenant_data.contains_key(&tenant_id)
            {
                tenant_data.insert(tenant_id.clone(), WriteRequest::new());

            };
            let tenant_replication_request = tenant_data.get_mut(&tenant_id).unwrap();

            // append time series
            tenant_replication_request.timeseries.push(
                TimeSeries::from(time_series.clone()));

            // remember visited tenant
            visited_tenants.push(tenant_id);
        };

    };

    (tenants_detected, labels_detected)
}
