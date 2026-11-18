# 采集与运维配置

## 主机采集与历史

节点详情展示 CPU、负载、内存、Swap、挂载点和网络接口。采集使用 Agent 所在 Linux 实例的视角。WSL 页面显示 WSL 的观察结果，Windows 宿主机单独管理。首次速率和计数器重置之后可能暂时缺测，趋势按观测时刻保留空档。

中心支持以下环境变量。配置修改后重启中心，Agent 重连时获取采样间隔。面板「设置」显示生效值。

| 环境变量 | 默认值 | 范围 |
| --- | --- | --- |
| `ASTERION_SAMPLE_INTERVAL_SECS` | 15 秒 | 5～3600 秒 |
| `ASTERION_METRIC_RETENTION_DAYS` | 7 天 | 1～365 天 |
| `ASTERION_TREND_RETENTION_DAYS` | 90 天 | 30～365 天 |

按需查询进程和 systemd 服务时，面板显示读取权限、服务可用性和观察时间。在「服务资产」页登记人工字段，可以绑定同节点的发现记录。更新发现信息后，人工填入的部署路径仍由管理员维护。

## Docker

默认关闭 Docker 读取。需要监控时先配置独立的 Unix Socket 读取服务，再给 Agent 设置 `ASTERION_DOCKER_READER_SOCKET`。读取服务所在账号需要 Engine Socket 权限。Engine 可解析为本机或共享实例，使用 Engine ID 核对实际归属。容器日志仅取有限条数和字节，按常见敏感文本规则尽力脱敏。Docker 环境变量值默认不采集。

Docker 变更使用独立的操作 Socket。其策略同时限定 Engine ID、完整容器 ID 和支持的动作。两个 Socket 的访问组应分别配置。安装步骤见[远程部署](deployment.md#8-启用可选能力)。

## 受控任务

默认 Agent 未加载运维策略。启用时，将[策略样例](../deploy/executor.example.json)复制到受保护的配置目录，删除不使用的能力并替换占位符，设置 `ASTERION_EXECUTOR_CONFIG_FILE` 后重启 Agent。预定义命令使用绝对可执行路径和类型化参数，参数校验在节点侧执行。systemd 白名单登记具体的 `.service` 名称，包含 `\xHH` 转义的实例名称须按 systemd 显示的完整名称填写。[systemd Unit 命名规则](https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html)

在相应容器、Unit、进程或预定义命令处发起任务，核对节点、目标和动作后确认。进程信号按 PID 和启动标识核验，KILL 需要重新验证管理员口令。运维页展示状态、已收到的输出、耗时和审计。确认和取消采用异步处理，页面随后读取任务状态。预定义命令的子进程须留在任务进程组，取消和超时会清理该组。已经应用的服务状态变更需要通过后续操作恢复。受控本地命令要求支持 seccomp 的 Linux x86-64 或 AArch64。

## Linux 账号和公钥

Agent 按本机权限读取用户、组和授权公钥的指纹。普通账号的创建、附加组与公钥变更通过独立的本机账号操作服务启用。目标账号和可管理组来自本机策略。管理员确认目标并重新验证口令后才会提交。公钥操作会在配置的私有目录备份修改前的授权文件。受保护账号、特权组和系统 SSH 配置保留给现有主机运维流程。

## 终端、告警和趋势

交互终端通过独立的本机 Unix Socket 服务运行。先为终端配置专用非 root 账号及单独的访问组，再让中心和 Agent 设置 `ASTERION_TERMINAL_ENABLED=1`。Agent 通过 `ASTERION_TERMINAL_HELPER_SOCKET` 连接终端服务，并检查对端 UID 与 Socket 目录归属一致，且区别于 Agent UID。终端服务账号只拥有终端 Socket 访问权限。中心使用受限私网入口，每次建立会话前复核管理员口令。每节点支持一个会话，空闲 2 分钟或到达 15 分钟后关闭。多行粘贴需在面板确认。屏幕内容不入库。部署步骤见[远程部署](deployment.md#交互终端和保留周期)。

告警规则按节点配置 CPU、内存、磁盘利用率或 Agent 离线条件，可设持续时间。面板记录开始和恢复时间。长趋势使用五分钟聚合，支持最近 7 天及 30 天。告警及长期趋势不依赖外部 SaaS。
