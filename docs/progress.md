# 项目进度

已实现 M0 至 M3 的规划能力。标准部署提供 HTTPS、单管理员会话、节点注册和只读采集。受控任务、账号变更和独立终端按节点授权启用。

| 范围 | 当前能力 | 验证 |
| --- | --- | --- |
| M0、NOD-001～NOD-006 | 注册、持久身份、凭证吊销、心跳、总览 | WSL 三个独立 Agent、TLS 握手、重连及 v1/v2 协议兼容 |
| M1、MET/PRO/DOC/SVC/AST | 主机指标、进程、Docker 和 systemd 读取、服务资产和发现来源 | 真实 WSL 采样与进程查询、隔离 Docker Engine 夹具、同节点资产绑定与跨节点拒绝 |
| M2、PRO-004、DOC-006、SVC-004、OPS-001～OPS-005 | 本地白名单、二段确认、增量输出、取消与审计 | 隔离任务运行期间收到输出、取消、结果入库及会话隔离回归 |
| M3、OPS-006、USR-001～USR-004 | 独立身份的终端、Linux 账号和公钥、告警规则、长期趋势 | WSL 独立 UID 终端与凭证访问隔离、临时公钥文件回归、实际采样触发告警 |
| NFR-008、NFR-010 | 锁定依赖许可证清单、发布和备份恢复流程 | 依赖清单生成、SQLite 备份一致性与 systemd 样例静态检查 |

WSL 中 Rust 工作区 51 项测试、严格 Clippy、前端 7 项测试、类型检查和生产构建已通过。Chromium 使用真实中心和 Agent 验证了移动尺寸下的登录、指标、进程、资产、任务输出、Linux 账号、独立终端、告警、设置和注销。回归验证也覆盖了任务确认和取消收到空的 HTTP 202 响应后可以正常继续操作，同一 SQLite 文件只能由一个中心实例使用，以及恢复时的会话和注册凭证清理语句。

管理面板采用统一的登录界面、响应式导航和共享组件。已从 Windows 验证页面和 API 可达，并在 WSL 的 Chromium 中验证桌面、平板和手机尺寸的导航、键盘焦点、会话校验失败与恢复及登录注销。

VPS 生产部署以及真实 Docker、systemd 与账号变更的现场验收仍需在目标环境进行。Windows Host Agent 和 LocalOpsGateway 适配为可选扩展。

[工程架构](architecture.md) · [本地开发](development.md) · [远程部署](deployment.md) · [采集与运维配置](monitoring-and-operations.md) · [前端设计规范](design-system.md) · [产品需求](Asterion-REQUIREMENTS.md) · [版本记录](version.md)
