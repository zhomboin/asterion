#!/usr/bin/env bash
set -euo pipefail

deploy_dir=$(cd -- "$(dirname -- "$0")" && pwd -P)
if [[ ${1:-} == --help || ${1:-} == -h ]]; then
    echo '用法：bash deploy/install-center.sh [可信的 asterion-<构建编号>.tar.gz]'
    echo '在中心服务器运行。省略归档时，以普通构建账号从当前检出构建。'
    exit 0
fi
if (( $# > 1 )); then
    echo '只接受一个发布归档路径，详见 --help。' >&2
    exit 2
fi
if [[ ! -t 0 || ! -t 1 ]]; then
    echo '请从中心服务器的交互终端运行安装向导。' >&2
    exit 1
fi
if [[ $(. /etc/os-release && echo "$ID:$VERSION_ID") != ubuntu:24.04 || ! -d /run/systemd/system ]]; then
    echo '中心安装需要使用 systemd 的 Ubuntu 24.04。' >&2
    exit 1
fi

if (( $# )); then
    archive=$(realpath -e -- "$1")
else
    cd -- "$deploy_dir/.."
    if (( EUID == 0 )); then
        echo '源码构建请用普通 sudo 用户运行本命令。root 可传入可信归档直接安装。' >&2
        exit 1
    fi
    release_id=$(git rev-parse --short=12 HEAD)
    archive="$HOME/asterion-$release_id.tar.gz"
    if [[ ! -f $archive || ! -f $archive.sha256 ]]; then
        bash "$deploy_dir/build-release.sh"
    fi
fi
if (( EUID == 0 )); then
    exec /usr/bin/python3 -I -B "$deploy_dir/center/install.py" "$archive"
fi
exec sudo -- /usr/bin/python3 -I -B "$deploy_dir/center/install.py" "$archive"
