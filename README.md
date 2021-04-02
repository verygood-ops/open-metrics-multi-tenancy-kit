open-metrics-multi-tenancy-kit
=================================

OM-mt-K allows to implement multi-tenancy for metrics collection,
rule and alert management, and exposition in Cortex environments.

Metrics collection multi-tenancy
--------------------------------
A proxy component introspects Prometheus GRPC metrics `remote_write` stream,
injects tenants in accordance to https://cortexmetrics.io/docs/guides/auth/,
and forwards requests to specified upstream. Tenants can be either enabled
via command line, or specified via `IngestionTenant` k8s resources, managed by Controller.

Rule And Alert Management
-------------------------
Rules and Alerts are set up via k8s resources.
A controller component loads rules into Cortex.
A webhook component validates rules and alerts submitted.
`OpenMetricsRule` -- specifies open metrics recording rule
`OpenMetricsAlert` -- specifies open metrics alert

Open metrics exposition
-----------------------
TBD.

How to build
-------------

Local:

1) [Install Rust](https://doc.rust-lang.org/cargo/getting-started/installation.html)
2) `cargo build`

Docker:

`docker-compose build`

How to test
-----------

Local:

```
RUST_LOG=debug RUST_BACKTRACE=full cargo test -- --nocapture
```

Docker:

`docker-compose test`

How to run
----------
Local:

Run proxy
```
RUST_LOG=debug RUST_BACKTRACE=full cargo run \
    --bin open-metrics-multi-tenancy-proxy
```

Run informer

```
RUST_LOG=debug RUST_BACKTRACE=full cargo run \
    --bin open-metrics-multi-tenancy-informer
```

Docker:

`docker-compose up`
