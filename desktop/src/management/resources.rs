use super::{
    Event, Manager, facts,
    forms::FormKind,
    pages::{Kind::*, Page, enc, string},
    view::{btn, edit, items, link, mutation, section},
};
use axum::http::Method;
use iced::{
    Element,
    widget::{Column, row, rule, text, text_input},
};
use serde_json::{Value, json};

pub fn panel(m: &Manager) -> Element<'_, Event> {
    let mut c = Column::new().spacing(8);
    let v = &m.value;
    let p = &m.page;
    let base = format!("/nodes/{}", m.node);
    let cat = &m.catalog;
    let add = |target: Value, kind: &str| {
        mutation(
            "加入关注",
            Method::POST,
            format!("{base}/managed-resources/{kind}"),
            target,
            "加入此资源的持续采集清单？".into(),
            Page::detail(Pins, kind, Value::Null),
        )
    };
    match p.kind {
        Processes => {
            let rows = v.as_array().map_or(&[][..], Vec::as_slice);
            for r in rows.iter().take(24) {
                c = c.push(
                    row![
                        link(&string(r, "name"), Process, string(r, "pid"), r.clone()),
                        text(format!(
                            "PID {}    CPU {}%    {} B",
                            r["pid"],
                            facts::value(&r["cpu_percent"]),
                            facts::value(&r["memory_bytes"])
                        ))
                    ]
                    .spacing(8),
                );
            }
            if rows.is_empty() {
                c = c.push(text("没有匹配的进程"));
            }
        }
        Process => {
            c = c.push(facts::all(v));
            c = c.push(add(
                json!({"kind":"process","pid":v["pid"],"start_ticks":v["start_ticks"]}),
                "process",
            ));
            for (sig, title) in [("term", "请求结束"), ("kill", "强制终止")] {
                if cat["process"][sig] == true {
                    c=c.push(btn(title,Event::Prepare(json!({"type":"process","pid":v["pid"],"start_ticks":v["start_ticks"],"signal":sig}))));
                }
            }
        }
        Docker => {
            c = c.push(facts::facts(v, &["status", "engine_version", "truncated"]));
            for r in items(v, "containers") {
                c = c.push(
                    row![
                        link(
                            &string(r, "name"),
                            Container,
                            string(r, "id"),
                            json!({"engine_id":v["engine_id"]})
                        ),
                        text(format!("{} · {}", string(r, "image"), string(r, "status")))
                    ]
                    .spacing(8),
                );
            }
        }
        Container => {
            let engine = &p.item["engine_id"];
            c = c.push(facts::all(v)).push(
                row![
                    link("资源统计", Stats, &p.key, Value::Null),
                    link("日志", Logs, &p.key, Value::Null),
                    add(
                        json!({"kind":"docker","container_id":v["id"],"engine_id":engine}),
                        "docker"
                    )
                ]
                .spacing(6),
            );
            if items(cat, "docker_targets")
                .iter()
                .any(|r| &r["engine_id"] == engine && items(r, "container_ids").contains(&v["id"]))
            {
                for (verb, title) in verbs() {
                    c=c.push(btn(title,Event::Prepare(json!({"type":"docker","engine_id":engine,"container_id":v["id"],"verb":verb}))));
                }
            }
        }
        Stats => c = c.push(facts::all(v)),
        Logs => {
            c = c.push(
                row![
                    link("50 行", Logs, &p.key, json!({"lines":50})),
                    link("100 行", Logs, &p.key, json!({"lines":100})),
                    link("200 行", Logs, &p.key, json!({"lines":200}))
                ]
                .spacing(6),
            );
            if v["truncated"] == true {
                c = c.push(text("日志已截断"));
            }
            c = c.push(text(string(v, "text")).size(14));
        }
        Units => {
            c = c.push(facts::facts(v, &["status", "truncated"]));
            for r in items(v, "units") {
                c = c.push(
                    row![
                        link(&string(r, "name"), Unit, string(r, "name"), Value::Null),
                        text(format!("{} · {}", string(r, "active"), string(r, "sub")))
                    ]
                    .spacing(8),
                );
            }
        }
        Unit => {
            c = c.push(facts::all(v));
            if v["verified"] == true {
                c = c.push(mutation(
                    "启用持续监控",
                    Method::PUT,
                    format!("{base}/service-monitors/{}", enc(&p.key)),
                    Value::Null,
                    format!("持续监控 {}？", p.key),
                    Page::new(Monitors),
                ));
            }
            if items(cat, "systemd_units").contains(&json!(p.key)) {
                for (verb, title) in verbs() {
                    c = c.push(btn(
                        title,
                        Event::Prepare(json!({"type":"systemd","unit":p.key,"verb":verb})),
                    ));
                }
            }
        }
        Monitors => {
            for r in items(v, "monitors") {
                let unit = string(r, "unit");
                let enabled = r["enabled"] == true;
                c = c
                    .push(section(&unit))
                    .push(facts::facts(
                        r,
                        &[
                            "enabled",
                            "status",
                            "active",
                            "sub",
                            "reason",
                            "checked_at",
                            "changed_at",
                        ],
                    ))
                    .push(
                        row![
                            link("状态记录", MonitorHistory, &unit, Value::Null),
                            mutation(
                                if enabled {
                                    "停用监控"
                                } else {
                                    "启用监控"
                                },
                                if enabled { Method::DELETE } else { Method::PUT },
                                format!("{base}/service-monitors/{}", enc(&unit)),
                                Value::Null,
                                format!("更改 {unit} 的监控状态？"),
                                p.clone()
                            )
                        ]
                        .spacing(6),
                    );
            }
            if items(v, "monitors").is_empty() {
                c = c
                    .push(text("尚未添加服务监控"))
                    .push(link("选择服务", Units, "", Value::Null));
            }
        }
        MonitorHistory => {
            for r in items(v, "history") {
                c = c.push(facts::all(r)).push(rule::horizontal(1));
            }
            if items(v, "history").is_empty() {
                c = c.push(text("暂无状态变更"));
            }
        }
        Pins => {
            c = c.push(
                row![
                    link("进程", Pins, "process", Value::Null),
                    link("容器", Pins, "docker", Value::Null),
                    link("查看实时状态", PinLive, p.or("process"), Value::Null)
                ]
                .spacing(6),
            );
            for r in items(v, "resources") {
                c = c.push(
                    row![
                        text(string(r, "name")),
                        mutation(
                            "取消关注",
                            Method::DELETE,
                            format!("{base}/managed-resources/{}/{}", p.or("process"), r["id"]),
                            Value::Null,
                            format!("移除 {} 的关注？", string(r, "name")),
                            p.clone()
                        )
                    ]
                    .spacing(6),
                );
            }
            if items(v, "resources").is_empty() {
                c = c.push(text("尚未关注资源"));
            }
        }
        PinLive => {
            let rows = v
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(items(v, "containers"));
            for r in rows {
                c = c.push(facts::facts(
                    r,
                    &[
                        "name",
                        "pid",
                        "cpu_percent",
                        "memory_bytes",
                        "image",
                        "state",
                        "status",
                    ],
                ));
            }
            if rows.is_empty() {
                c = c.push(text("当前没有匹配的实时资源"));
            }
        }
        Accounts => {
            let allowed = items(&cat["accounts"], "usernames")
                .iter()
                .filter(|name| {
                    **name != "root" && !items(v, "users").iter().any(|u| u["username"] == **name)
                })
                .collect::<Vec<_>>();
            if !allowed.is_empty() {
                c = c.push(edit(
                    "创建普通账号",
                    FormKind::Account,
                    &json!({"allowed":allowed}),
                    "create",
                ));
            }
            c = c.push(
                text_input("查找账号或 UID", &m.search)
                    .on_input(Event::Search)
                    .padding(6),
            );
            for u in items(v, "users").iter().filter(|u| {
                string(u, "username")
                    .to_lowercase()
                    .contains(&m.search.to_lowercase())
                    || string(u, "uid") == m.search
            }) {
                c = c
                    .push(text(format!(
                        "{}  ·  UID {}",
                        string(u, "username"),
                        u["uid"]
                    )))
                    .push(facts::facts(u, &["groups", "home", "shell"]));
                if cat["accounts"]["enabled"] == true
                    && items(&cat["accounts"], "usernames").contains(&u["username"])
                    && u["uid"] != 0
                    && u["system"] != true
                {
                    c = c.push(
                        row![
                            link("公钥", Keys, string(u, "username"), u.clone()),
                            edit("变更附加组", FormKind::Account, &json!({"username":u["username"],"groups":u["groups"],"allowed":cat["accounts"]["groups"]}), "set_groups")
                        ]
                        .spacing(6),
                    );
                }
            }
        }
        Keys => {
            c = c.push(section(&p.key)).push(edit(
                "添加公钥",
                FormKind::Account,
                &p.item,
                "add_key",
            ));
            for key in items(v, "keys") {
                c=c.push(facts::all(key)).push(btn("移除此公钥",Event::Prepare(json!({"type":"account","change":{"type":"remove_key","username":p.key,"fingerprint":key["fingerprint"]}}))));
            }
        }
        Catalog => {
            c = c.push(section("预定义命令"));
            for cmd in items(v, "commands") {
                c = c.push(edit(&string(cmd, "name"), FormKind::Command, cmd, ""));
            }
            for (group, title) in [
                ("packages", "软件安装"),
                ("configs", "服务配置"),
                ("containers", "容器服务"),
            ] {
                if items(v, "managed").contains(&json!(group)) {
                    c = c.push(link(title, Managed, group, Value::Null));
                }
            }
            if v["file_editor"] == true {
                c = c
                    .push(section("文件编辑"))
                    .push(
                        row![
                            text_input("远程文件的绝对路径", &m.search).on_input(Event::Search),
                            link("打开文件", Entry, &m.search, json!("files"))
                        ]
                        .spacing(6),
                    )
                    .push(link("Nginx 配置文件", Managed, "nginx", Value::Null));
            }
        }
        Managed => {
            let rows = items(v, if p.key == "nginx" { "paths" } else { "keys" });
            for key in rows.iter().filter_map(Value::as_str) {
                c = c.push(link(key, Entry, key, json!(p.key)));
            }
            if rows.is_empty() {
                c = c.push(text("没有匹配的授权目标"));
            }
        }
        Entry => {
            let group = p.item.as_str().unwrap_or("");
            c = c.push(facts::facts(
                v,
                &[
                    "name",
                    "package",
                    "installed",
                    "path",
                    "resolved",
                    "unit",
                    "reload",
                    "running",
                    "target",
                    "engine",
                    "profiles",
                ],
            ));
            let verb = match group {
                "nginx" => Some("save_nginx"),
                "files" => Some("save_text"),
                "configs" => Some("save_reload"),
                _ => None,
            };
            if let Some(verb) = verb {
                c = c.push(text(string(v, "content")).size(14)).push(edit(
                    "编辑",
                    FormKind::File,
                    v,
                    verb,
                ));
            }
            if group == "packages" && v["installed"] != true {
                c = c.push(btn("安装软件包", Event::Prepare(managed(v, "install"))));
            }
            if group == "containers" {
                let exists = v["target"].is_object();
                if v["profiles"].as_object().is_some_and(|o| !o.is_empty()) {
                    c = c.push(edit(
                        if exists {
                            "重建容器"
                        } else {
                            "创建容器"
                        },
                        FormKind::Container,
                        v,
                        if exists { "recreate" } else { "create" },
                    ));
                }
                if exists {
                    for (verb, title) in verbs() {
                        c = c.push(btn(title, Event::Prepare(managed(v, verb))));
                    }
                    if v["terminal"] == true && v["running"] == true {
                        c = c.push(btn(
                            "打开容器终端",
                            Event::ContainerTerminal(
                                json!({"key":v["key"],"engine":v["engine"],"id":v["target"]["id"]}),
                            ),
                        ));
                    }
                }
            }
        }
        Collector => {
            c = c.push(facts::all(v));
            if v["artifact_available"] == true && v["state"] != "unsupported" {
                c = c.push(edit("安装、更新或调整采集器", FormKind::Collector, v, ""));
            }
        }
        _ => {}
    }
    c.into()
}
fn verbs() -> [(&'static str, &'static str); 3] {
    [("start", "启动"), ("stop", "停止"), ("restart", "重启")]
}
fn managed(v: &Value, verb: &str) -> Value {
    json!({"type":"managed","change":{"key":v["key"],"revision":v["revision"],"verb":verb}})
}
