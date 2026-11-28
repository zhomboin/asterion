use super::{
    Event, Manager, Pending,
    forms::FormKind,
    pages::{Kind, Page, enc, string},
    view::{btn, items, section},
};
use crate::workspace::Workspace;
use anyhow::{Context, Result, ensure};
use axum::http::Method;
use iced::{
    Element, Task,
    widget::{button, column, row, scrollable, text, text_input},
};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub enum Purpose {
    Filter,
    Form,
}
#[derive(Clone)]
pub enum Choice {
    Open(Purpose),
    Query(String),
    Fetch(usize),
    Select(String, String),
    Cancel,
}
pub struct Picker {
    purpose: Purpose,
    query: String,
    offset: usize,
    result: Value,
}
impl Picker {
    pub fn view(&self, busy: bool) -> Element<'_, Event> {
        let mut c = column![
            section("选择节点"),
            row![
                text_input("名称或节点 ID", &self.query)
                    .on_input(|s| Event::Choose(Choice::Query(s)))
                    .on_submit(Event::Choose(Choice::Fetch(0))),
                btn("查找", Event::Choose(Choice::Fetch(0))),
                btn("返回", Event::Choose(Choice::Cancel))
            ]
            .spacing(8)
        ]
        .spacing(10);
        if !busy && !self.result.is_null() && items(&self.result, "nodes").is_empty() {
            c = c.push(text("没有匹配的节点"));
        }
        for n in items(&self.result, "nodes") {
            let name = string(n, "name");
            c = c.push(btn(
                format!("{} · {}", name, string(n, "ssh_profile")),
                Event::Choose(Choice::Select(string(n, "id"), name)),
            ));
        }
        c = c.push(
            row![
                button("上一页").on_press_maybe(
                    (self.offset > 0)
                        .then(|| Event::Choose(Choice::Fetch(self.offset.saturating_sub(24))))
                ),
                button("下一页").on_press_maybe(
                    (self.result["has_more"] == true)
                        .then(|| Event::Choose(Choice::Fetch(self.offset + 24)))
                )
            ]
            .spacing(8),
        );
        scrollable(c).into()
    }
}
impl Manager {
    pub(super) fn choose(&mut self, event: Choice, core: Arc<Workspace>) -> Result<Task<Event>> {
        match event {
            Choice::Open(purpose) => {
                self.picker = Some(Picker {
                    purpose,
                    query: String::new(),
                    offset: 0,
                    result: Value::Null,
                });
                self.choose(Choice::Fetch(0), core)
            }
            Choice::Query(q) => {
                if let Some(p) = &mut self.picker {
                    p.query = q;
                }
                Ok(Task::none())
            }
            Choice::Cancel => {
                self.picker = None;
                self.serial += 1;
                self.busy = false;
                self.pending = Pending::Read;
                self.error.clear();
                Ok(Task::none())
            }
            Choice::Fetch(offset) => {
                let p = self.picker.as_mut().context("选择窗口已关闭")?;
                p.offset = offset;
                p.result = Value::Null;
                let path = format!("/nodes?limit=24&offset={offset}&q={}", enc(&p.query));
                Ok(self.run(Pending::Choices, async move {
                    core.api(Method::GET, &path, Value::Null).await
                }))
            }
            Choice::Select(id, name) => {
                let p = self.picker.take().context("选择窗口已关闭")?;
                ensure!(
                    items(&p.result, "nodes").iter().any(|n| n["id"] == id),
                    "请选择本页节点"
                );
                match p.purpose {
                    Purpose::Filter => {
                        self.page.item["node_id"] = json!(id);
                        self.page.item["node_name"] = json!(name);
                        self.offset = 0;
                        Ok(self.load(core))
                    }
                    Purpose::Form => {
                        let f = self.form.as_mut().context("表单已关闭")?;
                        ensure!(
                            matches!(f.spec.kind, FormKind::Asset | FormKind::Rule),
                            "表单没有归属节点"
                        );
                        f.spec.data["node_id"] = json!(id);
                        f.spec.data["node_name"] = json!(name);
                        f.dirty = true;
                        Ok(Task::none())
                    }
                }
            }
        }
    }
    pub(super) fn choices_loaded(&mut self, v: Value) {
        if let Some(p) = &mut self.picker {
            p.result = v;
        }
    }
}
pub fn target(record: &Value, page: Page) -> Event {
    Event::NodePage(string(record, "node_id"), string(record, "node_name"), page)
}
pub fn alert_target(alert: &Value) -> Event {
    let page = match (alert["metric"].as_str(), alert["unit"].as_str()) {
        (Some("service"), Some(unit)) => Page::detail(Kind::MonitorHistory, unit, Value::Null),
        _ => Page::new(Kind::Node),
    };
    target(alert, page)
}
