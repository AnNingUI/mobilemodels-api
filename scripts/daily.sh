#!/usr/bin/env bash
# 本地每日自动搜集（配合 cron / Windows 任务计划程序）
#   cron:        0 18 * * *  cd /path/to/repo && ./scripts/daily.sh
#   Windows:     任务计划程序 → 新建任务 → 每天 02:00 → 程序: bash 参数: scripts/daily.sh
set -euo pipefail
cd "$(dirname "$0")/.."

B=./rust/target/release/mobilemodels-db

echo "[$(date '+%F %T')] collecting ..."
$B collect --source google-play --out brands

echo "[$(date '+%F %T')] rebuilding db ..."
$B build --data-dir data --source brands

echo "[$(date '+%F %T')] exporting json ..."
$B export data/devices.json --data-dir data

# 有变更才提交（无变更 git commit 会失败，忽略即可）
if git diff --quiet brands/; then
  echo "no data change"
else
  git add brands/ && git commit -m "chore(daily): refresh data" || true
fi
echo "[$(date '+%F %T')] done"
