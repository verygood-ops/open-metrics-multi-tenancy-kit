FROM rust:1.51.0-buster AS builder
ARG CARGO_BUILD_ARGS="--release"
USER root
RUN apt-get update
RUN apt-get -y install protobuf-compiler

ADD Cargo.lock Cargo.lock
ADD Cargo.toml Cargo.toml
ADD informer/Cargo.toml informer/Cargo.toml
ADD kube-metrics-multi-tenancy-lib/Cargo.toml kube-metrics-multi-tenancy-lib/Cargo.toml
ADD proxy/build.rs proxy/build.rs
ADD proxy/Cargo.toml proxy/Cargo.toml

ADD config/ config/

ADD informer/src informer/src/
ADD kube-metrics-multi-tenancy-lib/src kube-metrics-multi-tenancy-lib/src
ADD proxy/src/ proxy/src/

RUN cargo build ${CARGO_BUILD_ARGS}
