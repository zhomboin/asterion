use super::Event;
use iced::{
    Element, Length,
    widget::{Column, column, row, text},
};
use serde_json::Value;

pub fn failure(code: &str) -> Option<&'static str> {
    Some(match code {
        "ssh_target_changed" => "文件或实例已变化，请复制要保留的修改并重新读取。",
        "ssh_permission_denied" => "请检查 SSH 账号对文件、所在目录及备份目录的权限。",
        "ssh_file_hardlink" => "目标有多个硬链接，无法原子保存。",
        "ssh_nginx_unavailable" => "无法核对 Nginx 实例或配置，请刷新配置清单并检查读取权限。",
        _ => return None,
    })
}
pub fn metric(k: &str) -> &str {
    match k {
        "offline" => "节点离线",
        "cpu" => "CPU 利用率",
        "memory" => "内存利用率",
        "disk" => "文件系统利用率",
        "service" => "服务停机",
        s => s,
    }
}
pub fn label(k: &str) -> &str {
    match k {
        "wsl_distro" => "WSL 发行版",
        "gid" => "主组 GID",
        "bound_at" => "关联时间",
        "resource_id" => "实例标识",
        "pending" => "待同步",
        "missed" => "未能补回",
        "cancel_requested_at" => "请求取消时间",
        "name" => "名称",
        "id" => "标识",
        "node_name" => "节点",
        "ssh_profile" => "SSH 地址",
        "host_name" => "宿主机",
        "kind" => "类型",
        "os" => "系统",
        "arch" => "架构",
        "notes" => "备注",
        "description" => "说明",
        "user" => "用户",
        "username" => "账号",
        "last_seen_at" => "最后在线",
        "status" => "状态",
        "ssh_error" => "连接状态",
        "state" => "状态",
        "source" => "来源",
        "type" => "类型",
        "unit" => "服务",
        "active" => "活动状态",
        "sub" => "运行状态",
        "exec_start" => "启动命令",
        "working_directory" => "运行目录",
        "fragment_path" => "服务文件",
        "verified" => "已核实",
        "enabled" => "已启用",
        "start_ticks" => "启动标识",
        "parent_pid" => "父进程",
        "pid" => "PID",
        "command_line" => "命令行",
        "executable" => "可执行文件",
        "started_at" => "启动时间",
        "runtime" => "运行环境",
        "memory_bytes" => "内存（字节）",
        "cpu_percent" => "CPU（%）",
        "memory_percent" => "内存（%）",
        "disk_percent" => "磁盘（%）",
        "logical_cores" => "逻辑核心",
        "load" => "负载",
        "memory" => "内存",
        "swap" => "交换空间",
        "total_bytes" => "总量（字节）",
        "used_bytes" => "使用（字节）",
        "available_bytes" => "可用（字节）",
        "filesystems" => "文件系统",
        "mountpoint" => "挂载点",
        "fs_type" => "格式",
        "interfaces" => "网卡",
        "rx_bytes" => "接收字节",
        "tx_bytes" => "发送字节",
        "rx_bytes_per_second" => "接收 B/s",
        "tx_bytes_per_second" => "发送 B/s",
        "captured_at" | "observed_at" => "采样时间",
        "image" => "镜像",
        "health" => "健康状态",
        "ports" => "端口",
        "mounts" => "挂载",
        "restart_count" => "重启次数",
        "engine_id" | "engine" => "引擎",
        "engine_version" => "引擎版本",
        "compose_project" => "Compose 项目",
        "compose_service" => "Compose 服务",
        "home" => "家目录",
        "shell" => "登录 Shell",
        "groups" => "附加组",
        "system" => "系统账号",
        "management" => "管理方式",
        "tags" => "标签",
        "deploy_path" => "部署目录",
        "config_path" => "配置文件",
        "repo_url" => "仓库",
        "verified_at" => "核实时间",
        "metric" => "指标",
        "threshold" => "阈值",
        "value" => "观测值",
        "resolution" => "恢复原因",
        "opened_at" => "触发时间",
        "resolved_at" => "恢复时间",
        "for_seconds" => "持续秒数",
        "checked_at" => "检查时间",
        "changed_at" => "变化时间",
        "reason" => "原因",
        "actor" => "执行人",
        "action" => "操作",
        "output" => "输出",
        "error_code" => "错误",
        "exit_code" => "退出码",
        "created_at" => "创建时间",
        "finished_at" => "结束时间",
        "duration_ms" => "耗时（毫秒）",
        "audit" => "审计记录",
        "truncated" => "结果已截断",
        "challenge" => "确认码",
        "verb" => "动作",
        "key" => "目标",
        "revision" => "目标版本",
        "profile" => "配置方案",
        "settings" => "参数",
        "change" => "变更",
        "parameters" => "命令参数",
        "root" => "以 root 连接",
        "artifact_available" => "有匹配架构的安装文件",
        "interval_secs" | "sample_interval_secs" => "采样间隔（秒）",
        "max_mib" => "缓存上限（MiB）",
        "retention_days" | "metric_retention_days" => "指标保留天数",
        "trend_retention_days" => "趋势保留天数",
        "terminal_enabled" => "终端已启用",
        "enabled_at" => "启用时间",
        "fingerprint" => "指纹",
        "comment" => "备注",
        "path" => "文件路径",
        "resolved" => "实际路径",
        "package" => "软件包",
        "installed" => "已安装",
        "running" => "运行中",
        "target" => "目标",
        "profiles" => "可用配置方案",
        "signal" => "信号",
        "digest" => "内容摘要",
        "command" => "命令",
        "engine_version_string" => "引擎版本",
        "sample_count" => "采样数",
        "memory_limit_bytes" => "内存限制（字节）",
        "container_id" => "容器 ID",
        "terminal" => "终端",
        "restart" => "重启策略",
        "restart_choices" => "可选重启策略",
        "port_choices" => "可选端口",
        "mount_choices" => "可选挂载",
        "destination" => "目标",
        "mode" => "模式",
        "private_port" => "容器端口",
        "public_port" => "宿主端口",
        "protocol" => "协议",
        "address" => "地址",
        "action_type" => "操作类型",
        "expires_at" => "过期时间",
        other => other,
    }
}
pub fn value(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::Bool(b) => if *b { "是" } else { "否" }.into(),
        Value::String(s) => match s.as_str() {
            "online" => "在线",
            "offline" => "离线",
            "pending" => "等待",
            "active" => "活动",
            "resolved" => "已恢复",
            "succeeded" => "成功",
            "failed" => "失败",
            "queued" => "排队中",
            "running" => "运行中",
            "prepared" => "待确认",
            "expired" => "已过期",
            "interrupted" => "中断",
            "cancelled" => "已取消",
            "disconnected" => "已断开",
            "timed_out" => "超时",
            "available" => "可用",
            "unavailable" => "不可用",
            "permission_denied" => "权限不足",
            "not_installed" => "未安装",
            "healthy" => "正常",
            "down" => "停止",
            "unknown" => "未知",
            "enabled" => "启用",
            "disabled" => "停用",
            "recovered" => "已恢复",
            "true" => "是",
            "false" => "否",
            s => s,
        }
        .into(),
        Value::Array(a) => a.iter().map(value).collect::<Vec<_>>().join("  ·  "),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| format!("{} {}", label(k), value(v)))
            .collect::<Vec<_>>()
            .join("  ·  "),
        Value::Number(n) => n
            .as_f64()
            .filter(|_| n.is_f64())
            .map_or_else(|| n.to_string(), |x| format!("{x:.2}")),
    }
}
pub fn display(k: &str, v: &Value) -> String {
    if k == "metric"
        && let Some(s) = v.as_str()
    {
        return metric(s).into();
    }
    if k.ends_with("_at")
        && let Some(t) = v.as_i64()
        && let Some(t) = chrono::DateTime::from_timestamp(t, 0)
    {
        return t
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
    }
    value(v)
}
pub fn facts<'a>(data: &Value, keys: &[&str]) -> Element<'a, Event> {
    let mut list = Column::new().spacing(5);
    for key in keys {
        if let Some(v) = data.get(*key)
            && !v.is_null()
        {
            list = list.push(
                row![
                    text(label(key).to_owned()).size(14).width(112),
                    text(display(key, v)).size(14).width(Length::Fill)
                ]
                .spacing(10),
            );
        }
    }
    list.into()
}
pub fn all<'a>(v: &Value) -> Element<'a, Event> {
    if let Some(o) = v.as_object() {
        facts(
            v,
            &o.keys()
                .filter(|k| {
                    !matches!(
                        k.as_str(),
                        "challenge" | "content" | "sudo_password" | "password"
                    )
                })
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
    } else {
        column![text(value(v)).size(14)].into()
    }
}
