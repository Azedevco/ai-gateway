FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY migrations migrations
COPY templates templates
COPY static static
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /app gateway
WORKDIR /app
COPY --from=build /src/target/release/ai-gateway /usr/local/bin/ai-gateway
RUN mkdir -p /app/data && chown gateway /app/data
USER gateway
ENV GATEWAY_BIND=0.0.0.0:8090 DATABASE_URL=sqlite:///app/data/gateway.db
EXPOSE 8090
VOLUME ["/app/data"]
CMD ["ai-gateway"]
