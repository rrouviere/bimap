FROM rust:1.90-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates psmisc \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bimap /usr/local/bin/bimap
COPY --from=builder /app/test_all.sh /test_all.sh
RUN chmod +x /test_all.sh

CMD ["/test_all.sh"]
