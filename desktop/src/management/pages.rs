use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

pub fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}
pub fn string(v: &Value, k: &str) -> String {
    v[k].as_str()
        .map(str::to_owned)
        .or_else(|| v[k].as_i64().map(|n| n.to_string()))
        .unwrap_or_default()
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Nodes,
    Node,
    Metrics,
    Trends,
    Processes,
    Process,
    Docker,
    Container,
    Stats,
    Logs,
    Units,
    Unit,
    Monitors,
    MonitorHistory,
    Pins,
    PinLive,
    Assets,
    Asset,
    Discoveries,
    Alerts,
    Rules,
    Jobs,
    Job,
    Catalog,
    Managed,
    Entry,
    Accounts,
    Keys,
    Collector,
    Settings,
}
use Kind::*;
impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Nodes => "节点总览",
            Node => "节点资料",
            Metrics => "主机指标",
            Trends => "长期趋势",
            Processes => "进程",
            Process => "进程详情",
            Docker => "Docker",
            Container => "容器详情",
            Stats => "容器资源",
            Logs => "容器日志",
            Units => "systemd",
            Unit => "服务详情",
            Monitors => "持续监控",
            MonitorHistory => "服务状态记录",
            Pins => "关注资源",
            PinLive => "关注资源的实时状态",
            Assets => "服务资产",
            Asset => "资产详情",
            Discoveries => "发现记录",
            Alerts => "告警事件",
            Rules => "告警规则",
            Jobs => "任务记录",
            Job => "任务详情",
            Catalog => "运维操作",
            Managed => "授权目标",
            Entry => "目标详情",
            Accounts => "系统账号",
            Keys => "SSH 公钥",
            Collector => "采集器部署",
            Settings => "工作区设置",
        }
    }
}

#[derive(Clone)]
pub struct Page {
    pub kind: Kind,
    pub key: String,
    pub item: Value,
}
impl Page {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            key: String::new(),
            item: Value::Null,
        }
    }
    pub fn detail(kind: Kind, key: impl Into<String>, item: Value) -> Self {
        Self {
            kind,
            key: key.into(),
            item,
        }
    }
    pub fn path(&self, node: &str, search: &str, offset: usize) -> String {
        let base = format!("/nodes/{node}");
        let key = enc(&self.key);
        let q = enc(search);
        let page = format!("limit=24&offset={offset}");
        let filter = |name: &str| {
            self.item
                .get(name)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map_or_else(String::new, |s| format!("&{name}={}", enc(s)))
        };
        match self.kind {
            Nodes => format!("/nodes?metrics=true&{page}&q={q}{}", filter("status")),
            Node => base,
            Metrics => format!("{base}/metrics?range={}&limit=240", self.or("1h")),
            Trends => format!("{base}/trends?range={}", self.or("7d")),
            Processes => format!("{base}/processes?q={q}&sort=pid&limit=25&offset={offset}"),
            Process => format!("{base}/processes/{key}"),
            Docker | Container => format!("{base}/docker?q={q}"),
            Stats | Logs => format!(
                "{base}/docker/containers/{key}/{}",
                if self.kind == Stats {
                    "stats".to_string()
                } else {
                    format!("logs?lines={}", self.item["lines"].as_u64().unwrap_or(100))
                }
            ),
            Units => format!("{base}/systemd?q={q}"),
            Unit => format!("{base}/systemd/services/{key}"),
            Monitors => format!("{base}/service-monitors"),
            MonitorHistory => format!("{base}/service-monitors/{key}/history"),
            Pins | PinLive => format!(
                "{base}/managed-resources/{}{}?offset={offset}",
                self.or("process"),
                if self.kind == PinLive { "/live" } else { "" }
            ),
            Assets => format!(
                "/assets?{page}&q={q}{}{}{}",
                filter("node_id"),
                filter("management"),
                filter("tag")
            ),
            Asset => format!("/assets/{key}"),
            Discoveries => format!(
                "/nodes/{}/discoveries?{page}",
                self.item["node"].as_str().unwrap_or(node)
            ),
            Alerts => format!(
                "/alerts?{page}{}",
                if self.key.is_empty() {
                    String::new()
                } else {
                    format!("&state={key}")
                }
            ),
            Rules => format!("/alert-rules?{page}"),
            Jobs => format!("/operations?{page}{}", filter("node_id")),
            Job => format!("/operations/{key}"),
            Catalog => format!("{base}/operation-catalog"),
            Managed => format!("{base}/managed/{key}"),
            Entry => format!(
                "{base}/managed/{}?key={key}",
                self.item.as_str().unwrap_or("files")
            ),
            Accounts => format!("{base}/accounts"),
            Keys => format!("{base}/accounts/{key}/keys"),
            Collector => format!("{base}/collector-deployment"),
            Settings => "/settings".into(),
        }
    }
    pub fn or<'a>(&'a self, default: &'a str) -> &'a str {
        if self.key.is_empty() {
            default
        } else {
            &self.key
        }
    }
}
