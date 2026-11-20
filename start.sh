#!/usr/bin/env bash
set -euo pipefail
umask 077
cd -- "$(dirname -- "$0")"
root=$PWD

panel="$root/asterion-server"
if [[ ! -x $panel ]]; then
    [[ -f Cargo.toml ]] || { echo '请完整解压本机运行包。' >&2; exit 1; }
    export PATH="$HOME/.cargo/bin:$PATH"
    command -v cargo >/dev/null && command -v npm >/dev/null || {
        echo '源码启动需要 Rust 和 Node.js 24。使用本机运行包可直接启动。' >&2; exit 1;
    }
    if [[ ! -x web/node_modules/.bin/vite || web/package-lock.json -nt web/node_modules/.package-lock.json ]]; then
        npm --prefix web ci
    fi
    npm --prefix web run build
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/.local/cargo-target}"
    cargo build --locked -p asterion-server
    panel=$(realpath -- "$CARGO_TARGET_DIR/debug/asterion-server")
fi

export ASTERION_LOCAL=1
unset ASTERION_LISTEN_ADDR
if [[ -n ${ASTERION_LOCAL_PORT:-} ]]; then
    port=$ASTERION_LOCAL_PORT
    [[ $port =~ ^[0-9]{1,5}$ ]] && ((10#$port <= 65535)) || { echo '本机端口无效' >&2; exit 1; }
    export ASTERION_LISTEN_ADDR="127.0.0.1:$port"
fi
export ASTERION_WEB_DIR="$root/web/dist" ASTERION_DATABASE_URL='sqlite://panel.db'
export ASTERION_ADMIN_USERNAME=local
unset ASTERION_ADMIN_PASSWORD_FILE
state="${ASTERION_LOCAL_DATA_DIR:-$root/.local/panel}"
mkdir -p -- "$state"
cd -- "$state"
export ASTERION_SSH_CONFIG_FILE="$PWD/profiles.json"
echo '本机面板账号为 local。'
exec "$panel"
