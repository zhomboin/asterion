#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
binary=$(realpath -e -- "${1:-target/release/asterion-server}")
collector="$(dirname -- "$binary")/asterion-collector"
[[ -x $binary && -x $collector && -f web/dist/index.html ]] || { echo '请先构建面板程序、采集器和前端。' >&2; exit 1; }
mkdir -p .local
archive=".local/asterion-local-linux-$(uname -m).tar.gz"
tar -czf "$archive" start.sh docs/{local,managed-operations,monitoring-and-operations,development,deployment,collector}.md \
    deploy/ssh.example.json deploy/collector web/dist/index.html web/dist/static -C "$(dirname -- "$binary")" asterion-server asterion-collector
printf '本机运行包 %s/%s\n' "$PWD" "$archive"
