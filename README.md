# 手机型号数据 API

把手机型号数据（markdown / JSON）数据化到 **redb**（Rust 最快纯 Rust KV）与 **HNSW 向量索引**（hnsw_rs），
并提供高性能 HTTP API（axum + tokio）。**与任何第三方数据源无关** —— 数据由你自己提供，代码 MIT 协议可自由商用。

```
你的数据（brands/*.md）
   └─► build ──► data/mobilemodels.redb（KV + 向量）
                   └─► serve ──► HTTP API（/query /search /stats ...）
```

## 快速开始

```bash
# 1. 编译
cd rust && cargo +stable build --release && cd ..

# 2. 用示例数据建库（~1s）
./rust/target/release/mobilemodels-db build --data-dir data --source examples

# 3. 启动 API（默认 127.0.0.1:8080；设置 PORT 环境变量则监听 0.0.0.0:$PORT）
PORT=8080 ./rust/target/release/mobilemodels-db serve --data-dir data

# 4. 查询
curl "localhost:8080/query/model/MP-1000"          # 型号 ID 精确查询
curl "localhost:8080/search?q=MyPhone%20Two&k=5"    # 语义检索
```

## 数据格式（你自己提供）

把数据文件放进仓库根目录 `brands/`，每个文件一个品牌：

```markdown
# 品牌名

- 汇总范围: 任意元信息（会被忽略）

## 系列名（可选）

**[`CODE`] 设备名 (`codename`):**

`MODEL-1000`: 市场名 标准版
`MODEL-1001` `MODEL-1002`: 市场名 多型号
```

语法要点：

| 行 | 含义 |
|---|---|
| `# 标题` | 文件标题（忽略） |
| `- 键: 值` | 元信息（忽略） |
| `## 系列` | 系列名，进入后续设备的 `series` 字段 |
| `**[`CODE`] 名称 (`代号`):** | 设备头：`CODE`/代号均可省略（如 `**Pixel 7 (`panther`):**`、`**中兴天机 7:**`） |
| `` `ID1` `ID2`: 市场名 `` | 型号行：1 个或多个型号 ID + 市场名 |

额外支持（可选）：
- 裸型号列表文件（每行 `` `ID` `` 或 `` `ID`: 名称 ``）—— 如早期机型清单
- `| 名称 | 代号 | 年份 |` 表格文件
- `brands/` 目录旁的 `README.md` 里放 `| [文件名](brands/文件名.md) | 品牌名 |` 表格可指定品牌名（缺省用文件名）

建库命令：`mobilemodels-db build --data-dir data --source <你的数据目录>`（`--source` 默认为当前目录）。

## HTTP API

| 端点 | 说明 |
|---|---|
| `GET /health` | 健康检查 + 设备数 |
| `GET /stats` | 统计 + 品牌分布 |
| `GET /devices/{id}` | 按 id 取设备 |
| `GET /query/model/{key}` | 型号 ID 精确查询 |
| `GET /query/code/{key}` | 设备码查询 |
| `GET /query/codename/{key}` | 代号查询 |
| `GET /query/name/{key}` | 市场名查询 |
| `GET /query/brand/{key}` | 品牌查询（模糊） |
| `GET /query/series/{brand}/{series}` | 系列查询（模糊） |
| `GET /search?q=..&k=..&brand=..` | HNSW 语义检索 + 品牌过滤 |
| `GET /export` | 全量 JSON |

详细用法见 [rust/README.md](rust/README.md)。

## CLI 命令

```bash
mobilemodels-db build  [--data-dir DIR] [--source DIR]   # 解析数据 → redb + 向量
mobilemodels-db query  <model|code|codename|name|brand|series> <KEY> [SERIES]
mobilemodels-db search <TEXT> [-k N] [--brand NAME]
mobilemodels-db export <file.json>
mobilemodels-db serve  [--host ..] [--port ..]
mobilemodels-db stats
```

## 部署（免费平台）

见 [rust/README.md](rust/README.md)「部署」章节：Render / Koyeb / Oracle Cloud Always Free / Google Cloud Run，均支持 Dockerfile 直接部署。

## 许可

MIT License —— 代码完全独立，可商用；你提供的数据由你自行负责授权。
