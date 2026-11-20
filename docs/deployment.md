# Ubuntu 服务器部署

本文的 Bash 命令在服务器上执行。中心主机运行 Asterion、SQLite 和监控自身的 Agent。同机的 Nginx 负责域名 HTTPS 入口，Asterion 只监听回环地址。远端 Agent 通过域名上报；部署 SSH 会话结束后即可断开。中心支持使用 systemd 的 Ubuntu 22.04 和 24.04。

## 一键初始化中心

在中心服务器上以 root 登录，进入可信的项目 Git 检出目录，直接运行：

```bash
bash deploy/install-center.sh
```

**已部署的中心只需补上 Docker 采集时**，请使用含此向导的最新项目脚本，运行上面同一条命令，选择 **1「仅配置本机 Docker 只读采集」**。向导直接复用已安装的发布和本机 Agent 身份，无需原归档或面板口令。选择 **2** 会继续完整部署，选择 **3** 或直接回车退出。首次完整部署检测到系统 Docker 时，也会询问是否启用采集，默认跳过。

向导会创建或复用专用读取账号及访问组，启动读取服务，让本机 Agent 加载读取组和 Socket 配置。权限生效后，用 Agent 的实际身份读取一遍容器清单。看到“Docker 只读采集已验证”后，打开「中心服务器 → Docker → 添加监控」，选择要纳管的容器。清单返回 0 个表示 Engine 已连通但此次未返回容器。再次选择启用会复核现有配置，已生效的 Agent 继续运行。

自动配置适用于系统级 Docker 的标准 Socket（`/run/docker.sock`，属主 root、属组 docker，允许组写入）。没有本机 Engine 时会提示跳过。标准 Docker 服务已安装但未运行时，可以先配好读取服务，待 Engine 启动后刷新。使用用户级 Docker、自定义 Socket 或访问组时，按下方 [Docker 读取](#docker-读取)一节配置实际端点。个人运维账号无需额外授权，容器启停、重建和终端操作仍需单独授权。

**要从面板修改服务配置或操作容器**，已有中心重跑同一条命令，选 **4「配置服务文件编辑与重启、Docker 容器操作」**。再选 **1** 配服务、**2** 配容器、**3** 两项都配，或 **0** 跳过。新装完成后也会出现这组选项。向导复用本机已安装的发布、账号和 Agent，自动配置所选操作服务、访问组和凭据。每一项如何填写见[节点操作向导](managed-operations.md#中心一键启用)。

目标系统为使用 systemd 的 Ubuntu 22.04 或 24.04。先将面板域名的 A 记录指向中心 IPv4，确保公网 TCP 80 可以到达中心 Nginx，TCP 443 用于面板和 Agent。使用包含本安装向导的项目检出目录。向导会在当前目录和运行账号的家目录查找 `asterion-*.tar.gz`，校验文件须与包放在一起。找到一个包会直接使用；找到多个包会列出路径供选择。包放在其他目录时，可按第 1 节显式指定路径。

只有 root 账号也可以从头安装。找不到发布包时，脚本会安装构建依赖，从当前提交编译并打包，然后继续部署。也可以用有 sudo 权限的普通用户执行同一条命令。

向导询问的是日常运维用户名。已经手动创建过账号，就填写原用户名，向导会复用账号和 SSH 公钥；首次安装可填写要创建的普通用户名。向导会从现有中心配置和标准 Nginx 站点读取域名和端口。管理电脑可以使用动态公网 IP。向导只在需要通过 SSH 安装远端 Agent 时使用目标节点地址。

向导会把已解包的文件与所选归档逐一核对，补齐缺少的文件；核验内部证书、私钥和手动暂存的签发材料；识别 SQLite 中的管理员、待接入凭证和本机 Agent 身份。已经运行且配置生效的服务直接做健康检查。仅在注册、启用变更或退出一次性凭据配置时重启相关服务。

已有受信任的密钥和有效证书可直接使用。需要补签内部证书时输入原 CA 私钥口令；需要用已有面板账号登记 Agent 时输入原管理员口令；新签公网证书时填写你能收信的常用邮箱，Certbot 会自动注册账户，并用 HTTP 验证域名。已有 Certbot 证书沿用其续期配置。

按标准模板部署且已限制来源 IP 的站点，重跑后会切换为凭账号访问。Nginx 对登录接口按来源限速，管理 API 继续校验会话。自定义 Nginx 规则需要手动合并。

出现发布内容不同、同名服务指向其他程序、证书与私钥不配对或节点身份不符时，向导会保留现场，并提示具体冲突。核对原发布包或配置后重跑即可。看到“部署完成”后，从管理电脑打开输出的登录地址，确认「中心服务器」在线。更新已安装版本见第 7 节。

## 1. 单独准备或使用发布包

已有包须适配目标机器的架构与系统运行库；面向两个版本发布时，可在同架构的 Ubuntu 22.04 上构建并在两版验收。如需单独打包，可用 root 或有 sudo 权限的普通用户在构建机的项目目录运行：

```bash
bash deploy/build-release.sh
```

生成的 `asterion-<构建编号>.tar.gz` 和配套的 `.tar.gz.sha256` 位于构建账号的家目录。将这两个文件复制到中心后，在项目目录运行以下命令即可使用指定包。

```bash
bash deploy/install-center.sh "$HOME/asterion-<构建编号>.tar.gz"
```

以下第 2、3 节供逐步部署、自定义已有站点和恢复时查阅。一键安装成功后，可直接从第 4 节接入远端节点。

## 2. 配置服务器与个人运维用户

在 DNS 中将面板域名的 A 记录指向中心公网 IPv4。管理电脑和远端 Agent 使用同一 Nginx HTTPS 入口；中心内部监听选择空闲的高端口，不对公网开放。为第 3 节准备可收信的联系邮箱。首次签发及自动续期需要公网 TCP 80 可达 Nginx。

手动部署从放有发布包的目录开始，后续命令在 root Bash 中执行。以普通管理员身份登录时，在该目录用 `sudo bash` 进入 root 会话。私钥、证书、数据库和实际部署参数放在服务器私有目录。仓库中的样例保留占位符，本机留存的实际配置可放在 Git 忽略的 `.local/` 目录。

```bash
cd '<发布包所在目录>'
```

填入自己的参数。如使用个人配置说明中的参数块，可直接代入下方同名变量。

```bash
set -euo pipefail
release_id='<构建编号>'
domain='<面板域名>'
app_port='<内部监听端口>'
node_ip='<允许接入的远端节点 IPv4>'
ops='<个人 Linux 运维用户名>'
```

确认当前是首次安装，校验归档和配置，再准备系统依赖。已有中心或 Agent 时，请使用第 7 节的升级步骤。

```bash
test "$(id -u)" = 0
[[ $release_id =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ && $ops =~ ^[a-z][a-z0-9-]{0,23}$ ]]
python3 - "$domain" "$app_port" "$node_ip" <<'PY'
import ipaddress
import re
import sys

domain, port, node = sys.argv[1:]
labels = domain.split(".")
if len(labels) < 2 or labels[-1].isdigit() or len(domain) > 253 or not all(re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label) for label in labels):
    raise SystemExit("请填写有效的面板域名")
if not port.isascii() or not port.isdecimal() or not 10000 <= int(port) <= 65535:
    raise SystemExit("请选择有效的内部高端口")
ipaddress.IPv4Address(node)
PY
getent ahostsv4 "$domain" | awk '{print $1}' | sort -u
archive="asterion-$release_id.tar.gz"
sha256sum --check "$archive.sha256"
if test -n "$(ss -H -lnt "sport = :$app_port")"; then echo '内部端口已被占用'; exit 1; fi
nginx -t
systemctl is-active nginx
apt-get update
apt-get install -y openssl ca-certificates curl python3-paramiko sqlite3 sudo openssh-client
/usr/bin/python3 -I -c 'import paramiko'
if getent passwd "$ops" >/dev/null || getent group "$ops" >/dev/null; then
  echo '个人账号或同名组已存在，请先检查'; exit 1
fi
for path in /opt/asterion /etc/asterion /var/lib/{,private/}asterion \
  /opt/asterion-agent /etc/asterion-agent /var/lib/{,private/}asterion-agent \
  /etc/systemd/system/asterion-agent.service{,.d} /run/systemd/system/asterion-agent.service.d \
  /run/asterion-self-enrollment /run/asterion-setup \
  /etc/systemd/system/asterion-{server.service,server.service.d} \
  /run/systemd/system/asterion-server.service.d /run/asterion-first-admin \
  "/etc/sudoers.d/asterion-$ops"; do
  if test -e "$path" || test -L "$path"; then printf '已有路径：%s\n' "$path"; exit 1; fi
done
install -d -m 0700 /run/asterion-setup
archive_path="$PWD/$archive"
cd /run/asterion-setup
read -rp '粘贴个人运维账号的 SSH 公钥：' ops_public_key
printf '%s\n' "$ops_public_key" > ops-key.pub
ssh-keygen -lf ops-key.pub
```

核对公钥指纹后创建个人账号。面板使用独立的管理员口令。Linux 运维用户只获得查看中心最近 100 行日志和重启该服务的 sudo 权限。

```bash
adduser --disabled-password --gecos '' "$ops"
ops_home=$(getent passwd "$ops" | cut -d: -f6)
ops_group=$(id -gn "$ops")
install -d -m 0700 -o "$ops" -g "$ops_group" "$ops_home/.ssh"
install -m 0600 -o "$ops" -g "$ops_group" ops-key.pub "$ops_home/.ssh/authorized_keys"
rule=$(mktemp)
printf '%s ALL=(root) NOPASSWD: /usr/bin/systemctl restart asterion-server.service, /usr/bin/journalctl --no-pager -u asterion-server.service -n 100\n' "$ops" > "$rule"
visudo -cf "$rule"
install -o root -g root -m 0440 "$rule" "/etc/sudoers.d/asterion-$ops"
unlink "$rule"
sudo -l -U "$ops"
```

## 3. 安装中心、配置 Nginx 并开启自监控

本节配置两张用途不同的证书，不要互换：

| 连接 | 使用的证书 | 续期责任 |
| --- | --- | --- |
| 浏览器、远端 Agent → Nginx 域名入口 | Let's Encrypt 公网证书 | 中心的 `certbot.timer` 定期检查；续期成功后检查并重载 Nginx |
| Nginx、同机 Agent → 中心回环地址 | 内部 CA 签发的中心证书 | 运维人员在到期前用内部 CA 续签，更换证书后重启中心服务；不在 Certbot 管理范围内 |

先生成内部 CA 和回环证书。CA 私钥以口令加密，备份到受保护的位置供续签。

```bash
install -d -m 0700 /etc/asterion/pki
openssl genrsa -aes256 -out /etc/asterion/pki/ca.key 3072
openssl req -x509 -new -sha256 -days 1825 -key /etc/asterion/pki/ca.key \
  -subj '/CN=Asterion Internal CA' -addext 'basicConstraints=critical,CA:TRUE,pathlen:0' \
  -addext 'keyUsage=critical,keyCertSign,cRLSign' -out ca.crt
(umask 077; openssl req -new -newkey rsa:3072 -nodes -subj /CN=localhost -keyout tls.key -out tls.csr)
openssl x509 -req -sha256 -days 180 -in tls.csr -CA ca.crt -CAkey /etc/asterion/pki/ca.key \
  -CAserial /etc/asterion/pki/ca.srl -CAcreateserial \
  -extfile <(printf '%s\n' 'basicConstraints=critical,CA:FALSE' 'extendedKeyUsage=serverAuth' \
    'keyUsage=critical,digitalSignature,keyEncipherment' 'subjectAltName=DNS:localhost,IP:127.0.0.1') \
  -out tls.crt
openssl verify -purpose sslserver -verify_hostname localhost -CAfile ca.crt tls.crt
openssl verify -purpose sslserver -verify_ip 127.0.0.1 -CAfile ca.crt tls.crt
```

安装发布包和内部 TLS 配置。远端 Agent 使用系统信任库验证公网域名证书，中心自身的 Agent 使用内部 CA。中心只向指定的 Agent 目标发起 SSH 部署。

```bash
release="/opt/asterion/releases/$release_id"
install -d -m 0755 /opt/asterion /opt/asterion/releases "$release"
tar -xzf "$archive_path" --no-same-owner -C "$release"
chmod -R u=rwX,go=rX "$release"
ln -s "$release" /opt/asterion/current
install -d -m 0750 /etc/asterion
install -m 0600 tls.key /etc/asterion/tls.key
install -m 0644 tls.crt /etc/asterion/tls.crt
install -m 0644 ca.crt /etc/asterion/deploy-ca.crt
install -m 0644 "$release/deploy/asterion-server.service" /etc/systemd/system/
cat > /etc/asterion/server.env <<CONF
ASTERION_LISTEN_ADDR=127.0.0.1:$app_port
ASTERION_DATABASE_URL=sqlite:///var/lib/asterion/asterion.db
ASTERION_WEB_DIR=/opt/asterion/current/web
ASTERION_SSH_DEPLOY_AGENT=/opt/asterion/current/bin/asterion-agent
ASTERION_SSH_DEPLOY_CENTER_URL=https://$domain
ASTERION_SSH_DEPLOY_NETWORKS=$node_ip/32
ASTERION_TERMINAL_ENABLED=0
CONF
chmod 0600 /etc/asterion/server.env
```

给个人面板管理员设置 12 到 128 字节的口令，终端不会回显。临时初始化配置存放在 `/run`。

```bash
read -rsp '设置面板管理员口令：' secret; echo
read -rsp '再次输入：' again; echo
size=$(printf %s "$secret" | wc -c)
[[ "$secret" == "$again" && $size -ge 12 && $size -le 128 && $secret =~ [^[:space:]] ]]
(umask 077; printf '%s\n' "$secret" > /run/asterion-first-admin)
unset secret again
install -d -m 0755 /run/systemd/system/asterion-server.service.d
cat > /run/systemd/system/asterion-server.service.d/10-bootstrap.conf <<UNIT
[Service]
LoadCredential=admin-password:/run/asterion-first-admin
RuntimeDirectory=asterion-bootstrap
RuntimeDirectoryMode=0700
ExecStart=
ExecStart=/bin/sh -ec '/usr/bin/install -m 0600 /run/credentials/%n/admin-password /run/asterion-bootstrap/admin-password; exec /opt/asterion/current/bin/asterion-server'
Environment=ASTERION_ADMIN_USERNAME=${ops}-panel
Environment=ASTERION_ADMIN_PASSWORD_FILE=/run/asterion-bootstrap/admin-password
UNIT
printf '面板用户名：%s-panel\n' "$ops"
```

在同一 root Bash 中，保留第 2 节设置的 `domain`，先[准备 HTTP 验证](public-tls.md#准备-http-验证)，再[签发域名证书并启用自动续期](public-tls.md#签发证书并启用自动续期)。确认定时器有下次运行时间、模拟续期成功后回到这里。此时只检查磁盘上的证书；Nginx 实际提供的证书在站点配置完成后验收。

```bash
public_chain="/etc/letsencrypt/live/$domain/fullchain.pem"
public_key="/etc/letsencrypt/live/$domain/privkey.pem"
test -s "$public_chain"
test -s "$public_key"
openssl verify -purpose sslserver -verify_hostname "$domain" \
  -CAfile /etc/ssl/certs/ca-certificates.crt -untrusted "$public_chain" "$public_chain"
test "$(openssl x509 -in "$public_chain" -noout -pubkey)" = "$(openssl pkey -in "$public_key" -pubout)"
```

中心首先按内部端口启动。在本机确认后，再接入 Nginx 的 HTTPS 站点。

```bash
systemctl daemon-reload
systemd-analyze verify asterion-server.service
systemctl enable --now asterion-server.service
curl --noproxy '*' --retry 8 --retry-connrefused --retry-delay 1 -fsS \
  --cacert /etc/asterion/deploy-ca.crt https://127.0.0.1:$app_port/healthz
```

下面按同机 Ubuntu Nginx 的 `http` 配置已包含 `/etc/nginx/conf.d/*.conf` 编写。已有该域名站点时，将 `map` 放在 `http` 级，把 `server` 中的证书路径、代理、域名检查与访问控制合并到现有站点。新站点可直接执行下列命令。公网登录、API 和 Agent WebSocket 共用同一域名，代理到中心时校验证书。[Nginx WebSocket 配置](https://nginx.org/en/docs/http/websocket.html) · [上游 TLS 校验](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_ssl_verify)

```bash
site_file=/etc/nginx/conf.d/$domain.conf
test -d /etc/nginx/conf.d
test ! -e "$site_file" && test ! -L "$site_file" || { echo '请合并到已有站点配置'; exit 1; }
if nginx -T 2>/dev/null | awk -v host="$domain" '
  $1 == "server_name" {gsub(/;/, " "); for (i=2; i<=NF; i++) if ($i == host) found=1}
  END {exit !found}'; then
  echo '该域名已有站点，请合并配置'; exit 1
fi
cat > "$site_file" <<NGINX
map \$http_upgrade \$asterion_connection_upgrade {
    default upgrade;
    '' close;
}
map \$uri \$asterion_login_client {
    default "";
    /api/v1/auth/login \$binary_remote_addr;
}
limit_req_zone \$asterion_login_client zone=asterion_login:1m rate=6r/m;
server {
    listen 443 ssl;
    server_name $domain;
    ssl_certificate "$public_chain";
    ssl_certificate_key "$public_key";

    if (\$ssl_server_name != "$domain") { return 444; }
    if (\$host != "$domain") { return 444; }
    limit_req zone=asterion_login burst=5 nodelay;
    limit_req_status 429;

    location / {
        proxy_pass https://127.0.0.1:$app_port;
        proxy_ssl_server_name on;
        proxy_ssl_name localhost;
        proxy_ssl_trusted_certificate /etc/asterion/deploy-ca.crt;
        proxy_ssl_verify on;

        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection \$asterion_connection_upgrade;
        proxy_read_timeout 1h;
        proxy_send_timeout 1h;
    }
}
NGINX
if nginx -t; then
  systemctl reload nginx
else
  mv -- "$site_file" "$site_file.failed-$(date +%s)"
  echo 'Nginx 配置检查失败，已退出加载目录的 *.conf 匹配范围'
  exit 1
fi
curl --noproxy '*' --resolve "$domain:443:127.0.0.1" -fsS "https://$domain/healthz"
```

已有域名站点时，也将该站点的 `ssl_certificate` 和 `ssl_certificate_key` 指向上述 Certbot 路径，检查并重载 Nginx。通过[证书验收](public-tls.md#验证-nginx-实际提供的证书)核对域名当前使用的证书。

### 安装并注册监控中心自身的 Agent

继续在中心的 root Bash 执行。Agent 复用中心发布包中的二进制，使用专用 systemd 身份和持久目录。回环 HTTPS 健康检查应返回 `{"status":"ok"}`。

```bash
curl --noproxy '*' --cacert /etc/asterion/deploy-ca.crt -fsS https://127.0.0.1:$app_port/healthz
install -d -m 0755 /opt/asterion-agent
ln -s /opt/asterion/current /opt/asterion-agent/current
install -m 0644 /opt/asterion/current/deploy/asterion-agent.service /etc/systemd/system/
install -d -m 0750 /etc/asterion-agent
cat > /etc/asterion-agent/agent.env <<CONF
ASTERION_SERVER_URL=https://127.0.0.1:$app_port
ASTERION_AGENT_STATE_FILE=/var/lib/asterion-agent/identity.json
CONF
chmod 0600 /etc/asterion-agent/agent.env
install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
cat > /etc/systemd/system/asterion-agent.service.d/20-local.conf <<'UNIT'
[Unit]
Wants=asterion-server.service
After=asterion-server.service
[Service]
LoadCredential=ca.pem:/etc/asterion/deploy-ca.crt
Environment=ASTERION_AGENT_CA_FILE=/run/credentials/%n/ca.pem
UNIT
```

下面通过本机面板 API 自动登记「中心服务器」，一次性注册凭证仅存入 root 可读的运行时目录。登记使用首次启动时的面板口令，完成后注销临时登录会话。[自监控部署脚本](../deploy/self_monitor.py)

```bash
self_monitor=/opt/asterion/current/deploy/self_monitor.py
python3 "$self_monitor" register --port "$app_port" --username "$ops-panel"
install -d -m 0755 /run/systemd/system/asterion-agent.service.d
cat > /run/systemd/system/asterion-agent.service.d/10-self-enroll.conf <<'UNIT'
[Service]
LoadCredential=enrollment:/run/asterion-self-enrollment/token
RuntimeDirectory=asterion-agent-bootstrap
RuntimeDirectoryMode=0700
ExecStart=
ExecStart=/bin/sh -ec '/usr/bin/install -m 0600 /run/credentials/%n/enrollment /run/asterion-agent-bootstrap/enrollment; exec /opt/asterion-agent/current/bin/asterion-agent'
Environment=ASTERION_AGENT_ENROLLMENT_FILE=/run/asterion-agent-bootstrap/enrollment
UNIT
systemctl daemon-reload
systemd-analyze verify asterion-agent.service
systemctl enable --now asterion-agent.service
```

验证本机身份已经保存，且中心收到了该节点的新心跳和新指标。确认后移除一次性注册配置，重启 Agent 并复核重连。后续开机会自动使用已保存的身份。

```bash
python3 "$self_monitor" wait
unlink /run/systemd/system/asterion-agent.service.d/10-self-enroll.conf
unlink /run/asterion-self-enrollment/token
systemctl daemon-reload
systemctl restart asterion-agent.service
python3 "$self_monitor" wait
```

如果首次注册超过 10 分钟且面板仍显示「待接入」，在面板对已确认是本机的待接入节点换发一次性凭证。用 `read -rsp '新凭证：' token` 隐式输入，执行 `(umask 077; printf '%s\n' "$token" > /run/asterion-self-enrollment/token)` 和 `unset token`，然后重启 Agent，再执行上方的心跳与指标验证。已在线的本机节点可沿用保存的身份。

在浏览器打开面板域名的 `/login`，使用新建的面板管理员登录，确认「中心服务器」在线。然后在中心清理本次初始化口令和内部私钥的临时副本，重启中心并复核自监控。

```bash
test "$(pwd -P)" = /run/asterion-setup
cmp -s tls.key /etc/asterion/tls.key
unlink tls.key
unlink /run/systemd/system/asterion-server.service.d/10-bootstrap.conf
unlink /run/asterion-first-admin
systemctl daemon-reload
systemctl restart asterion-server.service
curl --noproxy '*' --retry 8 --retry-all-errors --retry-delay 1 -fsS \
  --resolve "$domain:443:127.0.0.1" "https://$domain/healthz"
systemctl is-active asterion-server.service asterion-agent.service nginx
python3 "$self_monitor" wait
```

## 4. 安装和注册远端 Agent

在公网面板「节点 → 接入服务器」填写远端主机的 SSH 信息。核对主机指纹和命令中的中心域名后确认安装，以「已收到 Agent 心跳」作为完成标志。已有节点可从「Agent 接入」重新部署。[SSH 接入步骤](ssh-enrollment.md)

### 手动安装远端 Agent

需要手动安装或启用可选运维程序时，先在中心的构建账号下将第 1 节生成的归档和校验文件复制到目标服务器。

```bash
release_id='<构建编号>'
archive="asterion-$release_id.tar.gz"
scp "$HOME/$archive" "$HOME/$archive.sha256" '<SSH用户>@<远端主机>:'
```

接下来在目标服务器执行。目标的架构和运行库须与发布包兼容。

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates
cd "$HOME"
release_id='<同一个构建编号>'
archive="asterion-$release_id.tar.gz"
sha256sum --check "$archive.sha256"
agent_release="/opt/asterion-agent/releases/$release_id"
sudo install -d -m 0755 "$agent_release"
sudo tar -xzf "$archive" --no-same-owner -C "$agent_release" \
  bin/asterion-agent bin/asterion-docker-reader bin/asterion-docker-operator bin/asterion-account-operator bin/asterion-terminal-helper deploy
sudo install -d -m 0750 /etc/asterion-agent
sudo install -m 0600 "$agent_release/deploy/agent.env.example" /etc/asterion-agent/agent.env
sudo install -m 0644 "$agent_release/deploy/asterion-agent.service" /etc/systemd/system/asterion-agent.service
sudo ln -sfnT "$agent_release" /opt/asterion-agent/current
sudoedit /etc/asterion-agent/agent.env
```

将 `ASTERION_SERVER_URL` 设为 `https://<面板域名>`。按本指南通过 Nginx 和 Let's Encrypt 接入时，远端 Agent 使用系统信任库，不设置 `ASTERION_AGENT_CA_FILE`，直接跳到注册凭证步骤。只有远端 Agent 确实连接私有 CA 签发的入口时，才执行下面两段配置。

```bash
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudo install -m 0644 '<私有 CA 的 PEM 文件>' /etc/asterion-agent/ca.pem
sudoedit /etc/systemd/system/asterion-agent.service.d/20-private-ca.conf
```

```ini
[Service]
LoadCredential=ca.pem:/etc/asterion-agent/ca.pem
Environment=ASTERION_AGENT_CA_FILE=/run/credentials/%n/ca.pem
```

在「节点 → 接入服务器 → 手动接入」填写名称并选择对应环境。生成凭证后，确认中心地址为目标节点可访问的 HTTPS 域名。将连接配置复制到 `agent.env`，用编辑器将凭证写为一行。凭证只在签发时显示，有效期 10 分钟。

```bash
sudo install -m 0600 /dev/null /etc/asterion-agent/enrollment
sudoedit /etc/asterion-agent/enrollment
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudoedit /etc/systemd/system/asterion-agent.service.d/10-enrollment.conf
```

```ini
[Service]
LoadCredential=enrollment:/etc/asterion-agent/enrollment
RuntimeDirectory=asterion-agent-bootstrap
RuntimeDirectoryMode=0700
ExecStart=
ExecStart=/bin/sh -ec '/usr/bin/install -m 0600 /run/credentials/%n/enrollment /run/asterion-agent-bootstrap/enrollment; exec /opt/asterion-agent/current/bin/asterion-agent'
Environment=ASTERION_AGENT_ENROLLMENT_FILE=/run/asterion-agent-bootstrap/enrollment
```

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-agent.service
sudo systemctl enable --now asterion-agent.service
sudo systemctl status --no-pager asterion-agent.service
sudo journalctl -u asterion-agent.service -n 50 --no-pager
```

在总览确认节点在线。身份文件由 Agent 保存在 `/var/lib/asterion-agent/identity.json`，应由同一个 Agent 独占。注册成功后移除一次性凭证配置并验证重连。

```bash
sudo rm -- /etc/systemd/system/asterion-agent.service.d/10-enrollment.conf /etc/asterion-agent/enrollment
sudo systemctl daemon-reload
sudo systemctl restart asterion-agent.service
```

## 5. 远程访问和节点凭证

浏览器通过面板域名的 `/login` 登录。远端节点使用同一 HTTPS 域名，中心主机的 Agent 使用配置的回环端口。

节点断线超过 20 秒后显示离线，凭证吊销后旧 Agent 无法继续刷新心跳。要替换凭证，在节点列表或详情打开「Agent 接入 → 手动接入」并确认换发，该操作保留节点 ID 并作废原设备凭证及旧注册凭证。先停止 Agent，移走旧身份文件，再按第 4 节的注册凭证配置和启动步骤操作。

```bash
sudo systemctl stop asterion-agent.service
sudo install -d -m 0700 /var/backups/asterion-agent
sudo mv /var/lib/asterion-agent/identity.json "/var/backups/asterion-agent/identity-$(date -u +%Y%m%dT%H%M%SZ).json"
```

移除节点会作废其凭证。节点仍有归属资产时，先在资产目录将资产迁移到其他节点，或删除相应的资产记录。迁移前先解除原节点的发现实例关联。

## 6. 备份和恢复

中心备份包含 SQLite、服务配置和 TLS 文件。自定义数据库路径时将下列路径替换为实际路径。确保备份和复制目标只对管理员可读。[SQLite 备份说明](https://www.sqlite.org/backup.html)

```bash
backup_dir="/var/backups/asterion/$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0700 "$backup_dir"
sudo test -s /var/lib/asterion/asterion.db
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db ".backup '$backup_dir/asterion.db'"
sudo chmod 0600 "$backup_dir/asterion.db"
sudo sqlite3 -readonly "$backup_dir/asterion.db" 'PRAGMA quick_check;'
sudo install -m 0600 /etc/asterion/server.env /etc/asterion/tls.crt /etc/asterion/tls.key "$backup_dir/"
if sudo test -f /etc/asterion/deploy-ca.crt; then
  sudo install -m 0600 /etc/asterion/deploy-ca.crt "$backup_dir/"
fi
sudo install -m 0600 /etc/systemd/system/asterion-server.service "$backup_dir/"
readlink -f /opt/asterion/current | sudo tee "$backup_dir/release.txt" > /dev/null
```

确认 `quick_check` 返回 `ok`。上述命令不包含公网证书。通过受控的加密备份另存 Nginx 域名站点、内部 CA 签发私钥和口令，以及 `/etc/letsencrypt/`。Certbot 的 `live/` 包含符号链接，只复制它不足以恢复续期；备份须保留链接、属主和权限。该目录包含证书私钥、ACME 账户以及续期配置使用的凭据；若 Nginx 为多个站点共用，只允许有相应权限的管理员保管，不要在已有主机上整目录覆盖。

Agent 身份由独立主机维护。要保留身份，备份 `agent.env`、私有 CA 设置（如有）和 `identity.json`。身份文件含设备凭证；也可在恢复后重新签发注册凭证。

```bash
agent_backup="/var/backups/asterion-agent/$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0700 "$agent_backup"
sudo install -m 0600 /etc/asterion-agent/agent.env /var/lib/asterion-agent/identity.json "$agent_backup/"
if sudo test -f /etc/systemd/system/asterion-agent.service.d/20-local.conf; then
  sudo install -m 0600 /etc/systemd/system/asterion-agent.service.d/20-local.conf "$agent_backup/"
fi
if sudo test -f /etc/systemd/system/asterion-agent.service.d/20-private-ca.conf; then
  sudo install -m 0600 /etc/asterion-agent/ca.pem \
    /etc/systemd/system/asterion-agent.service.d/20-private-ca.conf "$agent_backup/"
fi
```

中心恢复会覆盖工作数据库。先确认恢复时间点，备份当前库，并安装匹配的发布包和服务配置。新主机也需先完成中心初始化，以创建持久化目录和数据库文件。

```bash
backup_dir='<已验证的备份目录>'
test "$(sudo sqlite3 -batch -bail -readonly "$backup_dir/asterion.db" 'PRAGMA quick_check;')" = ok || exit 1
sudo systemctl stop asterion-server.service
sudo test -s /var/lib/asterion/asterion.db
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db ".restore '$backup_dir/asterion.db'"
sudo sqlite3 -readonly /var/lib/asterion/asterion.db 'PRAGMA quick_check;'
```

若需同时恢复程序版本，以下命令将切换到备份记录的发布路径。确认该路径在目标机存在，且备份配置适用于该主机。

```bash
backup_release=$(sudo cat "$backup_dir/release.txt")
sudo test -x "$backup_release/bin/asterion-server"
sudo ln -sfnT "$backup_release" /opt/asterion/current
sudo install -m 0600 "$backup_dir/server.env" "$backup_dir/tls.key" /etc/asterion/
sudo install -m 0644 "$backup_dir/tls.crt" /etc/asterion/tls.crt
if sudo test -f "$backup_dir/deploy-ca.crt"; then
  sudo install -m 0644 "$backup_dir/deploy-ca.crt" /etc/asterion/deploy-ca.crt
fi
sudo install -m 0644 "$backup_dir/asterion-server.service" /etc/systemd/system/asterion-server.service
sudo systemctl daemon-reload
```

快照会带回备份时的登录会话和注册状态。保持中心停止，清理所有面板会话和尚未使用的注册凭证。以下命令使用样例 SQLite 路径，实际部署需使用恢复后的数据库文件。恢复启动后重新登录面板。

```bash
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db <<'SQL'
BEGIN IMMEDIATE;
DELETE FROM panel_sessions;
UPDATE node_enrollment_tokens
SET revoked_at = CAST(strftime('%s','now') AS INTEGER)
WHERE revoked_at IS NULL AND consumed_at IS NULL;
COMMIT;
SQL
```

同样在中心停止期间核对快照中的设备凭证，重新作废快照生成后已吊销的凭证。若无法确认哪些凭证仍有效，作废该库中的全部活动设备凭证，启动后在各节点原记录上重新签发注册凭证。

```bash
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db \
  "UPDATE agent_credentials SET revoked_at = CAST(strftime('%s','now') AS INTEGER) WHERE revoked_at IS NULL;"
```

完成凭证核对后启动中心并按第 3 节验证。

```bash
sudo systemctl start asterion-server.service
```

迁移中心主机时，先恢复内部回环证书并按第 3 节验证内部健康检查；公网入口单独验收。在新主机安装 Certbot 和续期配置使用的验证插件，恢复受保护的证书和续期配置；没有可用备份时，按[公网证书流程](public-tls.md)为同一面板域名重新签发。确认正式证书存在后再启用引用它的 Nginx 站点。按[证书验收](public-tls.md#验证-nginx-实际提供的证书)核对新主机实际提供的证书，并在新主机的 root Bash 中验证定时器和续期钩子：

```bash
domain='<面板域名>'
systemctl enable --now certbot.timer
systemctl list-timers --all certbot.timer
/usr/bin/certbot renew --cert-name "$domain" --dry-run
nginx -t
```

恢复 Agent 时使用原中心地址和仍有效的设备凭证。迁移到另一台主机前先停止原 Agent，避免两台主机使用同一身份。在新主机先按第 4 节安装 Agent 程序及服务单元，然后直接恢复下列文件。使用系统信任根且目标机留有旧的私有 CA 配置时，先确认是否需要保留该配置。

```bash
agent_backup='<已验证的 Agent 备份目录>'
sudo systemctl stop asterion-agent.service
sudo install -d -m 0750 /etc/asterion-agent
sudo install -m 0600 "$agent_backup/agent.env" /etc/asterion-agent/agent.env
if sudo test -f "$agent_backup/20-local.conf"; then
  sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
  sudo install -m 0644 "$agent_backup/20-local.conf" /etc/systemd/system/asterion-agent.service.d/
fi
if sudo test -f "$agent_backup/20-private-ca.conf"; then
  sudo test -s "$agent_backup/ca.pem" || exit 1
  sudo install -m 0644 "$agent_backup/ca.pem" /etc/asterion-agent/ca.pem
  sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
  sudo install -m 0644 "$agent_backup/20-private-ca.conf" \
    /etc/systemd/system/asterion-agent.service.d/20-private-ca.conf
fi
if ! sudo test -d /var/lib/asterion-agent; then
  sudo install -d -m 0700 /var/lib/asterion-agent
fi
sudo install -m 0600 "$agent_backup/identity.json" /var/lib/asterion-agent/identity.json
sudo chown --reference=/var/lib/asterion-agent -- /var/lib/asterion-agent/identity.json
sudo systemctl daemon-reload
sudo systemctl start asterion-agent.service
```

启动后在总览确认原节点重新上线。

## 7. 升级与卸载

升级前备份数据。用新构建编号和各自的解包命令准备中心和 Agent 发布目录。保留已有环境文件、TLS 和身份，使用以下命令更新对应 unit 和 `current` 链接。先升级中心并验证，再按节点升级 Agent。同机自监控 Agent 的 `current` 链接指向中心的 `current`；中心升级后重启该 Agent，复核指标即可。中心启动时执行数据库迁移。升级后检查中心健康，打开节点的 systemd 页签核对已开启监控的 Unit 继续更新。单个服务的监控配置、状态历史与告警保存在中心 SQLite 数据库中，随数据库一起备份。[单个服务监控](monitoring-and-operations.md#单个-systemd-服务监控)

```bash
cd "$HOME"
release_id='<新构建编号>'
archive="asterion-$release_id.tar.gz"
sha256sum --check "$archive.sha256" || exit 1
release_dir="/opt/asterion/releases/$release_id"
sudo mkdir -m 0755 "$release_dir" || exit 1
sudo tar -xzf "$archive" --no-same-owner -C "$release_dir" || exit 1
sudo chmod -R u=rwX,go=rX "$release_dir"
sudo systemctl stop asterion-server.service
sudo ln -sfnT "$release_dir" /opt/asterion/current
sudo install -m 0644 "$release_dir/deploy/asterion-server.service" /etc/systemd/system/asterion-server.service
sudo systemd-analyze verify /etc/systemd/system/asterion-server.service
sudo systemctl daemon-reload
sudo systemctl start asterion-server.service
```

```bash
sudo ln -sfnT "$agent_release" /opt/asterion-agent/current
sudo install -m 0644 "$agent_release/deploy/asterion-agent.service" /etc/systemd/system/asterion-agent.service
sudo systemd-analyze verify /etc/systemd/system/asterion-agent.service
installed_helpers=()
for helper in asterion-docker-reader asterion-docker-operator asterion-account-operator asterion-terminal-helper; do
  unit="$helper.service"
  if sudo test -f "/etc/systemd/system/$unit"; then
    sudo install -m 0644 "$agent_release/deploy/$unit" "/etc/systemd/system/$unit"
    sudo systemd-analyze verify "/etc/systemd/system/$unit"
    installed_helpers+=("$unit")
  fi
done
sudo systemctl daemon-reload
if ((${#installed_helpers[@]} > 0)); then
  sudo systemctl try-restart "${installed_helpers[@]}"
fi
sudo systemctl restart asterion-agent.service
```

卸载前先备份。移除单个 Agent 时，在面板吊销该节点的凭证，之后在 Agent 主机停用该服务。

```bash
sudo systemctl disable --now asterion-agent.service
for helper in asterion-docker-reader asterion-docker-operator asterion-account-operator asterion-terminal-helper; do
  unit="$helper.service"
  if sudo test -f "/etc/systemd/system/$unit"; then
    sudo systemctl disable --now "$unit"
  fi
done
archive_dir="/var/backups/asterion-agent/uninstalled-$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0700 "$archive_dir"
sudo mv /etc/systemd/system/asterion-agent.service /etc/asterion-agent /opt/asterion-agent "$archive_dir/"
for helper in asterion-docker-reader asterion-docker-operator asterion-account-operator asterion-terminal-helper; do
  unit="/etc/systemd/system/$helper.service"
  if sudo test -f "$unit"; then
    sudo mv "$unit" "$archive_dir/"
  fi
done
if sudo test -d /etc/systemd/system/asterion-agent.service.d; then
  sudo mv /etc/systemd/system/asterion-agent.service.d "$archive_dir/"
fi
sudo systemctl daemon-reload
```

停用中心前停用对应 Nginx 站点并检查、重载 Nginx。若启用了自监控，一并按上述步骤停用本机 Agent。以下命令归档中心配置和发布文件。

```bash
sudo systemctl disable --now asterion-server.service
archive_dir="/var/backups/asterion/uninstalled-$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0700 "$archive_dir"
sudo mv /etc/systemd/system/asterion-server.service /etc/asterion /opt/asterion "$archive_dir/"
if sudo test -d /etc/systemd/system/asterion-server.service.d; then
  sudo mv /etc/systemd/system/asterion-server.service.d "$archive_dir/"
fi
sudo systemctl daemon-reload
```

以上步骤保留 `/var/lib/asterion` 与 `/var/lib/asterion-agent`。确认备份可用且确需删除持久数据时，在移除对应 unit 之前使用 `sudo systemctl clean --what=state <服务名>`。

## 8. 启用可选能力

默认安装可以查看主机指标和进程，在 systemd 节点上查询服务或开启单个 Unit 的持续监控，并使用服务资产和告警。以下服务只在需要对应能力时安装。确认授权范围后执行选中的步骤。详细的能力边界和中心环境变量见[采集与运维配置](monitoring-and-operations.md)。

### Docker 读取

中心主机可直接使用[一键初始化中心](#一键初始化中心)中的 Docker 选项。下面的手动步骤适用于独立 Agent 或自定义部署。

在 Agent 主机创建读取服务账号和访问组。先确认 Docker Engine Socket 的所有者与组。样例基于本机 Docker Socket 和 `docker` 组，其他部署用 systemd drop-in 配置实际的 Socket 和权限。本机 drop-in 在升级时保留。

```bash
getent group docker
sudo groupadd --system asterion-readers
sudo useradd --system --gid asterion-readers --no-create-home --shell /usr/sbin/nologin asterion-docker
sudo install -m 0644 /opt/asterion-agent/current/deploy/asterion-docker-reader.service /etc/systemd/system/
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudoedit /etc/systemd/system/asterion-agent.service.d/30-docker-read.conf
sudoedit /etc/asterion-agent/agent.env
```

在 drop-in 的 `[Service]` 中设置 `SupplementaryGroups=asterion-readers`。在 `agent.env` 增加 `ASTERION_DOCKER_READER_SOCKET=/run/asterion-docker-reader/requests.sock`。已经存在的账号和组可直接复用。

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-docker-reader.service
sudo systemctl enable --now asterion-docker-reader.service
sudo systemctl restart asterion-agent.service
```

在节点的 Docker 页核对 Engine ID、容器和只读状态。

### 受控任务

软件安装、在线配置编辑和容器创建使用[节点操作部署步骤](managed-operations.md)。只读节点不安装这些服务。升级或卸载已开启的实例前，先按该文的维护步骤处理。

根据实际用途配置策略。未使用的部分从策略文件移除。systemd 动作仍由主机自己的授权策略决定。Docker 操作还需要下方的独立 Socket 服务。

```bash
sudo install -m 0600 /opt/asterion-agent/current/deploy/executor.example.json /etc/asterion-agent/executor.json
sudoedit /etc/asterion-agent/executor.json
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudoedit /etc/systemd/system/asterion-agent.service.d/40-operations.conf
```

在 drop-in 的 `[Service]` 中设置 `LoadCredential=executor.json:/etc/asterion-agent/executor.json` 及 `Environment=ASTERION_EXECUTOR_CONFIG_FILE=/run/credentials/%n/executor.json`。执行下方命令后先在面板确认允许列表和可控测试任务，再处理业务服务。

```bash
sudo systemctl daemon-reload
sudo systemctl restart asterion-agent.service
sudo journalctl -u asterion-agent.service -n 50 --no-pager
```

如需 Docker 启停，准备操作策略和独立访问组。Docker 操作服务需要可调用的 Docker CLI。样例沿用上方的 `asterion-docker` 服务账号。

```bash
sudo groupadd --system asterion-operators
sudo install -m 0600 /opt/asterion-agent/current/deploy/docker-operator.example.json /etc/asterion-agent/docker-operator.json
sudoedit /etc/asterion-agent/docker-operator.json
sudo install -m 0644 /opt/asterion-agent/current/deploy/asterion-docker-operator.service /etc/systemd/system/
sudoedit /etc/systemd/system/asterion-agent.service.d/40-operations.conf
```

为 Agent 的 `[Service]` 增加 `SupplementaryGroups=asterion-operators`。Agent 策略和操作服务策略须使用相同的 Engine 与容器白名单。然后验证、启动独立服务并重启 Agent。

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-docker-operator.service
sudo systemctl enable --now asterion-docker-operator.service
sudo systemctl restart asterion-agent.service
```

### Linux 账号变更

部署该服务前，确认普通用户和附加组列表，并为授权文件备份保留私有目录。服务以 root 运行，访问仅通过专用 Unix Socket 授权组。Agent 继续使用普通服务账号。

```bash
sudo groupadd --system asterion-account-access
sudo install -d -m 0700 /var/lib/asterion-account-operator/backups
sudo install -m 0600 /opt/asterion-agent/current/deploy/account-operator.example.json /etc/asterion-agent/account-operator.json
sudoedit /etc/asterion-agent/account-operator.json
sudo install -m 0644 /opt/asterion-agent/current/deploy/asterion-account-operator.service /etc/systemd/system/
sudoedit /etc/systemd/system/asterion-agent.service.d/40-operations.conf
sudoedit /etc/asterion-agent/executor.json
```

为 Agent 的 `[Service]` 增加 `SupplementaryGroups=asterion-account-access`，并在两份策略中填写一致的用户和组。再验证并启动。

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-account-operator.service
sudo systemctl enable --now asterion-account-operator.service
sudo systemctl restart asterion-agent.service
```

### 交互终端和保留周期

需要终端时，在目标 Agent 主机安装独立终端服务。样例使用专用动态非 root 账号和单独的访问组。终端身份与 Agent 以及 Docker、Linux 账号操作组隔离。

```bash
sudo groupadd --system asterion-terminal-access
sudo install -m 0644 /opt/asterion-agent/current/deploy/asterion-terminal-helper.service /etc/systemd/system/
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudoedit /etc/systemd/system/asterion-agent.service.d/50-terminal.conf
```

在 drop-in 的 `[Service]` 中设置 `SupplementaryGroups=asterion-terminal-access`。在 Agent 的 `agent.env` 增加 `ASTERION_TERMINAL_HELPER_SOCKET=/run/asterion-terminal-helper/requests.sock`。然后校验并启动独立终端服务。

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-terminal-helper.service
sudo systemctl enable --now asterion-terminal-helper.service
```

在中心 `server.env` 和对应 Agent 的 `agent.env` 添加 `ASTERION_TERMINAL_ENABLED=1`，启用前先将包括 Nginx 在内的管理入口限制到可信私网，终端仅允许在该网络访问。采样与保留配置写入中心 `server.env`。

```bash
sudoedit /etc/asterion/server.env
sudo systemctl restart asterion-server.service
```

在目标 Agent 主机执行。

```bash
sudoedit /etc/asterion-agent/agent.env
sudo systemctl restart asterion-agent.service
```

升级已启用辅助服务的节点时，按第 4 节解包新版本的二进制和样例，再按第 7 节更新已安装的 unit 并重启正在运行的服务。实际策略、环境文件和本机 drop-in 保留在各自的私有配置目录。
