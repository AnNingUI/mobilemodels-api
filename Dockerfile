# syntax=docker/dockerfile:1

# Render 免费实例内存只有 512MB，编译 Rust（axum/tokio/reqwest 等）会 OOM。
# 方案：GitHub Actions（build-release.yml）预编译二进制发布到 GitHub Release，
# 这里直接从 /releases/latest/download/mobilemodels-db 下载，构建只需 ~1 分钟。

# 仓库（用于下载二进制；Public 仓库无需认证）
ARG REPO=AnNingUI/mobilemodels-api

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

ARG REPO
# 首次部署时 build-release 工作流可能还没跑完，加重试
RUN curl --retry 8 --retry-all-errors --retry-delay 10 -sL \
      "https://github.com/${REPO}/releases/latest/download/mobilemodels-db" \
      -o /usr/local/bin/mobilemodels-db \
    && chmod +x /usr/local/bin/mobilemodels-db \
    && /usr/local/bin/mobilemodels-db stats 2>&1 | head -1 || true

# 数据源：每日工作流提交的 brands/*.json（进网/安卓/苹果/华为）
COPY brands/ brands/
COPY examples/ examples/

# 启动时重建 redb + 向量索引（~2s），随后提供 API
CMD ["sh", "-c", "mobilemodels-db build --data-dir /data --source /app/brands && exec mobilemodels-db serve --data-dir /data"]
