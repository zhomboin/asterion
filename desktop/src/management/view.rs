use super::{
    Event, Manager, facts,
    forms::{FormKind, Spec},
    pages::{Kind, Page, string},
    resources,
};
use Kind::*;
use axum::http::Method;
use iced::{
    Alignment, Color, Element, Length,
    widget::{
        Column, button, canvas, column, container, pick_list, row, rule, scrollable, space, text,
        text_editor, text_input,
    },
};
use serde_json::{Value, json};

pub fn btn<'a>(title: impl Into<String>, event: Event) -> Element<'a, Event> {
    button(text(title.into()).size(14))
        .on_press(event)
        .style(button::secondary)
        .padding([5, 9])
        .into()
}
pub fn link<'a>(
    title: &str,
    kind: Kind,
    key: impl Into<String>,
    item: Value,
) -> Element<'a, Event> {
    btn(title, Event::Go(Page::detail(kind, key, item)))
}
pub fn edit<'a>(title: &str, kind: FormKind, data: &Value, mode: &str) -> Element<'a, Event> {
    btn(title, Event::Edit(Spec::new(kind, data.clone(), mode)))
}
pub fn mutation<'a>(
    title: &str,
    method: Method,
    path: String,
    body: Value,
    description: String,
    after: Page,
) -> Element<'a, Event> {
    btn(
        title,
        Event::Mutation(method, path, body, description, after),
    )
}
pub fn section<'a>(s: impl Into<String>) -> Element<'a, Event> {
    text(s.into()).size(18).into()
}
pub fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

pub fn page(m: &Manager, active: bool) -> Element<'_, Event> {
    let modal = m.confirm.is_some() || m.form.is_some();
    let mut top = row![
        text("Asterion").size(24),
        text("管理工作区").size(18),
        space::horizontal()
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    if !modal {
        top = top
            .push(btn("SSH 连接", Event::Connect))
            .push(btn("锁定", Event::Lock));
    }
    let body: Element<'_, Event> = if let Some(c) = &m.confirm {
        let mut box_ = column![section(c.title.clone()), facts::all(&c.action)].spacing(12);
        if let Some(end) = c.prepared.get("expires_at") {
            box_ = box_.push(text(format!(
                "确认有效至 {}",
                facts::display("expires_at", end)
            )));
        }
        if c.step_up {
            box_ = box_.push(
                text_input("再次输入工作区口令", &c.password)
                    .secure(true)
                    .on_input(Event::Password),
            );
        }
        if c.sudo {
            box_ = box_.push(
                text_input("节点 sudo 密码（免密可留空）", &c.sudo_password)
                    .secure(true)
                    .on_input(Event::Sudo),
            );
        }
        box_.push(
            row![
                button("确认执行").on_press_maybe((!m.busy).then_some(Event::Confirm)),
                btn("返回", Event::Dismiss)
            ]
            .spacing(8),
        )
        .into()
    } else if let Some(form) = &m.form {
        let mut fields = column![section(form.title())].spacing(9);
        for (i, f) in form.fields.iter().enumerate() {
            let input: Element<'_, Event> = if f.choices.is_empty() {
                text_input(&f.label, &f.value)
                    .on_input(move |v| Event::Field(i, v))
                    .padding(7)
                    .into()
            } else {
                pick_list(f.choices.as_slice(), Some(f.value.clone()), move |v| {
                    Event::Field(i, v)
                })
                .into()
            };
            fields = fields.push(
                row![text(&f.label).width(210), input]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
        }
        if form.spec.kind == FormKind::File {
            fields = fields.push(text(string(&form.spec.data, "path"))).push(
                text_editor(&form.editor)
                    .on_action(Event::Editor)
                    .height(300),
            );
        }
        if form.spec.kind == FormKind::Container {
            fields = fields.push(facts::all(&form.spec.data["profiles"][form.get("profile")]));
        }
        if m.discard {
            fields = fields.push(
                row![
                    text("放弃未保存的修改？"),
                    btn("放弃修改", Event::Discard),
                    btn("继续编辑", Event::Keep)
                ]
                .spacing(8),
            );
        } else {
            fields = fields.push(
                row![
                    button("继续").on_press_maybe((!m.busy).then_some(Event::Submit)),
                    btn("返回", Event::Dismiss)
                ]
                .spacing(8),
            );
        }
        scrollable(fields).into()
    } else {
        let mut nav = column![section("工作区")].spacing(4);
        for k in [Nodes, Assets, Alerts, Rules, Jobs, Settings] {
            nav = nav.push(nav_item(k, m));
        }
        if !m.node.is_empty() {
            nav = nav.push(rule::horizontal(1)).push(text(&m.name).size(16));
            for k in [
                Node, Metrics, Trends, Processes, Docker, Units, Monitors, Pins, Catalog, Accounts,
                Collector,
            ] {
                nav = nav.push(nav_item(k, m));
            }
        }
        let mut head = row![
            section(m.page.kind.title()),
            space::horizontal(),
            btn("返回", Event::Back),
            btn("刷新", Event::Refresh)
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if m.busy {
            head = head.push(text("读取中…").size(13));
        }
        let mut content = column![head].spacing(10);
        if matches!(m.page.kind, Nodes | Assets | Processes | Docker | Units) {
            content = content.push(
                row![
                    text_input("查找名称", &m.search)
                        .on_input(Event::Search)
                        .on_submit(Event::Offset(0))
                        .padding(6),
                    btn("查找", Event::Offset(0))
                ]
                .spacing(6),
            );
        }
        if m.page.kind == Nodes {
            content = content.push(
                row![
                    btn("全部", Event::Filter("status", "".into())),
                    btn("在线", Event::Filter("status", "online".into())),
                    btn("离线", Event::Filter("status", "offline".into())),
                    btn("待连接", Event::Filter("status", "pending".into()))
                ]
                .spacing(6),
            );
        }
        if matches!(m.page.kind, Assets | Jobs) && !m.node.is_empty() {
            content = content.push(
                row![
                    btn("全部节点", Event::Filter("node_id", String::new())),
                    btn(
                        format!("当前节点 · {}", m.name),
                        Event::Filter("node_id", m.node.clone())
                    )
                ]
                .spacing(6),
            );
        }
        if m.page.kind == Assets {
            let types = [
                "全部方式",
                "docker",
                "compose",
                "systemd",
                "process",
                "other",
            ];
            content = content.push(pick_list(
                types,
                m.page.item["management"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(Some("全部方式")),
                |v| {
                    Event::Filter(
                        "management",
                        if v == "全部方式" {
                            String::new()
                        } else {
                            v.to_owned()
                        },
                    )
                },
            ));
        }
        if !m.value.is_null() {
            content = content.push(details(m, active));
        }
        let more = m.value["has_more"] == true
            || (m.page.kind == Processes && m.value.as_array().is_some_and(|a| a.len() > 24));
        if more || m.offset > 0 {
            let size = if m.page.kind == Pins { 12 } else { 24 };
            let mut paging = row![text(format!("第 {} 页", m.offset / size + 1))].spacing(8);
            if m.offset > 0 {
                paging = paging.push(btn("上一页", Event::Offset(m.offset.saturating_sub(size))));
            }
            if more {
                paging = paging.push(btn("下一页", Event::Offset(m.offset + size)));
            }
            content = content.push(paging);
        }
        row![
            scrollable(nav).width(156),
            rule::vertical(1),
            scrollable(content).width(Length::Fill)
        ]
        .spacing(12)
        .height(Length::Fill)
        .into()
    };
    let mut full = column![top, rule::horizontal(1), body]
        .spacing(10)
        .padding(16);
    if !m.error.is_empty() {
        full = full.push(
            text(&m.error)
                .size(14)
                .color(Color::from_rgb(0.65, 0.12, 0.12)),
        );
    }
    container(full).height(Length::Fill).into()
}
fn nav_item(k: Kind, m: &Manager) -> Element<'_, Event> {
    button(text(k.title()).size(14))
        .width(Length::Fill)
        .on_press(Event::Go(Page::new(k)))
        .style(if m.page.kind == k {
            button::primary
        } else {
            button::text
        })
        .into()
}

fn details(m: &Manager, active: bool) -> Element<'_, Event> {
    let v = &m.value;
    let node = &m.node;
    let p = &m.page;
    let mut c = Column::new().spacing(8);
    match p.kind {
        Nodes => {
            if let Some(counts) = v["status_counts"].as_object() {
                let n = |k| counts.get(k).and_then(Value::as_u64).unwrap_or(0);
                c = c.push(text(format!(
                    "在线 {}     离线 {}     待连接 {}",
                    n("online"),
                    n("offline"),
                    n("pending")
                )));
            }
            for n in items(v, "nodes") {
                c = c.push(
                    row![
                        btn(
                            string(n, "name"),
                            Event::SelectNode(string(n, "id"), string(n, "name"))
                        ),
                        text(facts::value(&n["status"])),
                        text(string(n, "ssh_profile"))
                    ]
                    .spacing(9)
                    .align_y(Alignment::Center),
                );
                if n["metrics"].is_object() {
                    c = c.push(facts::facts(
                        &n["metrics"],
                        &["cpu_percent", "memory_percent", "disk_percent"],
                    ));
                }
            }
            if items(v, "nodes").is_empty() {
                c = c
                    .push(text("没有匹配的节点"))
                    .push(btn("连接节点", Event::Connect));
            }
        }
        Node => {
            c = c
                .push(facts::facts(
                    v,
                    &[
                        "name",
                        "ssh_profile",
                        "host_name",
                        "kind",
                        "status",
                        "os",
                        "arch",
                        "last_seen_at",
                        "ssh_error",
                        "notes",
                    ],
                ))
                .push(
                    row![
                        edit("编辑资料", FormKind::Node, v, ""),
                        if active {
                            btn("打开 SSH 终端", Event::Terminal)
                        } else {
                            btn("连接 SSH", Event::Reconnect)
                        }
                    ]
                    .spacing(6),
                );
            if active {
                c = c.push(btn("断开此节点", Event::Disconnect));
            } else {
                c = c.push(mutation(
                    "移除节点",
                    Method::DELETE,
                    format!("/nodes/{node}"),
                    Value::Null,
                    format!("移除 {} 及其监控数据？", m.name),
                    Page::new(Nodes),
                ));
            }
        }
        Metrics | Trends => {
            let periods = if p.kind == Metrics {
                ["1h", "24h"]
            } else {
                ["7d", "30d"]
            };
            c = c.push(
                row![
                    link(periods[0], p.kind, periods[0], Value::Null),
                    link(periods[1], p.kind, periods[1], Value::Null)
                ]
                .spacing(6),
            );
            let gap = v["bucket_seconds"]
                .as_i64()
                .unwrap_or(15)
                .max(v["interval_seconds"].as_i64().unwrap_or(15))
                * 3;
            c = c
                .push(
                    row![
                        text("CPU").color(crate::chart::CPU),
                        text("内存").color(crate::chart::MEMORY),
                        text("磁盘（%）").color(crate::chart::DISK)
                    ]
                    .spacing(12),
                )
                .push(
                    canvas(crate::chart::Chart(&m.samples, gap, 100.))
                        .height(148)
                        .width(Length::Fill),
                );
            if !m.network.is_empty() {
                let peak = m
                    .network
                    .iter()
                    .flat_map(|n| [n.rx, n.tx])
                    .flatten()
                    .fold(1.0, f64::max);
                c = c
                    .push(
                        row![
                            text("接收").color(crate::chart::CPU),
                            text("发送").color(crate::chart::MEMORY),
                            text(format!("0–{peak:.0} B/s"))
                        ]
                        .spacing(12),
                    )
                    .push(
                        canvas(crate::chart::Chart(&m.network, gap, peak))
                            .height(92)
                            .width(Length::Fill),
                    );
            }
            if let Some(last) = items(v, "samples").last() {
                c = c.push(facts::all(if p.kind == Metrics {
                    &last["sample"]
                } else {
                    last
                }));
            } else {
                c = c.push(text("此时段没有采样"));
            }
            if v["collector"].is_object() {
                c = c.push(facts::all(&v["collector"]));
            }
        }
        Assets => {
            if !node.is_empty() {
                c = c.push(edit("新建资产", FormKind::Asset, &Value::Null, ""));
            }
            for a in items(v, "assets") {
                c = c.push(
                    row![
                        link(&string(a, "name"), Asset, string(a, "id"), Value::Null),
                        text(format!(
                            "{} · {}",
                            string(a, "node_name"),
                            string(a, "management")
                        ))
                    ]
                    .spacing(8),
                );
            }
            if items(v, "assets").is_empty() {
                c = c.push(text("暂无服务资产"));
            }
        }
        Asset => {
            c = c.push(facts::facts(
                v,
                &[
                    "name",
                    "node_name",
                    "management",
                    "description",
                    "repo_url",
                    "deploy_path",
                    "config_path",
                    "tags",
                    "notes",
                    "verified_at",
                    "last_seen_at",
                ],
            ));
            c = c.push(
                row![
                    edit("编辑", FormKind::Asset, v, ""),
                    mutation(
                        "删除资产",
                        Method::DELETE,
                        format!("/assets/{}", p.key),
                        Value::Null,
                        format!("删除资产 {}？", string(v, "name")),
                        Page::new(Assets)
                    ),
                    link(
                        "关联发现记录",
                        Discoveries,
                        p.key.clone(),
                        json!({"node":v["node_id"]})
                    )
                ]
                .spacing(8),
            );
            for b in items(v, "bindings") {
                c = c.push(
                    row![
                        text(string(b, "name")),
                        mutation(
                            "解除关联",
                            Method::DELETE,
                            format!("/assets/{}/bindings/{}", p.key, string(b, "id")),
                            Value::Null,
                            "解除此发现记录的关联？".into(),
                            p.clone()
                        )
                    ]
                    .spacing(6),
                );
            }
        }
        Discoveries => {
            for d in items(v, "discoveries") {
                c = c.push(
                    row![
                        text(format!("{} · {}", string(d, "kind"), string(d, "name"))),
                        mutation(
                            "关联",
                            Method::POST,
                            format!("/assets/{}/bindings", p.key),
                            json!({"discovery_id":d["id"]}),
                            format!("关联 {}？", string(d, "name")),
                            Page::detail(Asset, p.key.clone(), Value::Null)
                        )
                    ]
                    .spacing(6),
                );
            }
            if items(v, "discoveries").is_empty() {
                c = c.push(text("暂无发现记录"));
            }
        }
        Rules | Alerts => {
            if p.kind == Rules && !node.is_empty() {
                c = c.push(edit("添加当前节点规则", FormKind::Rule, &Value::Null, ""));
            }
            if p.kind == Alerts {
                c = c.push(
                    row![
                        link("活动", Alerts, "active", Value::Null),
                        link("已恢复", Alerts, "resolved", Value::Null),
                        link("全部", Alerts, "all", Value::Null)
                    ]
                    .spacing(6),
                );
            }
            let rows = items(v, if p.kind == Rules { "rules" } else { "alerts" });
            for a in rows {
                c = c.push(facts::facts(
                    a,
                    &[
                        "node_name",
                        "metric",
                        "unit",
                        "threshold",
                        "value",
                        "enabled",
                        "for_seconds",
                        "status",
                        "opened_at",
                        "resolved_at",
                        "resolution",
                    ],
                ));
                if p.kind == Rules {
                    c = c.push(
                        row![
                            edit("编辑", FormKind::Rule, a, ""),
                            mutation(
                                "删除规则",
                                Method::DELETE,
                                format!("/alert-rules/{}", string(a, "id")),
                                Value::Null,
                                "删除这条告警规则？".into(),
                                p.clone()
                            )
                        ]
                        .spacing(6),
                    );
                }
                c = c.push(rule::horizontal(1));
            }
            if rows.is_empty() {
                c = c.push(text("暂无记录"));
            }
        }
        Jobs => {
            for j in items(v, "operations") {
                c = c.push(
                    row![
                        link(
                            &format!(
                                "{} · {}",
                                string(j, "node_name"),
                                string(&j["action"], "type")
                            ),
                            Job,
                            string(j, "id"),
                            Value::Null
                        ),
                        text(facts::value(&j["state"])),
                        text(facts::display("created_at", &j["created_at"]))
                    ]
                    .spacing(8),
                );
            }
            if items(v, "operations").is_empty() {
                c = c.push(text("暂无任务"));
            }
        }
        Job => {
            c = c.push(facts::facts(
                v,
                &[
                    "node_name",
                    "actor",
                    "action",
                    "state",
                    "created_at",
                    "started_at",
                    "finished_at",
                    "output",
                    "error_code",
                    "exit_code",
                    "truncated",
                    "duration_ms",
                    "audit",
                ],
            ));
            if v["can_cancel"] == true {
                c = c.push(mutation(
                    "取消任务",
                    Method::POST,
                    format!("/operations/{}/cancel", p.key),
                    Value::Null,
                    "确认请求取消此任务？".into(),
                    p.clone(),
                ));
            }
        }
        Settings => c = c.push(facts::all(v)),
        _ => return resources::panel(m),
    }
    c.into()
}
