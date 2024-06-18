# Setup chef
FROM rust:1.76-slim-buster AS base

RUN apt-get update && apt-get install pkg-config libssl-dev git -y

RUN cargo install cargo-chef --locked

# Setup recipe
FROM base AS planner

WORKDIR /app

COPY . .

RUN cargo chef prepare --bin boilmaster --recipe-path recipe.json

# Build Boilmaster
FROM planner AS builder

WORKDIR /app

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --bin boilmaster --release --recipe-path recipe.json

COPY . .

RUN cargo build --release --bin boilmaster

# Create runtime image
FROM debian:buster-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install git -y

COPY --from=builder /lib/x86_64-linux-gnu/libz.so.1 /lib/x86_64-linux-gnu/libz.so.1
COPY --from=builder /app/boilmaster.toml /app
COPY --from=builder /app/target/release/boilmaster /app

VOLUME /app/patches /app/exdschema /app/versions

HEALTHCHECK --start-period=45s --interval=15s --retries=3 --timeout=5s CMD curl -sf http://localhost:8080/health/live || exit 1

EXPOSE 8080

ENTRYPOINT ["/app/boilmaster"]