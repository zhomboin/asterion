# SSH 连接与监控

从本机启动时，接入页可直接输入 SSH 地址或 OpenSSH 别名。私钥、主机记录和已解锁的 SSH agent 均来自运行面板的 Linux 用户。基础监控及终端的使用步骤见[本机使用](local.md)。以下为指定连接策略和常驻中心的配置方式。

中心从 `ASTERION_SSH_CONFIG_FILE` 指定的 UTF-8 JSON 文件读取连接名。每个连接至少包含目标地址和 SSH 用户，端口默认为 22。面板只列出连接名，私钥由中心的 OpenSSH 客户端读取。以下是单节点的最小本地配置。

```json
{
  "app": {
    "host": "<目标地址>",
    "user": "<已有SSH用户>",
    "port": 22
  }
}
```

默认从配置所在目录读取 `ssh.key` 和 `known_hosts`。各连接也可设置 `identity_file` 和 `known_hosts_file`，路径相对于配置文件或为绝对路径。systemd 部署时，额外私钥可通过独立的 `LoadCredential` 条目提供，并在连接中引用其凭据名。中心严格核对目标主机密钥，使用非交互公钥认证。以下命令可在中心以能读取相应凭据的身份验证。

```bash
ssh -F /dev/null -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile='<已核实的主机记录文件>' -i '<私钥>' -p '<端口>' -l '<SSH用户>' -- '<目标地址>' 'python3 --version'
```

节点需要 Linux、可读的 `/proc` 和 Python 3.9 或以上版本。每次采集由 SSH 会话启动一次短时读取。默认每 15 秒采集 CPU、内存、交换区、磁盘和网卡数据，保留原始数据 7 天、趋势 90 天。中心配置支持 `ASTERION_SAMPLE_INTERVAL_SECS`、`ASTERION_METRIC_RETENTION_DAYS` 和 `ASTERION_TREND_RETENTION_DAYS`。连接校验失败或采集超时会显示离线，节点详情保留原因和上次成功时间。

进程、Docker 和 systemd 页按需查询。Docker 读取使用远端 Unix Socket，操作使用远端 Docker CLI；systemd 使用远端 systemctl。目标账号已有权限决定可读、可执行的范围。持续服务检查、告警和历史数据由中心保存。

[专用操作配置](managed-operations.md) · [本地开发](development.md) · [部署](deployment.md)
