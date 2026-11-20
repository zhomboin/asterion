# 项目进度

| 功能 | 当前状态 |
| --- | --- |
| 节点接入 | 中心使用 OpenSSH 和已有登录密钥连接 Linux、WSL；已有节点可绑定 SSH 配置并保留 ID。 |
| 监控与资产 | 提供指标、趋势、告警、进程、Docker、systemd 及按节点关联的服务资产。 |
| 运维 | 软件安装、服务配置、容器、账号、公钥、预定义命令及交互终端均使用 SSH。操作保留会话校验、确认和审计。 |
| 部署 | 中心发布包包含一个服务二进制、面板和安装向导。目标环境的 Docker、systemd、包管理器与账号写权限需用实际 SSH 账号验收。 |

[本地开发](development.md) · [服务器部署](deployment.md) · [专用面板](managed-operations.md) · [版本记录](version.md)
