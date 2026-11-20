# 服务器部署

中心运行在启用 systemd 的 Ubuntu 22.04 或 24.04。浏览器通过 HTTPS 访问中心，中心通过出站 SSH 连接节点。WSL 节点需要将其 Linux SSH 入口开放给中心。

## 一键初始化中心

1. 在 DNS 中把管理域名指向中心。开放域名验证所需的 TCP 80 和面板所需的 TCP 443。准备已有的发布归档及配套 SHA-256 文件，或在与中心兼容的 Ubuntu 主机上从源码构建。

   ```bash
   bash deploy/build-release.sh
   ```

2. 在中心交互终端从项目或解包后的发布目录运行安装向导。需要 sudo 权限。手动传入可信归档可以跳过构建。

   ```bash
   bash deploy/install-center.sh '<发布包的绝对路径>'
   ```

3. 依次填写域名、回环监听端口、面板管理员名称前缀、面板口令及证书向导所需信息。向导安装中心服务、Nginx 并配置 HTTPS。面板账号为所填前缀加 `-panel`。检查服务、域名健康接口和续期定时器。

   ```bash
   systemctl is-active asterion-server nginx certbot.timer
   curl --fail 'https://<面板域名>/healthz'
   sudo journalctl -u asterion-server -n 50 --no-pager
   ```

## 配置中心出站 SSH

1. 在目标服务器已有的可信控制台核实 SSH 主机公钥指纹。在中心准备与该指纹匹配的 OpenSSH `known_hosts` 文件、目标账号已授权的私钥，以及[连接配置](monitoring-and-operations.md)所述的 JSON 文件。非默认端口的主机记录使用 `[主机]:端口`。

2. 将三个文件装入中心私有配置目录，并将随发布提供的 SSH 配置交给 systemd。systemd 在启动时向中心进程提供只读凭据副本。

   ```bash
   sudo install -d -m 0700 /etc/asterion/ssh
   sudo install -m 0600 '<私钥路径>' /etc/asterion/ssh/identity
   sudo install -m 0600 '<已核实的主机记录文件>' /etc/asterion/ssh/known_hosts
   sudo install -m 0600 '<连接配置文件>' /etc/asterion/ssh/profiles.json
   sudo install -d -m 0755 /etc/systemd/system/asterion-server.service.d
   sudo install -m 0644 /opt/asterion/current/deploy/center/ssh.conf.in /etc/systemd/system/asterion-server.service.d/30-ssh.conf
   sudo systemctl daemon-reload
   sudo systemctl restart asterion-server
   ```

3. 登录面板，在「节点 → 接入服务器」选择 SSH 连接。已有节点在「管理」中补选连接即可沿用节点 ID、历史采样、资产和任务记录。中心自身也可以用同样方式接入。修改 systemd 的凭据源后重启中心以加载新副本。

4. 检查节点显示的系统、架构和采样时间。进入预定义命令页执行只读命令，核对任务记录、退出码与输出。其他操作按[专用面板](managed-operations.md)选择需要管理的资源。

## 已部署中心升级

在维护窗口备份数据库、配置和当前发布。确认没有执行中的变更，再停止中心。替换下列占位符，选用新的发布目录。先验证 SHA-256，再解包自己构建或核实来源的归档。

```bash
archive='<新归档绝对路径>'
release='/opt/asterion/releases/<新构建编号>'
cd "$(dirname "$archive")"
sha256sum -c "$(basename "$archive").sha256"
sudo systemctl stop asterion-server
sudo install -d -m 0755 "$release"
sudo tar --no-same-owner -xzf "$archive" -C "$release"
sudo ln -sfnT "$release" /opt/asterion/current
sudo install -m 0644 "$release/deploy/asterion-server.service" /etc/systemd/system/asterion-server.service
sudo systemctl daemon-reload
sudo systemctl start asterion-server
curl --fail 'https://<面板域名>/healthz'
```

在面板逐个为原节点绑定 SSH 配置。确认采样和实际需要的操作通过后，可在相应服务器执行 `systemctl list-unit-files 'asterion*'` 核对以前安装的节点服务，再对确认退役的服务逐个执行 `sudo systemctl disable --now '<节点服务名>'`。保留中心的 `asterion-server.service`。

## 备份和恢复

将下列备份目录保存在受限位置。SSH 私钥、内部 CA 及面板凭据随配置备份一并保护。

```bash
backup='<新的私有备份目录>'
sudo install -d -m 0700 "$backup"
sudo sqlite3 /var/lib/asterion/asterion.db ".backup '$backup/asterion.db'"
sudo sqlite3 "$backup/asterion.db" 'PRAGMA integrity_check;'
sudo cp -a /etc/asterion "$backup/"
sudo cp -a /etc/systemd/system/asterion-server.service.d "$backup/"
readlink -f /opt/asterion/current
```

恢复时先停止中心并保留现场数据库及 WAL、SHM 文件，在同一状态目录恢复一致快照并保持数据库的服务属主和私有权限。切回对应发布、恢复配置，启动后校验登录和 SSH 采样。迁移域名或主机时按[公网证书](public-tls.md)核对证书及续期。
