# 通过 SSH 接入服务器与重新部署 Agent

中心通过 SSH 首次安装或重新部署 Agent，启动服务后断开。日常监控和运维通过 Agent 通道进行。目标机无需安装 Python。

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
4. 在配置页下方按执行顺序查看记录。深色的「执行命令」保留完整脚本，浅色的「日志输出」显示传输进度、退出码和验证结果。部署失败时，原因单列为「错误」。使用「返回节点列表」离开页面后，任务继续执行。在同一标签页、同一登录会话中重新进入原配置页可以继续查看。中心最多保留 30 分钟日志，重启后清空。页面只展示经过筛选的输出。
5. 「部署完成，SSH 已断开」仅表示安装与服务启动成功。「已收到 Agent 心跳」才表示 Agent 已经回连。可以直接打开节点查看后续状态。

部署器仅在目标机检查身份文件是否已有节点 ID，不把文件正文传回中心。确认身份已保存才清理首次启动配置；若身份还未保存，保留临时注册材料以供 Agent 重试。这些材料位于目标机的运行时目录，令牌只可消费一次、10 分钟到期，重启主机后不会保留。

## 在已有节点部署

1. 在「节点」列表点击该节点的「Agent 接入」，或进入「管理 → 部署 Agent」。在独立页面填写 SSH 主机、端口和用户名。
2. 点击「核对主机与部署命令」，核对指纹及页面下方的命令。待接入节点在空白服务器上安装并沿用已有节点 ID，确认部署会换发一次性凭证。
3. 已经注册的节点选择「重新部署并启动」。部署器核对目标机的节点 ID、中心地址和标准安装目录。目标需要 util-linux 的 flock。程序内容变化时，校验上传文件后切换版本并重启服务，保留原版本、已有配置、凭证及辅助程序。版本相同时校验后重启。重启失败时尝试恢复原版本。
4. 日志显示 SSH 断开后，在页面确认中心收到新的心跳。需要人工换发凭证时，可在同页选择「手动接入」。

## 失败处理

- **未启用或连接失败**：检查中心设置、Python 依赖、允许的网段、DNS、SSH 端口和中心到目标的路由。主机指纹变化时，重新通过可信渠道核验，不能跳过指纹检查。
- **部署前检查失败**：检查目标机到中心的 HTTPS 健康接口、证书、CPU 架构和 sudo 权限。首次安装需要空白目录。重新部署需要目标机保存的节点身份、中心地址和安装布局与所选节点匹配。身份已吊销或丢失时使用「手动接入」恢复。
- **部分步骤已执行**：失败或超时不会自动重发，也不删除已有安装。先按日志检查已创建的专用文件，再由管理员决定保留、修复或清理。未消费的失败部署凭证会撤销。不要因 SSH 断开就重复安装。
- **已启动但没有心跳**：在目标机检查服务和回连错误，不需要保持 SSH 部署会话：

```bash
systemctl status --no-pager asterion-agent.service
sudo journalctl -u asterion-agent.service -n 50 --no-pager
```
