use super::{
    Event, Manager, facts,
    pages::{Kind::*, Page, string},
    view::{items, link, mutation, section},
};
use axum::http::Method;
use iced::{
    Element,
    widget::{Column, row, rule, text},
};
use serde_json::{Value, json};

pub fn observed<'a>(kind: &str, r: &Value, live: &'a Value) -> (&'static str, Vec<&'a Value>) {
    if kind == "process" {
        let Some(rows) = live.as_array() else {
            return ("状态未知", vec![]);
        };
        let found = rows
            .iter()
            .filter(|p| p["name"] == r["target"])
            .collect::<Vec<_>>();
        return (
            if found.is_empty() {
                "未观测到"
            } else {
                "已观测到"
            },
            found,
        );
    }
    if live["status"] != "available" || live["engine_id"].as_str().is_none() {
        return ("状态未知", vec![]);
    }
    if live["engine_id"] != r["engine_id"] {
        return ("连接了其他引擎", vec![]);
    }
    let found = items(live, "containers")
        .iter()
        .filter(|c| c["id"] == r["target"])
        .collect::<Vec<_>>();
    (
        if !found.is_empty() {
            "已观测到"
        } else if live["truncated"] == true {
            "状态未知"
        } else {
            "未发现容器"
        },
        found,
    )
}
pub fn panel(m: &Manager) -> Element<'_, Event> {
    let group = m.page.or("process");
    let mut c = Column::new().spacing(7).push(
        row![
            link("进程", Pins, "process", Value::Null),
            link("容器", Pins, "docker", Value::Null),
            link(
                "添加关注",
                if group == "process" {
                    Processes
                } else {
                    Docker
                },
                "",
                Value::Null
            )
        ]
        .spacing(6),
    );
    if m.page.kind == Pins {
        c = c.push(link(
            "查看实时状态",
            PinLive,
            group,
            json!({"offset":m.offset}),
        ));
    }
    if let Some(error) = m.live["error"].as_str() {
        c = c.push(text(error));
    }
    for r in items(&m.value, "resources") {
        let (status, rows) = observed(group, r, &m.live);
        c = c.push(section(string(r, "name"))).push(text(status));
        if group == "process" && rows.len() > 12 {
            c = c.push(text("仅显示前 12 个匹配实例"));
        }
        for item in rows.into_iter().take(12) {
            let (kind, key, proof) = if group == "process" {
                (Process, string(item, "pid"), item.clone())
            } else {
                (
                    Container,
                    string(item, "id"),
                    json!({"engine_id":r["engine_id"]}),
                )
            };
            c = c.push(
                row![
                    link(&format!("详情 · {key}"), kind, key, proof),
                    facts::facts(
                        item,
                        &[
                            "state",
                            "cpu_percent",
                            "memory_bytes",
                            "image",
                            "status",
                            "health"
                        ]
                    )
                ]
                .spacing(7),
            );
        }
        c = c
            .push(mutation(
                "取消关注",
                Method::DELETE,
                format!("/nodes/{}/managed-resources/{group}/{}", m.node, r["id"]),
                Value::Null,
                format!("移除 {} 的关注？", string(r, "name")),
                Page::detail(Pins, group, Value::Null),
            ))
            .push(rule::horizontal(1));
    }
    if items(&m.value, "resources").is_empty() {
        c = c.push(text("此页没有关注资源"));
    }
    c.into()
}
