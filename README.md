# Asterion

Asterion 是面向个人和小型基础设施的自托管监控与运维平台。中心从 Linux VPS 和 WSL Ubuntu Agent 接收心跳与指标，提供节点查询、服务资产、告警和历史趋势。受控操作、Linux 账号变更与交互终端由部署者按需授权。

- [本地配置与启动](docs/development.md)（开发电脑）
- [一键初始化中心、部署节点与备份恢复](docs/deployment.md)（在 Ubuntu 22.04 或 24.04 中心服务器运行 `bash deploy/install-center.sh`）
- [公网证书：签发、自动续期与验收](docs/public-tls.md)（中心服务器的 Nginx）
- [通过 SSH 接入服务器与重新部署 Agent](docs/ssh-enrollment.md)
- [采集与运维配置](docs/monitoring-and-operations.md)
- [软件安装、服务配置与容器操作](docs/managed-operations.md)
- [前端设计规范](docs/design-system.md)
- [产品需求](docs/Asterion-REQUIREMENTS.md)
- [项目进度](docs/progress.md)
- [版本记录](docs/version.md)
