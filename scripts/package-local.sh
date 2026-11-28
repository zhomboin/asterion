#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
binary=$(realpath -e -- "${1:-target/release/asterion-server}")
binary_dir=$(dirname -- "$binary")
collector="$binary_dir/asterion-collector"
[[ -x $binary && -x $collector && -f web/dist/index.html ]] || { echo '请先构建面板程序、采集器和前端。' >&2; exit 1; }
mkdir -p .local
archive=".local/asterion-local-linux-$(uname -m).tar.gz"
artifacts=(asterion-server asterion-collector)
for arch in x86_64 aarch64; do
    relative="collectors/$arch/asterion-collector"
    if [[ -f "$binary_dir/$relative" && -x "$binary_dir/$relative" ]]; then
        artifacts+=("$relative")
    fi
done
tar -czf "$archive" start.sh docs/{local,managed-operations,monitoring-and-operations,development,deployment,collector,windows-desktop,desktop-parity}.md \
    deploy/ssh.example.json deploy/collector/asterion-collector.{service,conf,sysusers} \
    web/dist/index.html web/dist/static -C "$binary_dir" "${artifacts[@]}"
printf '本机运行包 %s/%s\n' "$PWD" "$archive"
