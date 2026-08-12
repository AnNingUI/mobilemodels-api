# syntax=docker/dockerfile:1

# ---------- build stage: compile the Rust binary ----------
FROM rust:1-slim AS builder
WORKDIR /build
COPY rust/Cargo.toml rust/Cargo.lock ./
COPY rust/src ./src
# Cache dependencies first (layer stays warm across rebuilds)
RUN cargo build --release

# ---------- runtime stage: tiny debian image ----------
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/mobilemodels-db /usr/local/bin/mobilemodels-db

# 数据源：部署时把你的数据放进仓库根目录 brands/（或挂载卷到 /app/brands）
# 数据格式见 README.md「数据格式」；examples/ 仅用于本地测试演示，不会被解析
COPY brands/ brands/
COPY misc/ misc/
COPY examples/ examples/

# 启动时重建 redb + 向量索引（~2s），随后提供 API
CMD ["sh", "-c", "mobilemodels-db build --data-dir /data --source /app && exec mobilemodels-db serve --data-dir /data"]
