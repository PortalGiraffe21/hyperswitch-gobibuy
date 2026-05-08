FROM rust:bookworm as chef
RUN apt-get update \
    && apt-get install -y libpq-dev libssl-dev pkg-config protobuf-compiler \
    && cargo install cargo-chef --locked
WORKDIR /router

FROM chef as planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef as builder
ARG EXTRA_FEATURES=""
ARG VERSION_FEATURE_SET="v1"

ENV CARGO_INCREMENTAL=0
ENV CARGO_BUILD_JOBS=1
ENV RUSTFLAGS="-C debuginfo=0"
ENV CARGO_NET_RETRY=10
ENV RUSTUP_MAX_RETRIES=10
ENV RUST_BACKTRACE="short"

# Build dependencies first — this layer gets cached
COPY --from=planner /router/recipe.json recipe.json
RUN cargo chef cook \
    --release \
    --no-default-features \
    --features release \
    --features ${VERSION_FEATURE_SET} \
    --recipe-path recipe.json

# Now copy source and build — only your changed code recompiles
COPY . .
RUN cargo build \
    --release \
    --no-default-features \
    --features release \
    --features ${VERSION_FEATURE_SET} \
    ${EXTRA_FEATURES}

FROM debian:bookworm
ARG CONFIG_DIR=/local/config
ARG BIN_DIR=/local/bin

COPY --from=builder /router/config/payment_required_fields_v2.toml ${CONFIG_DIR}/payment_required_fields_v2.toml

ARG RUN_ENV=sandbox
ARG BINARY=router
ARG SCHEDULER_FLOW=consumer

RUN apt-get update \
    && apt-get install -y ca-certificates tzdata libpq-dev curl procps

EXPOSE 8080

ENV TZ=Etc/UTC \
    RUN_ENV=${RUN_ENV} \
    CONFIG_DIR=${CONFIG_DIR} \
    SCHEDULER_FLOW=${SCHEDULER_FLOW} \
    BINARY=${BINARY} \
    RUST_MIN_STACK=6291456

RUN mkdir -p ${BIN_DIR}
COPY --from=builder /router/target/release/${BINARY} ${BIN_DIR}/${BINARY}
RUN useradd --user-group --system --no-create-home --no-log-init app
USER app:app
WORKDIR ${BIN_DIR}
CMD ./${BINARY}
