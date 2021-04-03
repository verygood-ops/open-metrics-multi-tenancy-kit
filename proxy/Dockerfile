FROM rust:1.51.0-buster AS builder
ARG CARGO_BUILD_ARGS="--release"
USER root
RUN apt-get update
RUN apt-get -y install protobuf-compiler

ADD Cargo.lock Cargo.lock
ADD Cargo.toml Cargo.toml
ADD informer/Cargo.toml informer/Cargo.toml
ADD kube-metrics-multi-tenancy-lib/Cargo.toml kube-metrics-multi-tenancy-lib/Cargo.toml
ADD proxy/Cargo.toml proxy/Cargo.toml
ADD proxy/build.rs proxy/build.rs

ADD config/ config/

ADD informer/src informer/src/
ADD kube-metrics-multi-tenancy-lib/src kube-metrics-multi-tenancy-lib/src
ADD proxy/src/ proxy/src/

RUN cargo build ${CARGO_BUILD_ARGS} --target x86_64-unknown-linux-gnu --bin open-metrics-multi-tenancy-proxy
RUN cargo install --target x86_64-unknown-linux-gnu --path=proxy/

FROM quay.io/verygoodsecurity/debian:buster-slim
RUN apt-get -y update && apt-get -y install libssl1.1
COPY --from=builder /usr/local/cargo/bin/open-metrics-multi-tenancy-proxy /usr/bin/open-metrics-multi-tenancy-proxy
ENTRYPOINT ["/usr/bin/open-metrics-multi-tenancy-proxy"]
