# syntax=docker/dockerfile:1.7

ARG RUST_IMAGE=cgr.dev/chainguard/rust@sha256:534e51f56558a4adc52cb9e59c5e36b8c2d0a2ad59df6a8d6cfa5a0af04ab101
ARG GIT_RUNTIME_IMAGE=cgr.dev/chainguard/git@sha256:9aa78ef5cb1b5c9eec7d490b747d1c38df2f4948627e0755f6fe141ee8032569

FROM ${RUST_IMAGE} AS cargo-chef
WORKDIR /workspace
RUN cargo install --locked cargo-chef

FROM cargo-chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM cargo-chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --package atlas-cli --bin atlas
COPY . .
RUN cargo build --release --locked --package atlas-cli --bin atlas

FROM ${GIT_RUNTIME_IMAGE} AS runtime
WORKDIR /home/git
COPY --from=builder /workspace/target/release/atlas /usr/local/bin/atlas
ENTRYPOINT ["/usr/local/bin/atlas"]
CMD ["--help"]
