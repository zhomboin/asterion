# Ubuntu 远程部署

中心和 Agent 使用 systemd 运行。每个持久 SQLite 文件只由一个中心进程使用。中心在回环地址或私有网络地址上提供 HTTPS。浏览器和 Agent 使用证书中的域名或 IP 访问，Agent 只发起出站连接。以下构建命令在 WSL Ubuntu 的 Bash 中执行，安装命令在各自目标 Ubuntu 的 Bash 中执行。所有带尖括号的值都应替换为实际配置，保存在 Git 之外。

## 1. 构建和上传发布包

按[本地开发](development.md#1-准备工具)安装构建工具。在项目根目录构建。构建环境须与目标机的 CPU 架构和系统运行库兼容。为每个发布包选择唯一的构建编号。

```bash
release_id='<构建编号>'
target='<SSH用户>@<SSH主机>'
stage=".local/asterion-$release_id"
npm --prefix web ci
npm --prefix web run build
cargo +stable build --locked --release --workspace
install -Dm755 target/release/asterion-server "$stage/bin/asterion-server"
install -Dm755 target/release/asterion-agent "$stage/bin/asterion-agent"
for helper in asterion-docker-reader asterion-docker-operator asterion-account-operator asterion-terminal-helper; do
  install -Dm755 "target/release/$helper" "$stage/bin/$helper"
done
install -d "$stage/web" "$stage/deploy"
cp -a web/dist/. "$stage/web/"
install -m 0644 deploy/*.service deploy/*.env.example deploy/*.example.json "$stage/deploy/"
install -d "$stage/deploy/managed"
install -m 0644 deploy/managed/{service,common,containers,console,host}.py "$stage/deploy/managed/"
tar -C "$stage" -czf ".local/asterion-$release_id.tar.gz" bin web deploy
(
  cd .local
  sha256sum "asterion-$release_id.tar.gz" > "asterion-$release_id.tar.gz.sha256"
)
scp ".local/asterion-$release_id.tar.gz" ".local/asterion-$release_id.tar.gz.sha256" "$target:"
ssh "$target"
```

使用 SSH 一键接入时，只向中心上传发布包。选择手动接入时，向每台 Agent 主机上传同一发布包和校验文件。

## 2. 安装中心和准备 TLS

在中心主机安装验证和备份工具，并验证上传文件。使用证书链 PEM 文件和对应的未加密私钥 PEM 文件。证书须包含客户端访问时使用的域名或 IP。

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates curl openssl sqlite3
cd "$HOME"
release_id='<同一个构建编号>'
archive="asterion-$release_id.tar.gz"
sha256sum --check "$archive.sha256"
release_dir="/opt/asterion/releases/$release_id"
sudo install -d -m 0755 "$release_dir"
sudo tar -xzf "$archive" --no-same-owner -C "$release_dir"
sudo install -d -m 0750 /etc/asterion
sudo install -m 0600 "$release_dir/deploy/server.env.example" /etc/asterion/server.env
sudo install -m 0644 '<证书链 PEM 文件>' /etc/asterion/tls.crt
sudo install -m 0600 '<私钥 PEM 文件>' /etc/asterion/tls.key
sudo install -m 0644 "$release_dir/deploy/asterion-server.service" /etc/systemd/system/asterion-server.service
sudo ln -sfnT "$release_dir" /opt/asterion/current
```

多个节点连接时，在 `/etc/asterion/server.env` 中将 `ASTERION_LISTEN_ADDR` 改为 Tailscale、WireGuard 或其他可信私网的本机地址和端口。管理入口按私网访问范围限制。默认回环监听适用于中心同机 Agent 或通过隧道访问的面板。

```bash
sudoedit /etc/asterion/server.env
```

systemd 为服务分配专用身份，并在 `/var/lib/asterion` 保存 SQLite。TLS 证书和私钥通过 `LoadCredential` 读取。更新同一签发机构的证书时，替换 PEM 后重启中心。更换私有 CA 时，先更新浏览器和 Agent 的信任设置。[systemd 凭据说明](https://systemd.io/CREDENTIALS/)

## 3. 初始化管理员并启动中心

首次使用新的 SQLite 数据库时，创建一个仅 root 可读的口令文件。参考[本地开发](development.md#3-启动中心)的用户名与口令限制。

```bash
sudo install -m 0600 /dev/null /etc/asterion/admin-password
sudoedit /etc/asterion/admin-password
sudo install -d -m 0755 /etc/systemd/system/asterion-server.service.d
sudoedit /etc/systemd/system/asterion-server.service.d/10-bootstrap.conf
```

将以下内容写入 `10-bootstrap.conf`，替换管理员用户名。systemd 在服务自己的临时目录生成权限为 0600 的副本。

```ini
[Service]
LoadCredential=admin-password:/etc/asterion/admin-password
RuntimeDirectory=asterion-bootstrap
RuntimeDirectoryMode=0700
ExecStartPre=/usr/bin/install -m 0600 %d/admin-password /run/asterion-bootstrap/admin-password
Environment="ASTERION_ADMIN_USERNAME=<管理员用户名>"
Environment=ASTERION_ADMIN_PASSWORD_FILE=/run/asterion-bootstrap/admin-password
```

```bash
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/asterion-server.service
sudo systemctl enable --now asterion-server.service
sudo systemctl status --no-pager asterion-server.service
sudo journalctl -u asterion-server.service -n 50 --no-pager
```

从有访问权限的主机验证 HTTPS。私有 CA 通过 `--cacert` 指定。证书已被系统信任时可省略该参数。

```bash
center_origin='https://<证书中的域名或IP>:<中心端口>'
curl --fail --silent --show-error --cacert '<CA PEM 文件>' "$center_origin/healthz"
curl --fail --silent --show-error --cacert '<CA PEM 文件>' -o /dev/null "$center_origin/login"
```

浏览器使用相同地址和可信 CA 打开 `/login`。确认登录成功后，删除临时初始化配置及明文口令文件，重启中心。

```bash
sudo rm -- /etc/systemd/system/asterion-server.service.d/10-bootstrap.conf /etc/asterion/admin-password
sudo systemctl daemon-reload
sudo systemctl restart asterion-server.service
```

使用原管理员口令再次登录。面板注销会撤销当前会话。会话最长有效 12 小时。

## 4. 安装和注册 Agent

首次安装优先使用[面板 SSH 接入](ssh-enrollment.md)。只需在中心启用一次；部署完成即断开 SSH。以下是手动接入步骤。

在每台目标机验证上传文件后，准备 Agent 的安装位置。WSL Ubuntu 使用同一 Agent。更新部署时，中心、Agent 和 Docker 只读采集程序应使用同一构建产物，以支持按需查询已纳管的进程和容器。运行 systemd 样例需要该发行版已启用 systemd，也可使用[本地开发](development.md#5-注册一个本地-agent)中的命令直接启动。

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

将 `ASTERION_SERVER_URL` 改为 Agent 可访问的中心 HTTPS 源地址。如果使用私有 CA，将私有 CA 的公开证书拷贝到下列位置，并建立 CA 配置。使用系统已信任的 CA 时跳过此步。

```bash
sudo install -d -m 0755 /etc/systemd/system/asterion-agent.service.d
sudo install -m 0644 '<私有 CA 的 PEM 文件>' /etc/asterion-agent/ca.pem
sudoedit /etc/systemd/system/asterion-agent.service.d/20-private-ca.conf
```

```ini
[Service]
LoadCredential=ca.pem:/etc/asterion-agent/ca.pem
Environment=ASTERION_AGENT_CA_FILE=%d/ca.pem
```

在「节点 → 接入服务器 → 手动接入」填写名称并选择 Linux 或 WSL。生成凭证后，确认弹窗中的中心地址可由目标服务器访问。将连接配置复制到 `agent.env`，用编辑器将凭证写为一行。凭证只在签发时显示，有效期 10 分钟。

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
ExecStartPre=/usr/bin/install -m 0600 %d/enrollment /run/asterion-agent-bootstrap/enrollment
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

仅面板通过 SSH 隧道访问时，在 WSL 开新终端。将中心回环端口转到本机回环。以下为使用默认中心端口的样例。

```bash
target='<SSH用户>@<SSH主机>'
ssh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:8080:127.0.0.1:8080 "$target"
```

证书包含 `127.0.0.1` 时可以打开 `https://127.0.0.1:8080/login`。证书仅包含域名时，将该域名在客户端解析到本机回环，使用该域名打开同一转发端口。完成后按 `Ctrl+C` 关闭隧道。

节点断线超过 20 秒后显示离线，凭证吊销后旧 Agent 无法继续刷新心跳。要替换凭证，在节点列表或详情打开「Agent 接入」并确认换发，该操作保留节点 ID 并作废原设备凭证及旧注册凭证。先停止 Agent，移走旧身份文件，再按第 4 节的注册凭证配置和启动步骤操作。

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
sudo install -m 0600 /etc/systemd/system/asterion-server.service "$backup_dir/"
readlink -f /opt/asterion/current | sudo tee "$backup_dir/release.txt" > /dev/null
```

确认返回 `ok`。发布包和 TLS 私钥按受控备份策略保存。Agent 身份由独立主机维护。需要保留身份时，备份 `agent.env`、私有 CA 设置（如有）和 `identity.json`。身份文件含设备凭证。也可在恢复后重新签发注册凭证。

```bash
agent_backup="/var/backups/asterion-agent/$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0700 "$agent_backup"
sudo install -m 0600 /etc/asterion-agent/agent.env /var/lib/asterion-agent/identity.json "$agent_backup/"
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
sudo install -m 0644 "$backup_dir/asterion-server.service" /etc/systemd/system/asterion-server.service
sudo systemctl daemon-reload
```

快照会带回备份时的登录会话和注册状态。保持中心停止，清理所有面板会话和尚未使用的注册凭证。以下命令使用样例 SQLite 路径，实际部署需使用恢复后的数据库文件。恢复启动后重新登录面板。

```bash
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db <<'SQL'
BEGIN IMMEDIATE;
DELETE FROM panel_sessions;
UPDATE node_enrollment_tokens
SET revoked_at = unixepoch()
WHERE revoked_at IS NULL AND consumed_at IS NULL;
COMMIT;
SQL
```

同样在中心停止期间核对快照中的设备凭证，重新作废快照生成后已吊销的凭证。若无法确认哪些凭证仍有效，作废该库中的全部活动设备凭证，启动后在各节点原记录上重新签发注册凭证。

```bash
sudo sqlite3 -batch -bail /var/lib/asterion/asterion.db \
  'UPDATE agent_credentials SET revoked_at = unixepoch() WHERE revoked_at IS NULL;'
```

完成凭证核对后启动中心并按第 3 节验证。

```bash
sudo systemctl start asterion-server.service
```

恢复 Agent 时使用原中心地址和仍有效的设备凭证。迁移到另一台主机前先停止原 Agent，避免两台主机使用同一身份。在新主机先按第 4 节安装 Agent 程序及服务单元，然后直接恢复下列文件。使用系统信任根且目标机留有旧的私有 CA 配置时，先确认是否需要保留该配置。

```bash
agent_backup='<已验证的 Agent 备份目录>'
sudo systemctl stop asterion-agent.service
sudo install -d -m 0750 /etc/asterion-agent
sudo install -m 0600 "$agent_backup/agent.env" /etc/asterion-agent/agent.env
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

升级前备份数据。用新构建编号和各自的解包命令准备中心和 Agent 发布目录。保留已有环境文件、TLS 和身份，使用以下命令更新对应 unit 和 `current` 链接。先升级中心并验证，再按节点升级 Agent。中心启动时执行数据库迁移。升级后检查中心健康，打开节点的 systemd 页签核对已开启监控的 Unit 继续更新。单个服务的监控配置、状态历史与告警保存在中心 SQLite 数据库中，随数据库一起备份。[单个服务监控](monitoring-and-operations.md#单个-systemd-服务监控)

```bash
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

停用中心前关闭所有正在使用的 SSH 隧道。以下命令归档中心配置和发布文件。

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

在 drop-in 的 `[Service]` 中设置 `LoadCredential=executor.json:/etc/asterion-agent/executor.json` 及 `Environment=ASTERION_EXECUTOR_CONFIG_FILE=%d/executor.json`。执行下方命令后先在面板确认允许列表和可控测试任务，再处理业务服务。

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

在中心 `server.env` 和对应 Agent 的 `agent.env` 添加 `ASTERION_TERMINAL_ENABLED=1`，保持监听和管理入口在回环或私有网络。采样与保留配置写入中心 `server.env`。

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
