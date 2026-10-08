FROM rust:1.90-slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src

RUN cargo build --release

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    iputils-ping \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bimap /usr/local/bin/bimap
ENTRYPOINT ["bimap"]
CMD ["--help"]
