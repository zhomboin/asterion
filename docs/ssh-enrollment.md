# 首次通过 SSH 接入服务器

SSH 只用于首次安装：中心连接目标，上传同一发布包的 Agent，启动服务后断开。日常监控、告警、容器管理、配置编辑和终端仍使用原有 Agent 通道，不保存 SSH 连接。目标机不需安装 Python。

## 一次性配置中心

1. 按[远程部署](deployment.md)构建、校验并安装发布包。SSH 目标需运行 Linux 和 systemd，具备标准 coreutils、curl；CPU 架构与 Agent 二进制须一致。目标 SSH 用户须为 root 或已获相应非交互 sudo 权限。面板不会设置 sudoers、修改 sshd 或安装可选运维服务。
2. 在中心执行：

```bash
sudo apt-get update
sudo apt-get install -y python3-paramiko
/usr/bin/python3 -I -c 'import paramiko; print(paramiko.__version__)'
test -r /opt/asterion/current/bin/asterion-agent
sudoedit /etc/asterion/server.env
```

3. 在中心环境文件中按[环境样例](../deploy/server.env.example)填写以下设置。不要提交实际地址、账号或私钥。本地开发可将变量保存在项目忽略的 `.local/` 下，启动中心前导入。

| 设置 | 要求 |
| --- | --- |
| `ASTERION_SSH_DEPLOY_AGENT` | 经校验、不可被面板用户替换的 Linux Agent 二进制绝对路径。实际上传的文件在目标机再次校验 SHA-256。 |
| `ASTERION_SSH_DEPLOY_CENTER_URL` | 目标机可访问且证书匹配的中心 HTTPS 源地址。不是浏览器的本地代理地址。 |
| `ASTERION_SSH_DEPLOY_NETWORKS` | 可部署目标的 CIDR 清单，逗号分隔。仅授权必要主机或管理网段。域名的所有解析结果都必须在清单内。 |
| `ASTERION_SSH_DEPLOY_CA_FILE` | 可选，目标机需要信任的公开 CA 证书；不得填写私钥。使用公共受信 CA 时不设置。 |

4. 在中心重启并检查健康：

```bash
sudo systemctl restart asterion-server.service
sudo systemctl status --no-pager asterion-server.service
```

## 接入一台新服务器

1. 在「节点 → 接入服务器」填写节点名、环境、SSH 主机、端口和用户名。选择「核对主机与部署命令」。此时只握手读取指纹，不提交 SSH 登录凭据。
2. 用受信任的主机控制台或管理员提供的指纹核对。可在目标机的可信控制台列出 SSH 主机公钥的 SHA-256 指纹：

```bash
for key in /etc/ssh/ssh_host_*_key.pub; do ssh-keygen -lf "$key" -E sha256; done
```

3. 展开命令，核对目的地址、服务和安装路径。勾选确认，输入密码或粘贴私钥（加密私钥另填解密口令），点击「一键部署并启动」。中心重新核对目标 IP、指纹和发布物；不接受自定义命令。确认 2 分钟后失效。不会自动读取本机 SSH 私钥。
4. 查看逐步更新的命令、退出码和验证结果。远端未经筛选的输出不回显，以免泄露凭据。SSH 认证材料和一次性注册令牌不进入命令行、页面日志或数据库。关闭弹窗不会重新部署；同一标签页、同一登录会话可重新打开查看，日志最多保留 30 分钟，中心重启后不可恢复。
5. 「部署完成，SSH 已断开」仅表示安装与服务启动成功。「已收到 Agent 心跳」才表示 Agent 已经回连。可以直接打开节点查看后续状态。

部署器仅在目标机检查身份文件是否已有节点 ID，不把文件正文传回中心。确认身份已保存才清理首次启动配置；若身份还未保存，保留临时注册材料以供 Agent 重试。这些材料位于目标机的运行时目录，令牌只可消费一次、10 分钟到期，重启主机后不会保留。

## 失败处理

- **未启用或连接失败**：检查中心设置、Python 依赖、允许的网段、DNS、SSH 端口和中心到目标的路由。主机指纹变化时，重新通过可信渠道核验，不能跳过指纹检查。
- **部署前检查失败**：检查目标机到中心的 HTTPS 健康接口、证书、CPU 架构和 sudo 权限。检测到 Agent 配置、安装目录、Unit 或已有身份时会拒绝覆盖。不提供一键重装；已有节点走原有维护或手动换发流程。
- **部分步骤已执行**：失败或超时不会自动重发，也不删除已有安装。先按日志检查已创建的专用文件，再由管理员决定保留、修复或清理。未消费的失败部署凭证会撤销。不要因 SSH 断开就重复安装。
- **已启动但没有心跳**：在目标机检查服务和回连错误，不需要保持 SSH 部署会话：

```bash
systemctl status --no-pager asterion-agent.service
sudo journalctl -u asterion-agent.service -n 50 --no-pager
```
