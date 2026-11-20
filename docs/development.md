# 本地开发

在 WSL 的 Ubuntu 项目目录执行 `bash start.sh`，启动器会构建前后端并在本机监听。需要 Linux 版 Rust stable、Node.js 24、npm 和 OpenSSH 客户端。接入步骤见[本机使用](local.md)。以下步骤用于开发使用 HTTPS 的常驻中心，另需 OpenSSL。

## 1. 准备工作区

```bash
cargo --version
node --version
ssh -V
umask 077
mkdir -p .local/dev
npm --prefix web ci
npm --prefix web run build
cargo build --locked -p asterion-server
```

## 2. 生成本地 HTTPS 证书并设置管理员

将下列占位符替换为本次开发的值。证书的 DNS 名应与浏览器访问地址一致。真实配置、口令、密钥和数据库放在 Git 忽略的 `.local/` 中。

```bash
export DEV_NAME='<本地访问域名>'
export DEV_PORT='<开发监听端口>'
openssl req -x509 -newkey rsa:3072 -nodes -days 14 -subj "/CN=$DEV_NAME" -addext "subjectAltName=DNS:$DEV_NAME" -keyout .local/dev/tls.key -out .local/dev/tls.crt
read -rsp '面板管理员口令（12 至 128 字节） ' initial_password
printf '%s\n' "$initial_password" > .local/dev/admin-password
unset initial_password
export ASTERION_ADMIN_USERNAME='<管理员名称>'
export ASTERION_ADMIN_PASSWORD_FILE="$PWD/.local/dev/admin-password"
export ASTERION_TLS_CERT_FILE="$PWD/.local/dev/tls.crt"
export ASTERION_TLS_KEY_FILE="$PWD/.local/dev/tls.key"
export ASTERION_WEB_DIR="$PWD/web/dist"
export ASTERION_DATABASE_URL="sqlite://.local/dev/center.db"
export ASTERION_LISTEN_ADDR="127.0.0.1:$DEV_PORT"
```

## 3. 接入 SSH 节点

在已有 SSH 登录权限的测试节点上，确认目标系统有 Python 3.9 或以上版本。按[连接配置](monitoring-and-operations.md)在 `.local/dev/` 准备 `ssh.key`、`known_hosts` 和 `profiles.json`。只验证面板时可以先使用空配置。

```bash
test -f .local/dev/profiles.json || printf '{}\n' > .local/dev/profiles.json
export ASTERION_SSH_CONFIG_FILE="$PWD/.local/dev/profiles.json"
target/debug/asterion-server
```

在浏览器中信任本地证书，打开 `https://<本地访问域名>:<开发监听端口>/login`。登录后在「节点 → 接入服务器」选择连接。首个采样完成后可查看指标、实时资源和操作面板。已有数据库会自动迁移，面板管理员只在首次启动时初始化。

## 4. 验证与停止

在另一个 WSL 终端按改动范围运行检查。前端修改后重新构建并刷新。使用 Ctrl+C 停止前台中心。

```bash
cargo fmt --all -- --check
cargo test --locked -p asterion-server --lib ssh
python3 -B -m unittest discover -s deploy/ssh
python3 -B -m unittest discover -s deploy/managed
npm --prefix web run check
npm --prefix web run build
```
