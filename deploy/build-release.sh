#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
as_root=()
if (( EUID != 0 )); then
    as_root=(sudo)
fi
mkdir -p .local
exec 9>.local/build-release.lock
flock 9
"${as_root[@]}" apt-get update
"${as_root[@]}" apt-get install -y build-essential ca-certificates curl git openssl pkg-config xz-utils
release_id=$(git rev-parse --short=12 HEAD)
archive="asterion-$release_id.tar.gz"
work=$(mktemp -d "$HOME/.asterion-build.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v rustup >/dev/null; then
    curl --proto '=https' --tlsv1.2 -fsSLo "$work/rustup-init.sh" https://sh.rustup.rs
    sh "$work/rustup-init.sh" -y --profile minimal --default-toolchain stable --no-modify-path
fi
rustup toolchain install stable --profile minimal
if ! node -e 'process.exit(process.platform==="linux" && process.versions.node.split(".")[0]==="24" ? 0 : 1)' 2>/dev/null; then
    case "$(uname -m)" in
        x86_64) arch=x64 ;;
        aarch64) arch=arm64 ;;
        *) echo '需要适用于此架构的 Linux Node.js 24。' >&2; exit 1 ;;
    esac
    (
        cd "$work"
        curl -fsSLo hashes https://nodejs.org/dist/latest-v24.x/SHASUMS256.txt
        read -r hash pkg < <(awk -v a="$arch" '$2 ~ ("^node-v24\\.[0-9]+\\.[0-9]+-linux-" a "\\.tar\\.xz$") { print; exit }' hashes)
        version=${pkg#node-}
        curl -fsSLo "$pkg" "https://nodejs.org/dist/${version%%-linux-*}/$pkg"
        printf '%s  %s\n' "$hash" "$pkg" | sha256sum --check -
        mkdir node-tools
        tar -xf "$pkg" --strip-components=1 -C node-tools
    )
    export PATH="$work/node-tools/bin:$PATH"
fi
export CARGO_TARGET_DIR="$PWD/target"
npm --prefix web ci
npm --prefix web run build
cargo +stable build --locked --release --workspace
stage="$work/release"
for bin in asterion-server asterion-agent asterion-docker-reader asterion-docker-operator asterion-account-operator asterion-terminal-helper; do
    install -Dm755 "$CARGO_TARGET_DIR/release/$bin" "$stage/bin/$bin"
done
install -d "$stage/web" "$stage/deploy/managed" "$stage/deploy/center"
cp -a web/dist/. "$stage/web/"
install -m 0644 deploy/*.service deploy/*.env.example deploy/*.example.json deploy/self_monitor.py deploy/install-center.sh "$stage/deploy/"
install -m 0644 deploy/managed/{service,common,containers,console,host}.py "$stage/deploy/managed/"
install -m 0644 deploy/center/{install,host,inputs,existing,pki,services,docker_reader,docker_probe}.py deploy/center/*.in "$stage/deploy/center/"
tar -C "$stage" -czf "$work/$archive" bin web deploy
(cd "$work" && sha256sum "$archive" > "$archive.sha256")
mv -fT -- "$work/$archive" "$HOME/$archive"
mv -fT -- "$work/$archive.sha256" "$HOME/$archive.sha256"
printf '发布包已保存到 %s/%s\n' "$HOME" "$archive"
