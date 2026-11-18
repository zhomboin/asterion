# Asterion — 产品需求与 MVP 实施规范

> 当前实现与验收证据在[项目进度](progress.md)中维护。
>
> 项目简介：面向小规模基础设施的轻量自托管监控与运维平台。
>
> 适用范围：个人及小规模基础设施（首期 2 台 Linux VPS + 1 个 Windows 11 WSL Ubuntu 实例）

## 1. 背景与目标

用户目前管理两台 Linux 云服务器和 Windows 11 本地 WSL Ubuntu。服务器间切换不方便，难以快速确认服务运行在哪台机器、部署目录在哪里、由 Docker 还是 systemd 管理，以及如何定位日志和配置文件。

**Asterion 的目标**：通过一个自托管 Web 控制台，统一管理多个节点，提供主机指标、进程、Docker、systemd 状态、服务资产目录和受控运维操作；保持轻量，不接管现有服务部署流程。

### 1.1 核心价值

1. **一个入口**：统一查看所有 VPS、WSL 节点在线状况与资源负载。
2. **知道在哪里**：跨节点搜索服务，核实其进程/容器、启动方式、路径、配置文件和仓库。
3. **看到发生什么**：CPU、内存、磁盘、网络、进程、容器和服务状态可追踪。
4. **可安全操作**：按权限对指定目标执行受控操作，保留审计与失败信息。
5. **不侵入现有部署**：保留现有 Nginx、Docker Compose、systemd、SSH、脚本目录与配置。

### 1.2 非目标

首期**不开发**：应用商店、网站托管控制台、域名/证书申请、数据库可视化管理、CI/CD 系统、自动修复或自动部署、Kubernetes、Windows 完整系统管理、跨租户 SaaS、Prometheus/Grafana 替代品、任意文件编辑器、基于 AI 的自动运维。

初期支持**单管理员账号**，但核心授权结构必须预留后续角色扩展；不在首期实现复杂多租户 RBAC。

## 2. 产品边界与版本

| 版本 | 定位 | 完成范围 |
|---|---|---|
| M0（基础骨架） | 可部署、可注册 | Rust Workspace、React Web、SQLite 迁移、鉴权、Agent 连接、节点注册 |
| M1（只读 MVP） | 首个可用版本 | 多节点总览、指标、进程、Docker/systemd 只读发现、服务资产目录、跨节点检索 |
| M2（受控操作） | 安全运维 | Docker 启停/重启、systemd 指定服务操作、预定义命令、审批式确认、审计 |
| M3（扩展管理） | 完善能力 | Linux 用户/公钥管理、交互终端、监控告警、长期趋势、可选 Windows Host Agent |

M1 验收只读监控和资产管理。M2 和 M3 的远端变更需要按能力单独授权，终端按节点明确启用。

## 3. 角色与部署环境

### 3.1 管理对象

- **Node**：被管理的一个独立操作系统实例。Linux VPS 和每个 WSL 发行版实例都视为 Node。
- **Host**：物理/云主机或 Windows 宿主机。一个 Host 可包含一个或多个 Node（例如 Windows → WSL Ubuntu）。
- **Runtime**：Node 内的运行环境，如本机 systemd、Docker Engine；运行环境可被多个客户端访问，必须识别其真实归属。
- **Service Asset**：业务/系统服务的统一记录；可关联 Docker 容器、Compose Project、systemd Unit、普通进程、Nginx 代理配置等。

### 3.2 初期环境

- 两台 Ubuntu Linux VPS；版本及资源规格以 Agent 实际上报为准，不硬编码。
- Windows 11 本机的 WSL2 Ubuntu；Windows 主机管理不包含在 M1 中。
- 现有业务包括 Docker 容器、Nginx、systemd 服务及自定义部署脚本；Agent 不得修改其默认行为。
- Rust Server 可以和其中一个 Agent 部署在同一台 VPS 上；无需新增第三台 VPS。

### 3.3 账号与权限

- M0/M1：单管理员 + 强口令哈希 + 登录会话；支持注销、会话过期，禁止默认凭证。
- M2：操作分级（read / operate / privileged）与资源范围校验；所有变更操作需确认并记录审计。
- M3：可扩展角色（viewer/operator/admin）及 Linux 用户管理。**面板账号**与**Linux 系统账号**是不同实体，不得混淆。

## 4. 主要用户场景

**US-01：统一查看**：打开首页查看所有节点是否在线，了解 CPU、内存、磁盘和异常情况。

**US-02：跨节点定位**：搜索“Vaultwarden”或一个项目名，知道在哪台机器、由谁管理、路径在哪里，以及最后核实时间。

**US-03：追踪 Docker**：选择一个节点查看容器、状态、端口、镜像、重启次数、资源占用和日志。

**US-04：查看进程与服务**：查看占用资源最高的进程，并能关联对应的 systemd Unit 或 Docker 服务（无法关联则显示未知）。

**US-05：管理 WSL**：不依赖 WSL 动态 IP，从同一管理平台查看 WSL Ubuntu；WSL 停止时明确显示“Agent 失联”，不假定 Windows 已关机。

**US-06：受控操作（M2）**：对指定节点的指定服务执行经授权的重启/预定义脚本，看到执行结果与审计记录。

**US-07：Linux 账号管理（M3）**：查看用户/组/SSH 公钥，并在高权限确认后进行有限的增改，不提供可绕过现有 SSH 安全策略的默认快捷操作。

## 5. 功能需求与验收标准

标注：P0 必须交付（M1）；P1 M2；P2 M3。

### 5.1 节点与连接

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| NOD-001 | P0 | 中心创建节点注册凭证，Agent 主动建立 TLS 连接 | 不必开放 Agent 入站端口；节点能上线并被识别 |
| NOD-002 | P0 | 按持久 Node ID 标识节点，不使用 IP 作为主键 | WSL IP 变化后仍被识别为同一节点 |
| NOD-003 | P0 | 显示在线、离线、最近心跳、版本、系统信息 | 心跳超时标记离线；离线历史指标不被错误刷新为 0 |
| NOD-004 | P0 | 节点可自定义名称、环境类型、标签和备注 | 支持两台 VPS 和一个 WSL Ubuntu 同时登记 |
| NOD-005 | P0 | 注册凭证一次性使用或短时有效，支持吊销 | 失效凭证无法新增连接；吊销后旧 Agent 被拒绝 |
| NOD-006 | P0 | 节点重新连接自动恢复、幂等处理 | 临时断线后不产生重复 Node |

### 5.2 主机监控

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| MET-001 | P0 | CPU 利用率、核心数、负载 | 采样有时间戳、单位；符合节点 OS 视角 |
| MET-002 | P0 | 内存总量/已用/可用、Swap | 数值单位统一，展示比例和绝对值 |
| MET-003 | P0 | 文件系统挂载点、容量、可用空间 | 区分多个挂载点；伪文件系统默认过滤 |
| MET-004 | P0 | 网络接口接收/发送速率与累计字节 | 速率基于相邻采样计算，异常重置不产生巨值 |
| MET-005 | P0 | Node 最近一小时趋势，支持 24h 视图 | 新节点/缺测数据出现空档，不补 0 |
| MET-006 | P0 | 可配置采样间隔和保留时长 | 默认指标 15 秒采样、保留 7 天（可配置） |

采样和存储要求：Agent 本地采样；中心端入库并限制写入量；历史数据有自动清理策略。默认值为设计起点，不是性能承诺。

### 5.3 进程监控

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| PRO-001 | P0 | 进程列表（PID、名称、CPU、内存、状态、启动时间） | 可以排序和按名称/PID 搜索 |
| PRO-002 | P0 | 按需查询进程详情（命令行、可执行路径、父 PID、运行用户） | 无权限字段用“不可用”显示，不伪造 |
| PRO-003 | P0 | 进程快照按需读取而非全部高频上报 | 多节点切换无明显大规模历史数据积累 |
| PRO-004 | P1 | 对指定 PID 执行 TERM；KILL 作为二次高危操作 | PID 与启动时间二次核验，避免 PID 复用误杀 |

### 5.4 Docker 监控与服务发现

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| DOC-001 | P0 | 发现 Docker Engine、版本、连接状态 | 未安装/无权限/不可达分别展示 |
| DOC-002 | P0 | 列出容器（ID、名称、镜像、状态、健康状态、启动时间、重启次数） | 与 `docker ps -a` 所见可核对 |
| DOC-003 | P0 | 查询容器 CPU、内存、网络指标，及最近日志 | 查询失败有准确错误；日志长度与范围受限 |
| DOC-004 | P0 | 识别 Compose 项目/服务标签（如存在） | 缺少标签时不推测 Compose 来源 |
| DOC-005 | P0 | 获取挂载、端口、环境元数据（敏感环境变量必须脱敏/默认不采集） | 不泄露密码、Token 等密钥 |
| DOC-006 | P1 | 经授权启动、停止、重启单个容器 | 指定 Engine+容器 ID，确认后执行并审计 |
| DOC-007 | P1 | 在资产目录中关联 Docker 容器与业务服务 | 可手动维护关联并核对来源 |

**安全约束**：访问 Docker Socket 的组件具备近似主机高权限，不得把可直接访问 Socket 的进程等同普通只读沙箱。M1 优先隔离 Docker 采集能力；M2 必须单独审核权限边界。禁止公开 Docker API TCP 端口。

### 5.5 systemd 与服务资产

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| SVC-001 | P0 | 发现 systemd 可用性和可枚举的 Unit | WSL 中 systemd 未启用时有明确提示 |
| SVC-002 | P0 | 列出 Service Unit 的名称、Active/Sub 状态和描述 | 可按 Unit 名称过滤；不把 inactive 一律当异常 |
| SVC-003 | P0 | 读取可获得的 ExecStart、WorkingDirectory、FragmentPath | 字段携带来源、观测时间和可信状态 |
| AST-001 | P0 | 全局服务资产目录，可按名称、节点、运行方式、标签搜索 | 从任意页面可定位服务属于哪个 Node |
| AST-002 | P0 | 资产记录部署路径、配置路径、仓库 URL、说明、管理方式 | 允许人工录入/编辑；不确定字段显示“未核实” |
| AST-003 | P0 | 采集信息和人工登记信息分开存储 | 自动发现更新不覆盖用户填写信息 |
| AST-004 | P0 | 资产变更与发现时间可追溯 | 显示 last_seen、来源和编辑时间 |
| AST-005 | P1 | Docker、systemd、进程之间可建立关联 | 允许人工纠错，不依据名称进行强制合并 |
| SVC-004 | P1 | 受控启停/重启指定 systemd Unit | 仅允许配置的 Unit，操作记录齐全 |

**资产自动发现边界**：Docker 可发现镜像、标签、挂载和端口；systemd 可发现 Unit 与部分路径；项目源码目录、仓库归属和 Nginx 反向代理业务关系通常不能百分之百推断，必须支持人工确认。Nginx 配置解析不属于 M1 必选项。

### 5.6 受控运维与终端

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| OPS-001 | P1 | 预定义命令目录，命令参数使用明确 schema | 不拼接用户输入给 `sh -c`；参数在 Agent 二次校验 |
| OPS-002 | P1 | 任务返回 task_id、状态、退出码、输出、耗时 | 失败超时与连接中断可区分 |
| OPS-003 | P1 | 任务超时、输出截断、并发限制、取消能力 | 超时不会形成无限后台进程；输出大小受控 |
| OPS-004 | P1 | 修改类任务确认目标、动作和影响范围 | 对容器和服务误操作有确认关口 |
| OPS-005 | P1 | 全部变更操作写审计（用户、节点、目标、时间、结果） | 普通管理员不能在 UI 中篡改审计记录 |
| OPS-006 | P2 | 交互式终端，独立会话/空闲超时 | 必须二次验证或等效强化授权，禁止默认开放公网 |

若已有 LocalOpsGateway 实现预定义命令、审批、审计，可通过适配层复用；不得默认假设其接口或功能已实现。M1 不依赖该项目。

### 5.7 Linux 用户与公钥（M3）

| 编号 | 优先级 | 要求 | 验收标准 |
|---|---|---|---|
| USR-001 | P2 | 只读查看账号 UID、GID、Shell、Home、组成员 | 对系统账号做显式标识 |
| USR-002 | P2 | 查看指定用户 SSH 公钥的类型、指纹、备注 | 不读取或展示 SSH 私钥 |
| USR-003 | P2 | 经授权管理受限范围的普通用户和公钥 | root、系统账号及 sshd 安全配置不允许普通操作直接修改 |
| USR-004 | P2 | 所有账号/公钥变更有确认、审计和必要的备份策略 | 操作失败可报告恢复办法 |

## 6. WSL 专项规范

- WSL Ubuntu 运行相同 Linux Agent；`node_kind=wsl`，记录 WSL 发行版名（如可确定）、宿主机标识（手动或授权绑定）及 Linux 系统信息。
- Agent 心跳缺失仅表示 **WSL Agent 不可达**，不得断言 Windows 宿主机离线。
- WSL 可能因空闲而停止；在 Windows 启动后即使启用 WSL systemd 也不代表 Ubuntu 自动启动。
- Agent 主动连接中心，禁止依赖 WSL 的固定 IP 或要求公网映射 WSL 管理端口。
- Docker 需要区分 WSL 本机 Engine 与 Docker Desktop 集成的远程/共享 Engine；记录 Engine ID / Endpoint 指纹，避免重复计数。
- WSL 内的 CPU/内存视图不代表 Windows 宿主机全局用量；界面明确标注监控范围。
- **Windows Host Agent** 属于 M3 可选扩展，用于后续查看 Windows 在线状态、管理 WSL 生命周期；不在 M1 执行 Windows PowerShell 或 `wsl --shutdown`。

## 7. 页面与交互设计

### 7.1 信息架构

- `/login`：登录与会话管理。
- `/overview`：多节点卡片、在线情况、整体告警摘要。
- `/nodes`：节点列表、节点创建、标签和筛选。
- `/nodes/:id`：系统指标、进程、Docker、systemd、节点元信息；按页签展示。
- `/assets`：跨节点服务资产列表、搜索与筛选。
- `/assets/:id`：运行位置、运行方式、实例关联、部署路径、配置路径、仓库、备注、来源与最后观测时间。
- `/operations`、`/operations/:id`：任务列表、状态与审计。
- `/alerts`：节点告警、历史和规则。
- Linux 账号和终端入口在节点详情。允许的远端变更从节点和资源上下文发起。
- `/settings`：查看中心生效的采样、保留与终端设置。通过中心配置调整这些参数，在节点管理页调整节点凭证。

### 7.2 交互原则

1. **轻量简洁**：监控、资产和操作为一级功能，不新增没有明确需求的侧栏模块。
2. **任何数据都标注来源与时效**：在线/离线、最后采样时间、手工确认和自动发现分明。
3. **先定位后操作**：所有操作按钮必须在节点/Engine/容器/Unit 上下文中出现，防止跨节点误操作。
4. **失败可诊断**：权限不足、Agent 不在线、Docker 不存在、超时均提供有区别的反馈。
5. **不显示凭证**：日志、环境变量与命令输出含敏感信息时尽可能过滤，明确提示无法保证所有任意输出都被安全脱敏。

## 8. 组件与技术方案

- Rust stable、Cargo Workspace、Tokio 和 Axum 用于中心与 Agent。Agent 通过 TLS WebSocket 主动连接中心。
- Web 使用 React、TypeScript 和 Vite。中心提供静态资源，并以同源 API 提供面板功能。
- SQLite 和 SQLx 存放节点、指标、资产、任务、审计和告警。迁移与代码一起发布。
- Agent 使用 Linux procfs 和系统接口读取主机与进程，通过固定的本机 systemd 查询读取 Unit。Docker 读写分别由独立进程和 Unix Socket 承载。
- 发布 Linux 二进制和 Web 静态资源，使用 systemd 部署。中心与 Agent 可以独立部署。

各组件职责见[工程架构](architecture.md)。方案面向个人及小规模节点，不强制依赖 Redis、Kafka、Prometheus 或 Grafana。性能与存储成本通过测试环境测量。

## 9. 数据模型

| 实体 | 关键字段 | 说明 |
| --- | --- | --- |
| `hosts`、`nodes`、`runtimes` | 稳定 ID、类型、观测时间 | 表示宿主机、Linux 实例和运行环境。节点在线状态由心跳和凭证计算 |
| `agent_credentials`、`node_enrollment_tokens` | node_id、fingerprint、revoked_at；注册到期与消费时间 | 保存凭证摘要，换发与吊销在事务内更新 |
| `panel_admin`、`panel_sessions` | username、password_hash、token_hash、expires_at | 面板账号和会话 |
| `metric_samples` | node_id、observed_at、sample | 保存原始样本与 Agent 采样时间，索引中心接收时间 |
| `metric_rollups` | node_id、bucket_at、sample_count、各指标汇总 | 独立保留的 5 分钟趋势 |
| `service_assets` | node_id、name、management、路径、仓库、verified_at | 人工登记和确认 |
| `discovered_resources` | node_id、kind、resource_scope、resource_id、observed_at | Agent 观测的 Docker、systemd 和进程实例 |
| `asset_bindings` | asset_id、discovery_id、node_id、bound_at | 同节点的人工绑定 |
| `operation_jobs`、`operation_audit_events` | task_id、node_id、actor、action、state、执行时间和结果 | 保存确认和实际执行状态。取消以及重启中断可追溯 |
| `alert_rules`、`alert_events` | node_id、metric、threshold、for_seconds、发生及恢复时间 | 保存节点告警规则和历史 |

同名服务可在不同节点重复存在。通过节点、运行环境与实际资源身份组合定位实例。人工记录、观察信息和告警均保留各自时间。节点与资产的管理动作保留独立审计记录。容器环境变量、SSH 私钥和终端内容不进入中心数据库。

## 10. API 与 Agent 协议

### 10.1 面板和连接

| 方法和路径 | 用途 |
| --- | --- |
| `GET /healthz` | 检查中心与 SQLite 连接 |
| `POST /api/v1/auth/login`、`POST /api/v1/auth/logout` | 登录和注销 |
| `GET /api/v1/me` | 获取当前管理员会话 |
| `GET /api/v1/settings` | 查看采样间隔、保留天数及终端启用状态 |
| `GET /api/v1/nodes`、`GET /api/v1/nodes/{nodeId}` | 分页列举节点或读取节点详情 |
| `POST /api/v1/nodes/enrollment-token` | 创建节点并签发一次性注册凭证 |
| `PATCH /api/v1/nodes/{nodeId}` | 更新节点名称、类型、标签或备注 |
| `POST /api/v1/nodes/{nodeId}/enrollment-token` | 保持节点 ID 并换发注册凭证 |
| `DELETE /api/v1/nodes/{nodeId}/credentials` | 吊销节点注册凭证和设备凭证 |
| `DELETE /api/v1/nodes/{nodeId}` | 移除节点；存在关联服务资产时返回 409 |
| `GET /api/v1/agent/connect` | Agent 带身份凭证发起 TLS WebSocket 升级 |

面板 API 使用管理员会话。变更请求要求 `X-Asterion-CSRF: 1`，并检查浏览器的请求来源。错误结构包含 `code`、`message` 和 `request_id`，不返回原始凭证。

### 10.2 监控及资产

下表路径都以 `/api/v1` 为前缀。

| 方法和路径 | 用途 |
| --- | --- |
| `GET /nodes/{id}/metrics?range=1h` | 读取 1 小时或 24 小时的原始样本，支持限制条数 |
| `GET /nodes/{id}/processes`、`GET /nodes/{id}/processes/{pid}` | 进程快照和详情。列表支持名称/PID、排序、分页 |
| `GET /nodes/{id}/docker/containers` | Engine 状态和容器清单 |
| `GET /nodes/{id}/docker/containers/{container}/stats`、`GET /nodes/{id}/docker/containers/{container}/logs` | 指定完整容器 ID 的可用统计和有限日志 |
| `GET /nodes/{id}/systemd/services`、`GET /nodes/{id}/systemd/services/{unit}` | Unit 清单和详情。清单支持 `q` 名称过滤 |
| `GET /nodes/{id}/discoveries` | 按类型列举已观察到的资源 |
| `GET /assets`、`POST /assets` | 跨节点搜索与登记资产，支持名称、节点、运行方式和标签筛选 |
| `GET /assets/{id}`、`PATCH /assets/{id}`、`DELETE /assets/{id}` | 查询、编辑和移除人工记录 |
| `POST /assets/{id}/bindings`、`DELETE /assets/{id}/bindings/{discoveryId}` | 绑定或解除同节点观察资源 |

### 10.3 运维、账号、终端和告警

下表路径都以 `/api/v1` 为前缀。

| 方法和路径 | 用途 |
| --- | --- |
| `GET /nodes/{id}/operation-catalog` | 读取节点允许的操作和参数约束 |
| `POST /nodes/{id}/operations` | 准备运维任务，固定节点、目标与动作 |
| `POST /operations/{id}/confirm`、`POST /operations/{id}/cancel` | 同一会话确认或请求取消。高风险操作另外校验口令 |
| `GET /operations`、`GET /operations/{id}` | 任务列表、状态、实时受限输出及审计 |
| `GET /nodes/{id}/accounts`、`GET /nodes/{id}/accounts/{name}/keys` | Linux 账号、组和 SSH 公钥信息。账号变更走运维确认 |
| `GET /nodes/{id}/terminal/capability`、`POST /nodes/{id}/terminal` | 查询终端启用状态与经口令复核后开启会话 |
| `GET /terminal/{id}`、`POST /terminal/{id}/input`、`POST /terminal/{id}/resize`、`DELETE /terminal/{id}` | 分段读终端输出、写入、调整窗口和关闭 |
| `GET /nodes/{id}/trends?range=7d` | 读取 7 天或 30 天的 5 分钟趋势 |
| `GET /alerts` | 查看活动、已恢复告警 |
| `GET /alert-rules`、`POST /alert-rules`、`PATCH /alert-rules/{id}`、`DELETE /alert-rules/{id}` | 查看、配置和移除节点告警规则 |

### 10.4 Agent 协议

中心兼容协议 1 和 2。两者均通过已鉴权的 TLS WebSocket 承载 `hello`、`welcome`、`heartbeat` 和 `heartbeat_ack`。协议 2 增加 `configure`、`metrics`、`task`、`task_progress`、`task_reply` 和 `cancel_task`。消息携带版本、节点 ID、序号和时间戳。任务还携带 ID、类型、参数和到期时间。中心按节点、在线连接和任务 ID 隔离请求，Agent 进行本地策略和时限检查。滚动升级与详细限制见 [ADR-001](adr/001-agent-protocol.md)。

## 11. 安全基线

### 11.1 威胁模型

主要风险：面板账号被盗、Agent 注册凭证泄露、命令滥用、Docker Socket 导致主机提权、WebSocket 会话劫持、日志/资产信息泄露、请求重放、中心服务器单点故障、恶意或被攻陷 Agent 伪造监控结果。

### 11.2 强制控制

1. 默认监听回环地址。其他节点与面板通过受控私有网络接入。终端禁止对公网开放。
2. 面板口令使用强哈希算法；登录限流，会话 Cookie 设置 Secure、HttpOnly 和 SameSite 属性，变更请求检查 CSRF。
3. Agent 采用 TLS + 独立身份凭证；注册凭证短时效、可撤销；密钥文件仅进程账户可读；禁止默认共用同一个长期 Token。
4. Agent 采集进程默认最小权限；Docker、systemd、Linux 用户修改功能需单独授权与隔离。**Docker Socket 属高权限入口**。
5. M1 只读。M2 不允许任意 Shell 字符串执行，优先使用绝对路径与参数白名单，避免使用 `sh -c` 拼接。
6. M2 所有变更操作审计，针对高危操作二次确认；操作必须指定节点、运行环境与资源稳定 ID。
7. 任务设置超时、并发、输出字节数和速率限制；避免高开销采样和无限流日志造成资源耗尽。
8. UI 不保存服务器 SSH 私钥；不自动读取用户私钥；不默认修改 sshd、sudoers、防火墙或现有部署目录。
9. 中心数据和 Agent 配置有备份/恢复说明；用户可撤销节点凭证并移除节点。
10. 日志及审计存储对敏感内容实施最小化和脱敏，且声明脱敏对任意命令输出无法完全保证。

## 12. 非功能性要求

| 编号 | 要求 | 验收方式 |
|---|---|---|
| NFR-001 | 中心与 Agent 自托管，不强制依赖 SaaS/云账号 | 断开第三方 SaaS 后本地功能仍可使用 |
| NFR-002 | 支持 3 个目标节点（2 VPS + 1 WSL） | 集成环境同时在线、支持断线重连 |
| NFR-003 | Agent 启动不影响现有 Docker/systemd/Nginx | 在测试机和生产只读阶段对比现有服务状态 |
| NFR-004 | 默认短期数据保留与自动清理 | 7 天后过期采样被清理，资产和审计按独立策略保留 |
| NFR-005 | Agent 网络中断不阻塞业务，不无限增长队列 | 模拟断网后内存与磁盘不会无限增长 |
| NFR-006 | 多节点数据不可串写、串查、串操作 | 访问控制与多节点隔离测试 |
| NFR-007 | 前后端明确展示缺测、离线、无权限、未发现 | UI 端到端测试 |
| NFR-008 | 依赖及许可证清单可审计 | 生成 SBOM/依赖列表并检查许可证 |
| NFR-009 | 支持滚动升级的协议兼容窗口 | 旧 Agent 连新 Server 的兼容性测试（协议 1 至 2） |
| NFR-010 | 提供安装、升级、卸载及备份恢复文档 | 全新 Ubuntu 环境按文档完成部署 |

## 13. 测试计划与验收

### 13.1 必备测试

- Rust 单元测试：指标计算、速率计算、时间边界、状态机、参数校验、命令白名单。
- 集成测试：SQLite 迁移、Agent 注册、吊销凭证、断线重连、Server/Agent 版本不兼容。
- Docker 测试：Engine 缺失、Socket 拒绝、容器重启、容器退出、日志截断、Compose 标签缺失。
- systemd 测试：服务运行/停止、无 systemd、权限不足、路径无法确定。
- WSL 测试：WSL IP 变化、WSL 停止、恢复运行、Docker Desktop 与 WSL 独立 Engine 的差异。
- 安全测试：未授权访问、跨节点访问、重复请求、过期令牌、命令注入、CSRF、审计泄露、超量输出。
- Web E2E：节点切换、资产全局搜索、离线状态、缺测值、来源展示、录入/编辑资产。

### 13.2 M1 完成定义（DoD）

- [ ] 三个目标节点均可安装 Agent 并注册到中心（可使用测试环境替代真实生产环境验证）。
- [ ] 总览展示各节点在线状态和最后心跳；模拟离线后状态正确。
- [ ] 节点详情展示 CPU、内存、磁盘、网络和基础趋势。
- [ ] 进程列表可按需查询并过滤，权限缺失不导致页面崩溃。
- [ ] Docker 存在时能展示 Engine/容器/资源指标/有限日志；无 Docker 时展示明确状态。
- [ ] systemd Service Unit 列表与状态可见；无 systemd 时提示准确。
- [ ] 服务资产可手动创建、标注路径/仓库、关联发现实例，并按所有节点全局检索。
- [ ] 自动发现信息不会覆盖人工确认的部署路径，显示来源与最后观测时间。
- [ ] 没有任何会修改远端运行状态的接口在 M1 对外启用。
- [ ] 管理入口具备认证、Agent 认证、TLS 和基本审计；测试通过。
- [ ] 提供完整部署/卸载/配置与已知限制文档，并能从备份恢复 SQLite。

## 14. 迭代任务拆解

### M0：工程骨架与协议

1. 初始化 Rust Workspace、React 工程、静态检查和 CI。
2. 定义 `Node`、`Host`、`Runtime`、`ServiceAsset` 类型与 SQLite schema。
3. 完成登录认证、节点注册凭证、Agent TLS 通信及心跳。
4. 构建最简 `/overview`，接入模拟节点与真实 Agent。
5. 完成配置加载、结构化日志、健康检查、systemd 示例服务文件。

### M1：只读监控和资产目录

1. 主机指标和历史采样。
2. 进程按需查询。
3. Docker Engine/容器发现与只读统计、日志。
4. systemd Service Unit 列举与路径元数据。
5. 服务资产 CRUD、手工确认、关联和全局搜索。
6. WSL Agent 场景与多节点断连恢复测试。
7. 安全检查、备份/恢复和文档验收。

### M2：运维

1. 定义操作权限与命令目录；可选 LocalOpsGateway 适配。
2. Docker 容器、systemd 指定 Unit 的受控操作。
3. 任务状态、输出流、超时、撤销和审计。中心任务详情显示受限的执行中输出和最终结果。
4. 高风险操作确认与并发隔离。

### M3：进阶

1. Linux 用户/用户组/SSH 公钥只读与受控变更。
2. Web 终端及强化授权。
3. 告警、长期趋势和可配置保留周期。
4. 按需设计 Windows Host Agent（只在真正需要 WSL 生命周期管理时引入）。

## 15. 技术决策记录（ADR）

- [ADR-001](adr/001-agent-protocol.md)：Agent 连接协议及兼容版本。
- [ADR-002](adr/002-tls-and-credentials.md)：TLS、设备凭证签发和轮换。
- [ADR-003](adr/003-sqlite.md)：SQLite 持久化、短期采样和长期汇总。
- [ADR-004](adr/004-docker.md)：Docker 权限边界、隔离和启用策略。
- [ADR-005](adr/005-systemd.md)：systemd 访问方式与最小权限。
- [ADR-006](adr/006-identity.md)：宿主机、节点、运行环境身份及 WSL Engine 去重。
- [ADR-007](adr/007-operations.md)：受控运维、终端和 LocalOpsGateway 集成边界。

## 16. 风险和限制

1. Rust + React 涉及前端、后端及 Agent 三类开发与发布流程，初期开发成本高于单一工具。
2. Rust 库或系统接口可能无法完整覆盖特定 WSL/systemd 版本，必须明确版本与兼容降级路径。
3. Linux 服务“实际部署目录”通常无法自动可靠定位；界面必须支持人工登记和确认。
4. Docker Desktop 共享 Engine 可能导致跨运行环境重复发现，需要 Engine 归属和去重模型。
5. 中心服务在某台 VPS 上时会形成单点故障，必须保留独立 SSH 运维入口。
6. 开放 Docker 控制或 Linux 用户管理会增加安全风险，需要单独确认目标、最小权限和专项验收。

## 17. 开发执行准则

- 按阶段验收 M1、M2 和 M3。只读部署默认保持远端变更能力关闭。
- 每项需求必须关联编号、测试和验收证据；不得仅凭页面展示标记完成。
- 不以编造的服务器路径、CPU 占用或服务状态充当真实自动发现结果。
- 默认不覆盖用户当前 SSH、Nginx、Docker Compose、systemd、WSL 配置。
- 开发过程中新增重要取舍需写入 `docs/adr/` 并更新本文对应需求。
- 如果实现与本文冲突，先明确记录冲突和原因，再进行变更。
