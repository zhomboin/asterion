#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
binary=$(realpath -e -- "${1:-target/release/asterion-server}")
[[ -x $binary && -f web/dist/index.html ]] || { echo '请先构建面板程序和前端。' >&2; exit 1; }
mkdir -p .local
archive=".local/asterion-local-linux-$(uname -m).tar.gz"
tar -czf "$archive" start.sh docs/{local,managed-operations,monitoring-and-operations,development,deployment,public-tls}.md \
    deploy/ssh.example.json web/dist/index.html web/dist/static -C "$(dirname -- "$binary")" asterion-server
printf '本机运行包 %s/%s\n' "$PWD" "$archive"
