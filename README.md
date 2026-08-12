# 手机型号数据 API

把手机型号数据（JSON）数据化到 **redb**（Rust 最快纯 Rust KV）与 **HNSW 向量索引**（hnsw_rs），
并提供高性能 HTTP API（axum + tokio）。**与任何第三方数据源无关** —— 数据由你自己提供，代码 MIT 协议可自由商用。

```
你的数据（brands/*.json）
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

## 数据格式（JSON，你自己提供）

把数据放进仓库根目录 `brands/`（或任意目录/单文件，`--source` 指定）。
一个 JSON 文件 = 一个设备数组：

```json
[
  {
    "brand": "Apple",
    "series": "iPhone",
    "code": "N90AP",
    "name": "iPhone 4",
    "codename": "iPhone3,1",
    "models": [
      { "ids": ["A1332"], "market_name": "iPhone 4 (GSM)" },
      "A1333"
    ]
  }
]
```

字段规则：

| 字段 | 必填 | 说明 |
|---|---|---|
| `name` | ✅ | 设备名 |
| `brand` | 可选 | 品牌（缺省用文件名） |
| `series` / `code` / `codename` | 可选 | 系列 / 设备码 / 代号 |
| `models` | 可选 | 型号列表，每项支持三种写法：`"ID"`、`["ID1","ID2"]`、`{"ids":[...],"market_name":".."}`（market_name 缺省取 `name`） |
| `file` | 可选 | 来源标记（缺省为文件名） |

建库：`mobilemodels-db build --data-dir data --source brands`
（`--source` 可指向单个 `.json` 文件或含多个 `.json` 的目录，默认当前目录）。

## 数据来源（每日自动搜集，合法）

`collect` 命令从**公开一手来源**抓取事实数据（型号编号 / codename / 品牌 / 市场名均为事实，不受版权保护），
输出为标准 JSON 输入格式，直接喂给 `build`：

```bash
mobilemodels-db collect --source google-play --out brands   # Google Play 官方设备列表（5 万+ 台，含 codename）
mobilemodels-db build --data-dir data --source brands
```

### 每日凌晨自动运行

- **GitHub Actions（免费）**：仓库自带 `.github/workflows/daily.yml`，每天 02:00（北京时间）自动
  抓取 → 建库 → 提交更新。推送到 GitHub 即生效，也可 `workflow_dispatch` 手动触发。
- **本地**：`scripts/daily.sh` 配 cron（`0 18 * * *`）或 Windows 任务计划程序（每天 02:00）。

### 合法来源清单（对应数据类型）

| 数据类别 | 来源 | 授权情况 |
|---|---|---|
| Android 机型 + codename（全品牌） | Google Play 官方设备兼容列表 | Google 公开发布的事实数据，✅ 已实现（5 万+ 台） |
| **Apple（A 编号 ↔ 机型）** | Apple 官方支持页（HT201296） | 官方事实数据，大陆可直连，✅ 已实现（37 台，含 iPhone 17 系） |
| **华为 / 鸿蒙（含型号）** | Wikipedia: List of Huawei products | CC BY-SA（可商用），✅ 已实现（158 台，Ascend/Y/Nova/Enjoy 系） |
| 国行进网型号 ID | 工信部 TENAA / 3C 认证 | 官方备案记录（事实），🕐 计划中 |
| Apple 老机型 A 编号（2016 前） | Apple 官网/FCC 备案 | 官方事实，🕐 计划中 |

⚠️ 原则：只用官方/CC0/CC-BY-SA 一手来源；**不要**抓取 GSMArena 等 ToS 禁止批量抓取的聚合站；
不要复制第三方项目的文件编排（用自有 JSON 格式）。

> 注：Wikipedia 在大陆网络不可达，本地可设代理（如 `HTTPS_PROXY=http://127.0.0.1:7890`）
> 或交给每日 GitHub Actions（US 运行器）自动执行。Apple 官方源大陆可直连，无需代理。
> 所有解析器均有单测覆盖（`cargo test`）。

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
