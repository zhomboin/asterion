#!/usr/bin/env bash
# 在一次性 CI 主机上用实际的 Certbot、Nginx 和本地 ACME 验证服务器检查签发与续期。
set -euo pipefail
[[ ${GITHUB_ACTIONS:-} == true && -n ${RUNNER_TEMP:-} ]]
work=$(mktemp -d "$RUNNER_TEMP/asterion-acme.XXXXXXXX")
GOBIN="$work" go install github.com/letsencrypt/pebble/v2/cmd/pebble@v2.8.0
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj '/CN=localhost' \
    -addext 'subjectAltName=DNS:localhost' -addext 'basicConstraints=critical,CA:TRUE' \
    -keyout "$work/api.key" -out "$work/api.crt"
cat > "$work/config.json" <<EOF
{"pebble":{
    "listenAddress":"127.0.0.1:14000","managementListenAddress":"127.0.0.1:15000",
    "certificate":"$work/api.crt","privateKey":"$work/api.key","httpPort":80
}}
EOF
printf '127.0.0.1 panel.example.invalid\n' | sudo tee -a /etc/hosts >/dev/null
PEBBLE_VA_NOSLEEP=1 PEBBLE_WFE_NONCEREJECT=0 PEBBLE_AUTHZREUSE=0 "$work/pebble" -config "$work/config.json" &
pebble_pid=$!
trap 'kill "$pebble_pid" 2>/dev/null || true; wait "$pebble_pid" 2>/dev/null || true' EXIT
curl --fail --silent --show-error --retry 10 --retry-connrefused --retry-delay 1 \
    --cacert "$work/api.crt" https://localhost:15000/roots/0 -o "$work/issuer.crt"
sudo install -m 0644 "$work/api.crt" /usr/local/share/ca-certificates/asterion-ci-api.crt
sudo install -m 0644 "$work/issuer.crt" /usr/local/share/ca-certificates/asterion-ci-issuer.crt
sudo update-ca-certificates
# 模拟实际安装终端，避免 Certbot 把验收当作后台任务而随机延迟续期。
sudo --preserve-env=GITHUB_ACTIONS script -qec \
    '/usr/bin/python3 -I -B .github/workflows/check_center_install.py' /dev/null
