#!/usr/bin/env bash
set -euo pipefail

deploy_dir=$(cd -- "$(dirname -- "$0")" && pwd -P)
if [[ ${1:-} == --help || ${1:-} == -h ]]; then
    echo '用法：bash deploy/install-center.sh [可信的 asterion-<构建编号>.tar.gz]'
    echo '以 root 或有 sudo 权限的普通用户运行。省略归档时先查找当前目录和家目录，找不到就从源码构建。'
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
os_release=$(. /etc/os-release && echo "$ID:$VERSION_ID")
if [[ $os_release != ubuntu:22.04 && $os_release != ubuntu:24.04 ]] || [[ ! -d /run/systemd/system ]]; then
    echo '中心安装需要使用 systemd 的 Ubuntu 22.04 或 24.04。' >&2
    exit 1
fi

installer=(/usr/bin/python3 -I -B "$deploy_dir/center/install.py")
if (( EUID != 0 )); then
    installer=(sudo -- "${installer[@]}")
fi
if (( $# )); then
    archive=$(realpath -e -- "$1")
else
    shopt -s nullglob
    archives=()
    declare -A seen=()
    for file in "$PWD"/asterion-*.tar.gz "$HOME"/asterion-*.tar.gz; do
        path=$(realpath -e -- "$file")
        if [[ -z ${seen[$path]:-} ]]; then
            archives+=("$path")
            seen[$path]=1
        fi
    done
    case ${#archives[@]} in
        0)
            cd -- "$deploy_dir/.."
            echo '未找到发布包，正在从源码构建。'
            bash "$deploy_dir/build-release.sh"
            release_id=$(git rev-parse --short=12 HEAD)
            archive="$HOME/asterion-$release_id.tar.gz"
            ;;
        1) archive=${archives[0]} ;;
        *)
            echo '找到多个发布包，请选择要继续安装的可信归档。'
            PS3='编号：'
            select archive in "${archives[@]}"; do
                [[ -n $archive ]] && break
            done
            [[ -n ${archive:-} ]] || exit 1
            ;;
    esac
fi
printf '使用发布包 %s\n' "$archive"
exec "${installer[@]}" "$archive"
