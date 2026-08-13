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
# 下载预编译二进制；校验 ELF 魔数 + 可执行，失败立即报错（不能用 || true 掩盖）
RUN curl --retry 8 --retry-all-errors --retry-delay 10 -sL \
      "https://github.com/${REPO}/releases/latest/download/mobilemodels-db" \
      -o /usr/local/bin/mobilemodels-db \
    && chmod +x /usr/local/bin/mobilemodels-db \
    && if [ "$(head -c 4 /usr/local/bin/mobilemodels-db | od -An -tx1 | tr -d ' ')" != "7f454c46" ]; then \
         echo "ERROR: downloaded file is not an ELF binary (repo 是否 Public？build-release 是否已生成 Release？)" >&2; \
         exit 1; \
       fi \
    && /usr/local/bin/mobilemodels-db --help >/dev/null&& /usr/local/bin/mobilemodels-db --help >/dev/null /usr/local/bin/mobilemodels-db --version >/dev/null

# 数据源：每日工作流提交的 brands/*.json（进网/安卓/苹果/华为）
COPY brands/ brands/
COPY examples/ examples/

# 启动时重建 redb + 向量索引（~2s），随后提供 API
CMD ["sh", "-c", "mobilemodels-db build --data-dir /data --source /app/brands && exec mobilemodels-db serve --data-dir /data"]
