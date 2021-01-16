FROM quay.io/verygoodsecurity/rust-musl-builder:1.49.0 AS builder
ARG CARGO_ARGS="--release"

RUN rustup target add x86_64-unknown-linux-musl
ADD --chown=rust:rust build.rs build.rs
ADD --chown=rust:rust Cargo.lock Cargo.lock
ADD --chown=rust:rust Cargo.toml Cargo.toml
ADD --chown=rust:rust src/ src/
ADD --chown=rust:rust config config/
RUN cargo build
RUN cargo install --target x86_64-unknown-linux-musl --path=.

FROM debian:buster-slim
RUN apt-get -y update && apt-get -y install libssl1.1
COPY --from=builder /home/rust/.cargo/bin/open-metrics-multi-tenancy-proxy /usr/bin/open-metrics-multi-tenancy-proxy
ENTRYPOINT ["/usr/bin/open-metrics-multi-tenancy-proxy"]
