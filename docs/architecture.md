# 工程架构

「接入服务器」可由中心临时发起 SSH，核对目标后安装 Agent。部署器只执行随中心发布的固定步骤，退出后不保留 SSH 登录凭据。节点状态和后续操作仍由 Agent 通道负责。[接入配置](ssh-enrollment.md)

| 组件 | 职责 |
| --- | --- |
| `crates/asterion-server` | HTTPS、面板鉴权、节点身份、监控与资产 API、运维任务、终端会话和告警 |
| `crates/asterion-agent` | Linux 和 WSL 节点出站连接、设备身份、心跳、定期采样、按需读取和受控请求分发 |
| `crates/asterion-protocol` | Agent 与中心共享的消息、版本、运维动作及参数校验 |
| `crates/asterion-collector` | Linux 指标、进程、systemd 和 Docker 采集；提供独立 Docker 读取程序 |
| `crates/asterion-executor` | Agent 本地运维策略、进程身份核验、预定义命令和独立的 Docker 操作及 Linux 账号辅助服务 |
| `migrations` | SQLite 节点、会话、凭证、采样、资产、运维任务、纳管对象、服务监控和告警 |
| `web` | React 登录、总览、节点详情、服务资产、运维记录、告警和运行设置界面 |
| `deploy` | Ubuntu systemd 单元与配置样例 |

## 连接与读取

中心在提供 HTTPS 前加载证书、绑定监听地址、应用迁移并检查面板管理员。Linux 中心为持久 SQLite 文件持有独立锁，同一个数据库由一个中心进程使用。浏览器使用受保护的同源会话。Vite 在本地开发时通过 HTTPS 代理中心 API 和 Agent WebSocket 接入。健康检查验证中心与 SQLite 的连接。

Agent 使用一次性注册凭证取得节点身份，将设备凭证保存在节点本地。之后由 Agent 主动通过 TLS WebSocket 连接中心。中心接收有效握手与心跳后更新时间戳。凭证吊销和连接换代约束同一节点上的心跳、采样及任务响应。协议 1 的 Agent 仍可注册、重连和上报心跳。协议 2 提供采样、按需读请求和经授权下发的任务。

主机指标在 Agent 采样。中心分别记录 Agent 的采样时间与中心接收时间，后者用于在线判断、趋势窗口与保留清理。进程、systemd、Docker 和账号详情可按需查询。中心持久化管理员纳管的进程名称及容器身份。实时查询按当前清单页的目标过滤，发现清单由用户打开。管理员选定的 systemd Unit 由中心分批安排只读检查。中心保存最后一次检查结果和状态变化历史，重启后继续检查已启用的 Unit。发现信息按节点、类型、运行环境和资源身份写入观察表。资产编辑与发现写入独立存储，管理员可在同一节点上明确绑定。

## 运维与告警

中心将运维动作和目标固化为待确认任务。一次性确认凭证绑定面板会话，敏感操作需要重新验证管理员口令。任务与审计写入 SQLite，再经目标 Agent 下发。受限的输出快照在执行中同步，失败或断线时保留已有输出。Agent 按本机策略复核资源和参数。Docker 和 Linux 账号的高权限步骤使用独立的本机 Unix Socket 辅助服务。未能确认执行结果的任务保留单独状态，中心重启时标记中断，不自动执行。

启用终端需要中心与 Agent 都明确授权。浏览器以当前面板会话访问经口令复核建立的终端。Agent 通过本机 Unix Socket 调用独立非 root 服务，并校验对端 UID 与受保护的 Socket 目录归属。终端进程组受空闲时间、会话时长和 Linux seccomp 限制。审计记录开启和关闭，输入与输出不写入中心数据库。

告警规则按节点配置离线或 CPU、内存、磁盘阈值。中心定期评估记录，保存发生与恢复时间。单个 Unit 的告警由已核实的停止和活动状态驱动。查询结果不确定时保持原告警，停用监控时以停用为结束原因。总览和告警页共同展示两类事件。长期趋势使用独立的 5 分钟汇总，保留周期与原始采样分别设置。缺测不会填 0。可选 Windows Host Agent 与外部 LocalOpsGateway 适配不影响中心和 Linux Agent 运行。

## 技术决策

- [ADR-001 Agent 连接协议](adr/001-agent-protocol.md)
- [ADR-002 TLS 与设备凭证](adr/002-tls-and-credentials.md)
- [ADR-003 SQLite 持久化](adr/003-sqlite.md)
- [ADR-004 Docker 读取边界](adr/004-docker.md)
- [ADR-005 systemd 读取方式](adr/005-systemd.md)
- [ADR-006 节点与运行环境身份](adr/006-identity.md)
- [ADR-007 受控运维边界](adr/007-operations.md)

## 节点写操作入口

节点通过原有 Agent 连接执行受管变更。中心验证登录会话和确认凭证，节点上的独立服务从本机策略解析资源标识。安装、写配置和 Docker 权限不交给 Agent。详见[节点操作](managed-operations.md)。
