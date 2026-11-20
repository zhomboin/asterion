# 工程架构

浏览器通过 HTTPS 与中心通信。Rust 中心维护面板会话、SQLite 数据库、SSH 连接配置及任务调度。每个已绑定节点有独立任务队列，中心用 OpenSSH 发起采样和实时查询。节点只需接受 SSH 连接。

采样程序通过标准输入传入目标系统的 Python 解释器。中心根据采样结果记录节点状态，并把指标写入历史和聚合趋势。进程由 PID 和启动时钟识别，Docker 容器由 Engine ID 和完整容器 ID 识别，systemd 服务由具体 Unit 名识别。资产与发现记录按节点关联。

面板提交操作后，中心校验管理员会话和 CSRF、生成限时确认，再将已确认操作交给 SSH 调度。远端以登录用户的权限执行，并根据中心指定的目标、参数或内容摘要复核请求。交互终端使用独立 SSH PTY，中心将其绑定到开通终端的面板会话。

[连接与采集](monitoring-and-operations.md) · [专用面板](managed-operations.md) · [SQLite 存储](adr/003-sqlite.md)
