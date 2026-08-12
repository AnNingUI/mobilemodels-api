# mobilemodels-db（Rust 核心）

手机型号数据管道 + API 服务内核：解析 JSON 数据 → **redb**（纯 Rust 最快 ACID KV）+
**HNSW 向量索引**（hnsw_rs，纯 Rust），HTTP 层用 **axum + tokio**。

## 架构

```
你的数据（JSON 文件/目录）
   │  parser（serde_json）
   ▼
Device 记录
   ├─► redb KV（data/mobilemodels.redb）
   │     devices / by_model_id / by_code / by_codename / by_name / by_brand / by_series / vectors
   └─► HNSW 向量索引（1024 维 n-gram 哈希嵌入，DistCosine）
```

- **redb**：ACID、MVCC、B+ 树、内存映射，基准接近 RocksDB。
- **hnsw_rs**：纯 Rust HNSW（无 C 依赖），查询亚毫秒。
- **嵌入**：确定性特征哈希（ASCII token + 字符 n-gram，FNV-1a），离线、秒级构建；
  对型号 ID / 代号 / 名称的词法匹配极佳；无语义泛化（概念词如「折叠屏」需文档中出现）。

## 构建

```bash
cargo +stable build --release   # 需要 stable（nightly 在部分 crate 上有编译器 ICE）
```

## 使用

```bash
B=./target/release/mobilemodels-db

$B collect --source google-play --out ../brands     # Google Play 官方列表（每日可跑）
$B collect --source wikipedia-apple --out ../brands  # Apple（Wikipedia，需外网）
$B collect --source wikipedia-huawei --out ../brands # 华为/鸿蒙（Wikipedia）
$B collect --source wikipedia-honor --out ../brands  # 荣耀（Wikipedia）
$B build --data-dir ../data --source ../examples   # 示例 JSON 数据建库（<1s）
$B query model MP-1000 --data-dir ../data          # 型号精确查询
$B query codename my_phone_1 --data-dir ../data
$B query brand 示例 --data-dir ../data             # 品牌（模糊）
$B query series 示例 "折叠" --data-dir ../data     # 系列（模糊）
$B search "MyFold" -k 5 --data-dir ../data         # 语义检索
$B export ../data/devices.json --data-dir ../data
$B serve --data-dir ../data --port 8080            # HTTP API
```

`--source` 指定 JSON 数据文件/目录（默认当前目录），`--data-dir` 指定数据库输出目录；
`collect` 支持 `--source google-play | wikipedia-apple | wikipedia-huawei | wikipedia-honor`、`--limit N`（调试用）。
Wikipedia 源需外网（大陆本地不可达），由每日 GitHub Actions 执行；解析逻辑有单测覆盖。

## HTTP API

| 端点 | 说明 |
|---|---|
| `GET /health` | 健康检查 + 设备数/维度/索引节点 |
| `GET /stats` | 统计 + 品牌分布 |
| `GET /devices/{id}` | 按 id 取设备 |
| `GET /query/model/{key}` | 型号 ID（精确） |
| `GET /query/code/{key}` | 设备码 |
| `GET /query/codename/{key}` | 代号（精确） |
| `GET /query/name/{key}` | 市场名（精确） |
| `GET /query/brand/{key}` | 品牌（模糊） |
| `GET /query/series/{brand}/{series}` | 系列（品牌+系列模糊） |
| `GET /search?q=..&k=..&brand=..` | HNSW 语义检索（品牌过滤） |
| `GET /export` | 全量设备 JSON |

`serve` 为 12-factor：设置 `PORT` 环境变量时自动绑定 `0.0.0.0:$PORT`。

## 部署（免费平台）

项目根目录自带 `Dockerfile` + `render.yaml`，启动流程 `build --source /app` → `serve`。

1. **Render.com**（免费、免信用卡）：New → Blueprint → 选仓库 → 自动读 `render.yaml`；空闲 15 分钟休眠。
2. **Koyeb**：Create Web Service → Dockerfile。
3. **Oracle Cloud Always Free**（永久在线最佳）：`docker run -d --restart unless-stopped -p 80:10000 -e PORT=10000 <镜像>`。
4. **Google Cloud Run**：`gcloud run deploy --source . --allow-unauthenticated`（按量免费、自动缩零）。

免费平台 /data 为临时盘，每次冷启动重建数据库（~2s）；数据量大时可把 `--data-dir` 指向持久盘。

## 实测

| 操作 | 耗时 |
|---|---|
| 全量 build（解析 → redb + 向量） | 6830 条数据约 2.3s |
| redb 精确查询 | ~50ms（进程冷启动） |
| HNSW 语义检索（服务内，含 HTTP+JSON） | ~3ms |
| 500 并发混合负载 | 2.4k RPS，0 失败 |

## 许可

MIT License。数据由你自行提供并负责其授权（详见根目录 README）。
