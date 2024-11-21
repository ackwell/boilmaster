# Setup chef
FROM rust:1.82.0-slim-bookworm AS base

RUN apt-get update && apt-get install pkg-config libssl-dev git -y

RUN cargo install cargo-chef --locked

# Setup recipe
FROM base AS planner

WORKDIR /app

COPY . .

RUN cargo chef prepare --bin boilmaster --recipe-path recipe.json

# Build Boilmaster
FROM base AS builder

WORKDIR /app

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --bin boilmaster --release --recipe-path recipe.json

COPY . .

RUN cargo build --release --bin boilmaster

# Create runtime image
FROM debian:bookworm-slim AS runtime

# Redirect persistent data into one shared volume
ENV BM_VERSION_PATCH_DIRECTORY="/app/persist/patches"
ENV BM_SCHEMA_EXDSCHEMA_DIRECTORY="/app/persist/exdschema"
ENV BM_VERSION_DIRECTORY="/app/persist/versions"
ENV BM_SEARCH_SQLITE_DIRECTORY="/app/persist/search"

WORKDIR /app

RUN apt-get update && apt-get install -y git curl

COPY --from=builder /lib/x86_64-linux-gnu/libz.so.1 /lib/x86_64-linux-gnu/libz.so.1
COPY --from=builder /app/boilmaster.toml /app
COPY --from=builder /app/target/release/boilmaster /app

VOLUME /app/persist

HEALTHCHECK --start-period=45s --interval=15s --retries=3 --timeout=5s CMD curl -sf http://localhost:8080/health/live || exit 1

EXPOSE 8080

ENTRYPOINT ["/app/boilmaster"]
