# 本地开发

在 Windows PowerShell 中运行 `wsl -d Ubuntu`。进入 WSL 后，将以下命令中的目录占位符替换为自己的 Asterion 工作区。后续命令均在 WSL Ubuntu 的 Bash 中执行。

```bash
cd '<Asterion 工作区>'
```

## 1. 准备工具

本地验证新的服务器操作使用 Python 标准库和隔离测试，不要求安装 Docker 或变更主机配置。完成工具准备后，按[本地运维验证](managed-operations.md#本地运维验证)执行。

```bash
sudo apt-get update
sudo apt-get install -y build-essential ca-certificates curl git openssl python3 sqlite3 xz-utils
```

如尚未安装 Rust，下载并运行官方安装程序。[Rust 安装说明](https://rust-lang.org/install)

```bash
mkdir -p .local
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o .local/rustup-init.sh
sh .local/rustup-init.sh --profile minimal --default-toolchain stable --no-modify-path
. "$HOME/.cargo/env"
```

```bash
rustup toolchain install stable
rustup component add --toolchain stable rustfmt clippy
cargo +stable --version
```

前端使用 Node.js 24。如需安装，将版本号和架构替换为下载页所列发行版。[Node.js 下载页](https://nodejs.org/en/download)

```bash
node_version='<Node.js 24 的完整发行版本>'
node_arch='<linux-x64 或 linux-arm64>'
node_package="node-$node_version-$node_arch"
mkdir -p .local/node
curl -fsSLo ".local/node/$node_package.tar.xz" "https://nodejs.org/dist/$node_version/$node_package.tar.xz"
curl -fsSLo .local/node/SHASUMS256.txt "https://nodejs.org/dist/$node_version/SHASUMS256.txt"
(
  set -e
  cd .local/node
  awk -v file="$node_package.tar.xz" '$2 == file' SHASUMS256.txt > "$node_package.sha256"
  sha256sum --check "$node_package.sha256"
  tar -xf "$node_package.tar.xz"
)
export PATH="$PWD/.local/node/$node_package/bin:$PATH"
node --version
npm --version
```

使用 npm 的终端须加载该 `PATH`，或使用已安装的 Node.js 24。

## 2. 准备本地证书

首次开发时，在忽略的 `.local/tls` 目录创建本地 CA 和包含回环地址的服务端证书。已有可用证书时复用对应 PEM 文件。

```bash
(
  set -e
  umask 077
  install -d -m 0700 .local/tls
  openssl req -x509 -newkey rsa:3072 -nodes -sha256 -days 30 \
    -subj '/CN=Asterion Development CA' \
    -addext 'basicConstraints=critical,CA:TRUE,pathlen:0' \
    -addext 'keyUsage=critical,keyCertSign,cRLSign' \
    -keyout .local/tls/ca.key -out .local/tls/ca.crt
  openssl req -new -newkey rsa:2048 -nodes -sha256 -subj '/CN=localhost' \
    -keyout .local/tls/center.key -out .local/tls/center.csr
  openssl x509 -req -sha256 -days 30 -CA .local/tls/ca.crt -CAkey .local/tls/ca.key \
    -CAcreateserial -in .local/tls/center.csr -out .local/tls/center.crt \
    -extfile <(printf '%s\n' 'basicConstraints=critical,CA:FALSE' \
      'keyUsage=critical,digitalSignature,keyEncipherment' 'extendedKeyUsage=serverAuth' \
      'subjectAltName=DNS:localhost,IP:127.0.0.1')
  openssl verify -CAfile .local/tls/ca.crt -verify_ip 127.0.0.1 .local/tls/center.crt
)
```

将 `.local/tls/ca.crt` 导入开发浏览器使用的用户信任库。使用 Windows 浏览器时，运行 `certmgr.msc` 打开当前用户证书库，在「受信任的根证书颁发机构 → 证书」中选择导入。只导入 CA 证书，私钥留在受保护的目录中。开发结束后可从浏览器信任库移除此 CA。

## 3. 启动中心

首次启动需要一个自定义管理员名及口令文件。用户名可以使用字母、数字、点、下划线、连字符和 `@`，长度为 1 到 64。口令为 12 到 128 个 UTF-8 字节的单行文本。使用终端隐式输入，避免把口令写进命令历史。

```bash
export ASTERION_ADMIN_USERNAME='<管理员用户名>'
read -rsp '管理员口令：' asterion_admin_secret
printf '\n'
(umask 077; printf '%s\n' "$asterion_admin_secret" > .local/admin-password)
unset asterion_admin_secret
chmod 0600 .local/admin-password
export ASTERION_ADMIN_PASSWORD_FILE="$PWD/.local/admin-password"
```

在第一个 Bash 终端启动中心。先前已经初始化的数据库可以省略两个管理员变量。管理员口令文件仅用于初始化新库。

```bash
export ASTERION_TLS_CERT_FILE="$PWD/.local/tls/center.crt"
export ASTERION_TLS_KEY_FILE="$PWD/.local/tls/center.key"
cargo +stable run --locked -p asterion-server
```

在另一终端验证健康接口和数据库表。

```bash
curl --fail --silent --show-error --cacert .local/tls/ca.crt https://127.0.0.1:8080/healthz
sqlite3 -readonly .local/asterion.db '.tables'
```

健康接口返回 `{"status":"ok"}`。中心配置如下。

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `ASTERION_LISTEN_ADDR` | `127.0.0.1:8080` | HTTPS 监听地址 |
| `ASTERION_DATABASE_URL` | `sqlite://.local/asterion.db` | SQLite 文件地址，自定义父目录须预先创建并可写 |
| `ASTERION_WEB_DIR` | 空 | 中心托管的前端构建目录 |
| `ASTERION_TLS_CERT_FILE` | 必填 | PEM 格式 TLS 证书链文件 |
| `ASTERION_TLS_KEY_FILE` | 必填 | PEM 格式 TLS 私钥文件 |
| `ASTERION_ADMIN_USERNAME` | 首次启动必填 | 单管理员用户名 |
| `ASTERION_ADMIN_PASSWORD_FILE` | 首次启动必填 | 权限为 0400 或 0600 的口令文件 |

## 4. 启动前端

另开终端，使 Vite 使用本地 HTTPS，并验证它代理的中心证书。中心地址变化时设置 `ASTERION_DEV_SERVER_URL` 为相应 HTTPS 源地址。

```bash
npm --prefix web ci
export ASTERION_TLS_CERT_FILE="$PWD/.local/tls/center.crt"
export ASTERION_TLS_KEY_FILE="$PWD/.local/tls/center.key"
export ASTERION_DEV_CA_FILE="$PWD/.local/tls/ca.crt"
npm --prefix web run dev
```

打开 `https://127.0.0.1:5173/login`，用初始化时的管理员口令登录。开发完成后在 Vite 终端按 `Ctrl+C`。

验证正式 Web 产物时，先停止原中心进程，设置与上一步相同的服务端 TLS 证书和私钥，然后启动中心。

```bash
npm --prefix web run build
ASTERION_WEB_DIR="$PWD/web/dist" cargo +stable run --locked -p asterion-server
```

浏览器打开 `https://127.0.0.1:8080/login`，确认生产构建可登录。

## 5. 注册一个本地 Agent

在面板打开「节点 → 接入服务器」，选择 WSL 并生成一次性注册凭证。弹窗提供连接配置和接入步骤。若 Agent 与中心均在同一个 WSL 环境，可使用下面命令中的中心地址。注册凭证有效期为 10 分钟。为每个 Agent 使用独立的身份目录。

```bash
install -d -m 0700 .local/agent-wsl
read -rsp '注册凭证：' asterion_enrollment
printf '\n'
(umask 077; printf '%s\n' "$asterion_enrollment" > .local/agent-wsl/enrollment)
unset asterion_enrollment
chmod 0600 .local/agent-wsl/enrollment
export ASTERION_SERVER_URL='https://127.0.0.1:8080'
export ASTERION_AGENT_CA_FILE="$PWD/.local/tls/ca.crt"
export ASTERION_AGENT_STATE_FILE="$PWD/.local/agent-wsl/identity.json"
export ASTERION_AGENT_ENROLLMENT_FILE="$PWD/.local/agent-wsl/enrollment"
cargo +stable run --locked -p asterion-agent
```

在总览核对节点在线、最近心跳、系统与版本。按 `Ctrl+C` 停止 Agent，等待约 20 秒及下一次页面刷新，确认节点离线。去掉 `ASTERION_AGENT_ENROLLMENT_FILE` 后用相同身份文件重启，确认同一节点重新上线。

```bash
unset ASTERION_AGENT_ENROLLMENT_FILE
cargo +stable run --locked -p asterion-agent
```

验证完成后在 Agent 和中心终端分别按 `Ctrl+C`。注册成功后可删除已消费的注册文件，保留身份文件供重连。

`ASTERION_SERVER_URL` 接受中心 HTTPS 源地址或完整的 `wss://…/api/v1/agent/connect`。私有 CA 通过 `ASTERION_AGENT_CA_FILE` 加入系统信任根；使用系统已信任的签发机构时可以省略。身份目录必须由 Agent 用户持有且权限为 0700。注册文件和身份文件仅由 Agent 用户读取，权限为 0400 或 0600。

## 6. 检查改动

根据改动范围运行对应检查。提交前格式化改动，完整检查在 GitHub Actions 运行。

```bash
cargo +stable fmt --all
cargo +stable test --locked -p asterion-server --test auth
cargo +stable test --locked -p asterion-server --lib nodes::tests
cargo +stable test --locked -p asterion-agent
npm --prefix web run check
npm --prefix web run build
```

## 7. 本地验收和依赖检查

保持两个 Agent 实例运行，分别使用不同的身份目录。在总览确认两个节点同时上线。打开其中一个节点的「指标」和「进程」，核对当前 WSL 的数据，并切换最近 1 小时、24 小时和长趋势视图。停止其中一个 Agent，确认另一个节点的查询继续工作，最后心跳和已有采样保留。

在测试节点的 systemd 页签打开预期持续运行的 Unit 详情，选择「启用监控」，查看最近检查时间和「状态历史」。退出面板再打开，确认中心继续检查。只停止和重连本次验证的 Agent，确认 Unit 暂显示未知，连接恢复后重新报告实际状态。在隔离的测试节点使用专用验证服务核对停止和恢复告警。告警仍在进行时选择「停用监控」，核对结束原因。[单个服务监控](monitoring-and-operations.md#单个-systemd-服务监控)

Docker 读取、受控任务、Linux 账号变更和终端按[采集与运维配置](monitoring-and-operations.md)启用。在本地试用任务或终端时，使用测试 Agent 和临时目录。可以先用运维策略样例中的 `echo_value` 检查确认、结果和审计。启用真实主机能力前，确认本机的 Engine、Unit 或账号属于授权范围。

```bash
cargo +stable test --workspace --locked
cargo +stable clippy --workspace --all-targets --locked -- -D warnings
npm --prefix web test
python3 scripts/dependency_licenses.py > .local/dependency-licenses.tsv
```

GitHub Actions 检查 Rust 和 Web，并生成锁定依赖的许可证清单。开发完毕后在各运行终端按 `Ctrl+C` 停止临时中心、Agent、Web 和可选采集服务。
