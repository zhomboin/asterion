# SSH 连接与监控

## 直接连接服务器

面板使用启动它的 Linux 用户的 OpenSSH 客户端。先核对服务器主机指纹，然后在「节点 → 接入服务器」填写别名、主机地址、端口和用户名。可填写远端账号密码、指定本机私钥，或沿用 OpenSSH 配置。监控、专用操作和终端共用节点的认证设置。[启动和接入步骤](local.md)

## 连接配置和专用操作

要为已经接入的节点设置可操作的软件包、容器或命令，在本机数据目录的 `profiles.json` 里使用节点详情显示的完整连接标识（例如 `ssh://app`）作为键。条目包含一个 `policy` 对象。保存后面板自动加载，继续使用同一 SSH 身份和节点历史。[专用面板配置](managed-operations.md)

也可以在该文件中定义命名连接，指定 `host`、`user` 和可选的 `port`（默认 22），然后在接入页的下拉框中选择。命名连接默认从配置文件所在目录读取 `ssh.key` 和 `known_hosts`，可通过 `identity_file` 和 `known_hosts_file` 指定路径。样例中的占位符应替换为自己核实的值，实际配置留在本机数据目录。[命名连接样例](../deploy/ssh.example.json)

## 采样和权限

直连 Linux 节点需要可读的 `/proc` 和常用系统命令。基础采样包括 CPU、内存、根分区和网卡累计流量。完整采样、命名连接的监控和专用资源查询需要 Python 3.9 或以上版本。Docker、systemd 和进程信息按需读取，实际范围由所选 SSH 账号的权限决定。

面板默认每 15 秒采样，原始数据保留 7 天，趋势保留 90 天。可在启动时分别设置 `ASTERION_SAMPLE_INTERVAL_SECS`、`ASTERION_METRIC_RETENTION_DAYS` 和 `ASTERION_TREND_RETENTION_DAYS`。采样、告警和持续服务检查随本机程序运行，历史记录存放在本机数据目录。连接失败时可在节点详情查看原因和上次成功时间。
