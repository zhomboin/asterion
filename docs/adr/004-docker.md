# ADR-004 Docker 读取与权限

Docker 读取和操作分别启用。Agent 通过本机 Unix Socket 连接独立的 Docker 读取程序。读取程序固定调用 Engine 的查询入口，限制容器、统计和日志的响应大小。采集包含 Engine ID、容器状态、Compose 标签、挂载和端口。容器环境变量不进入采集响应。日志有行数、字节数限制及常见敏感字段处理。

变更操作使用独立的 Docker 操作辅助程序。Admin 在面板确认后，中心下发 Engine ID、完整容器 ID 和动作。Agent 和辅助服务都检查允许列表。辅助服务再次核对 Engine 身份，避免共享 Engine 上的误操作。

能访问 Docker Socket 的程序按高权限组件部署。Agent 本身无需访问原始 Docker Socket，也不公开 Engine TCP 接口。共享 Engine 的识别规则见 [ADR-006](006-identity.md)。
