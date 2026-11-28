mod facts;
pub mod forms;
mod list;
pub mod pages;
mod registry;
mod resources;
mod selection;
mod view;

use crate::{model::Sample, workspace::Workspace};
use anyhow::{Context, Result, ensure};
use axum::http::Method;
use forms::{Form, Spec, Submit, Write};
use iced::{Element, Task, widget::text_editor};
use pages::{Kind, Page, string};
use serde_json::{Value, json};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Clone)]
pub enum Event {
    Go(Page),
    SelectNode(String, String),
    NodePage(String, String, Page),
    Choose(selection::Choice),
    Tag(String),
    Refresh,
    Filter(&'static str, String),
    Search(String),
    Offset(usize),
    Back,
    Edit(Spec),
    Field(usize, String),
    Editor(text_editor::Action),
    Submit,
    Dismiss,
    Discard,
    Keep,
    Prepare(Value),
    Mutation(Method, String, Value, String, Page),
    Password(String),
    Sudo(String),
    Confirm,
    Completed(u64, Result<Value, String>),
    RestoreDraft,
    CopyDraft,
    ReloadFile,
    CopyEditor,
    Repository(String),
    ContainerTerminal(Value),
    Connect,
    Reconnect,
    Rebind,
    Disconnect,
    Terminal,
    Lock,
}
enum Pending {
    Read,
    ReloadFile(String),
    Choices,
    Write(Page, bool),
    Prepare(Option<String>),
    Confirm(String),
}
pub struct Confirm {
    pub title: String,
    pub step_up: bool,
    pub sudo: bool,
    pub action: Value,
    pub password: Zeroizing<String>,
    pub sudo_password: Zeroizing<String>,
    work: Option<Write>,
    prepared: Value,
    content: Option<String>,
}
impl Confirm {
    fn expired(&self) -> bool {
        self.prepared["expires_at"]
            .as_i64()
            .is_some_and(|end| end <= chrono::Utc::now().timestamp())
    }
}
pub struct Manager {
    pub page: Page,
    pub node: String,
    pub name: String,
    pub search: String,
    pub offset: usize,
    pub tag: String,
    picker: Option<selection::Picker>,
    pub overview: Value,
    pub live: Value,
    pub value: Value,
    pub samples: Vec<Sample>,
    pub network: Vec<crate::chart::Network>,
    pub connection_errors: std::collections::HashMap<String, String>,
    pub error: String,
    pub busy: bool,
    pub form: Option<Form>,
    pub confirm: Option<Confirm>,
    pub discard: bool,
    reload_after_discard: bool,
    pub terminal_enabled: bool,
    draft: Option<(String, Page, Form)>,
    back: Vec<(Page, String, usize)>,
    serial: u64,
    pending: Pending,
    pub catalog: Value,
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            page: Page::new(Kind::Nodes),
            node: String::new(),
            name: String::new(),
            search: String::new(),
            offset: 0,
            tag: String::new(),
            picker: None,
            overview: Value::Null,
            live: Value::Null,
            value: Value::Null,
            samples: vec![],
            network: vec![],
            connection_errors: Default::default(),
            error: String::new(),
            busy: false,
            form: None,
            confirm: None,
            discard: false,
            reload_after_discard: false,
            terminal_enabled: false,
            draft: None,
            back: vec![],
            serial: 0,
            pending: Pending::Read,
            catalog: Value::Null,
        }
    }
}
impl Manager {
    pub fn view(&self, active: bool, registered: bool) -> Element<'_, Event> {
        view::page(self, active, registered)
    }
    pub fn has_draft(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|(id, _, _)| self.page.kind == Kind::Job && self.page.key == *id)
    }
    pub fn choosing(&self) -> bool {
        self.picker.is_some()
    }
    pub fn dirty(&self) -> bool {
        self.form.as_ref().is_some_and(|f| f.dirty)
    }
    pub fn set_node(&mut self, id: String, name: String) {
        if self.node != id {
            self.draft = None;
        }
        self.node = id;
        self.name = name;
        self.catalog = Value::Null;
        self.back.clear();
    }
    pub fn load(&mut self, core: Arc<Workspace>) -> Task<Event> {
        let path = self.page.path(&self.node, &self.search, self.offset);
        let overview = self.page.kind == Kind::Nodes;
        let registry =
            (self.page.kind == Kind::Pins || self.page.kind == Kind::PinLive).then(|| {
                (
                    Page::detail(Kind::Pins, self.page.or("process"), Value::Null).path(
                        &self.node,
                        "",
                        self.offset,
                    ),
                    Page::detail(Kind::PinLive, self.page.or("process"), Value::Null).path(
                        &self.node,
                        "",
                        self.offset,
                    ),
                )
            });
        let needs_catalog = matches!(
            self.page.kind,
            Kind::Process | Kind::Container | Kind::Unit | Kind::Accounts | Kind::Keys
        );
        let node = self.node.clone();
        self.run(Pending::Read, async move {
            let primary = core.api(Method::GET, &path, Value::Null);
            if let Some((registry_path, live_path)) = registry {
                let saved = core.api(Method::GET, &registry_path, Value::Null).await?;
                let live = if view::items(&saved, "resources").is_empty() {
                    Value::Null
                } else {
                    core.api(Method::GET, &live_path, Value::Null)
                        .await
                        .unwrap_or_else(|error| json!({"error":error.to_string()}))
                };
                Ok(json!({"data":saved,"live":live}))
            } else if overview {
                let (nodes, alerts) = tokio::join!(
                    primary,
                    core.api(Method::GET, "/alerts?state=active&limit=5", Value::Null)
                );
                let alerts = alerts.unwrap_or_else(|error| json!({"error":error.to_string()}));
                Ok(json!({"data":nodes?,"overview":alerts}))
            } else if needs_catalog {
                let catalog_path = format!("/nodes/{node}/operation-catalog");
                let (data, catalog) =
                    tokio::join!(primary, core.api(Method::GET, &catalog_path, Value::Null));
                Ok(json!({"data":data?,"catalog":catalog.unwrap_or(Value::Null)}))
            } else {
                Ok(json!({"data":primary.await?}))
            }
        })
    }
    fn run(
        &mut self,
        pending: Pending,
        future: impl Future<Output = Result<Value>> + Send + 'static,
    ) -> Task<Event> {
        self.serial += 1;
        let epoch = self.serial;
        self.pending = pending;
        self.busy = true;
        self.error.clear();
        Task::perform(future, move |r| {
            Event::Completed(epoch, r.map_err(|e| format!("{e:#}")))
        })
    }
    fn go(&mut self, page: Page, remember: bool, core: Arc<Workspace>) -> Task<Event> {
        if remember {
            if self.back.len() >= 32 {
                self.back.remove(0);
            }
            self.back
                .push((self.page.clone(), self.search.clone(), self.offset));
        }
        self.offset = if page.kind == Kind::PinLive {
            page.item["offset"].as_u64().unwrap_or(0) as usize
        } else {
            0
        };
        self.tag = string(&page.item, "tag");
        self.page = page;
        self.value = Value::Null;
        self.samples.clear();
        self.form = None;
        self.confirm = None;
        self.search.clear();
        self.load(core)
    }
    fn reload_file(&mut self, core: Arc<Workspace>) -> Result<Task<Event>> {
        let form = self.form.as_ref().context("编辑器已关闭")?;
        ensure!(
            self.page.kind == Kind::Entry && form.spec.kind == forms::FormKind::File,
            "当前页面没有文件"
        );
        let path = self.page.path(&self.node, "", 0);
        Ok(
            self.run(Pending::ReloadFile(form.spec.mode.clone()), async move {
                core.api(Method::GET, &path, Value::Null).await
            }),
        )
    }
    fn write(&mut self, work: Write, core: Arc<Workspace>) -> Task<Event> {
        let removed = work.method == Method::DELETE && work.path == format!("/nodes/{}", self.node);
        self.run(Pending::Write(work.after, removed), async move {
            core.api(work.method, &work.path, work.body).await
        })
    }
    fn prepare(
        &mut self,
        action: Value,
        content: Option<String>,
        core: Arc<Workspace>,
    ) -> Result<Task<Event>> {
        let checked: asterion_protocol::operation::OperationAction =
            serde_json::from_value(action.clone())?;
        ensure!(checked.valid(), "操作参数不符合节点策略或格式要求");
        let path = format!("/nodes/{}/operations", self.node);
        Ok(self.run(Pending::Prepare(content), async move {
            core.api(Method::POST, &path, action).await
        }))
    }
    pub fn update(&mut self, event: Event, core: Arc<Workspace>) -> Result<Task<Event>> {
        match event {
            Event::Completed(s, result) if s == self.serial => {
                self.busy = false;
                let value = result.map_err(|e| {
                    if matches!(self.pending, Pending::Read) {
                        self.value = Value::Null;
                        self.catalog = Value::Null;
                    }
                    anyhow::Error::msg(e)
                })?;
                match std::mem::replace(&mut self.pending, Pending::Read) {
                    Pending::ReloadFile(mode) => {
                        ensure!(
                            value["content"].is_string() && value["revision"].is_string(),
                            "文件响应缺少内容或版本"
                        );
                        self.value = value.clone();
                        self.form = Some(Form::new(Spec::new(forms::FormKind::File, value, &mode)));
                    }
                    Pending::Choices => self.choices_loaded(value),
                    Pending::Read => {
                        self.overview = value["overview"].clone();
                        self.live = value["live"].clone();
                        let mut data = value["data"].clone();
                        self.value = Value::Null;
                        self.catalog = if value["catalog"]["enabled"] == true {
                            value["catalog"].clone()
                        } else {
                            Value::Null
                        };
                        if self.page.kind == Kind::Container {
                            let list = &data;
                            ensure!(
                                list["engine_id"] == self.page.item["engine_id"],
                                "Docker 引擎身份已改变，请重新选择"
                            );
                            let container = list["containers"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|v| v["id"] == self.page.key)
                                .context("容器已变化，请重新选择")?
                                .clone();
                            data = container;
                        }
                        if self.page.kind == Kind::Process && !self.page.item.is_null() {
                            ensure!(
                                data["start_ticks"] == self.page.item["start_ticks"]
                                    && data["name"] == self.page.item["name"],
                                "进程身份已改变，请重新选择"
                            );
                        }
                        self.value = data;
                        if self.has_draft() && self.value["state"] == "succeeded" {
                            self.draft = None;
                        }

                        if matches!(self.page.kind, Kind::Trends | Kind::Metrics) {
                            self.network = self.value["samples"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|s| {
                                    let rate = |key: &str| {
                                        if self.page.kind == Kind::Trends {
                                            s[key].as_f64()
                                        } else {
                                            s["sample"]["interfaces"]
                                                .as_array()?
                                                .iter()
                                                .filter_map(|i| i[key].as_f64())
                                                .reduce(|a, b| a + b)
                                        }
                                    };
                                    Some(crate::chart::Network {
                                        at: s["observed_at"].as_i64()?,
                                        rx: rate("rx_bytes_per_second"),
                                        tx: rate("tx_bytes_per_second"),
                                    })
                                })
                                .collect();
                            self.samples = self.value["samples"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|s| {
                                    if self.page.kind == Kind::Metrics {
                                        crate::model::from_value(&s["sample"]).ok().map(
                                            |mut sample| {
                                                sample.at =
                                                    s["observed_at"].as_i64().unwrap_or(sample.at);
                                                sample
                                            },
                                        )
                                    } else {
                                        Some(Sample {
                                            at: s["observed_at"].as_i64()?,
                                            cpu: s["cpu_percent"].as_f64(),
                                            memory: s["memory_percent"].as_f64(),
                                            disk: s["disk_percent"].as_f64(),
                                        })
                                    }
                                })
                                .collect();
                        }
                    }
                    Pending::Write(page, removed) => {
                        if removed {
                            self.set_node(String::new(), String::new());
                        }
                        return Ok(self.go(page, false, core));
                    }
                    Pending::Prepare(content) => {
                        self.confirm = Some(Confirm {
                            title: format!("确认对 {} 执行操作", self.name),
                            step_up: value["step_up_required"] == true,
                            sudo: value["action"]["type"] == "collector_deploy"
                                && self.value["root"] != true,
                            action: value["action"].clone(),
                            password: Default::default(),
                            sudo_password: Default::default(),
                            work: None,
                            prepared: value,
                            content,
                        });
                    }
                    Pending::Confirm(id) => {
                        if self
                            .form
                            .as_ref()
                            .is_some_and(|f| f.spec.kind == forms::FormKind::File)
                            && let Some(form) = self.form.take()
                        {
                            self.draft = Some((id.clone(), self.page.clone(), form));
                        }
                        return Ok(self.go(Page::detail(Kind::Job, id, Value::Null), true, core));
                    }
                }
            }
            Event::Completed(..) => {}
            Event::Repository(url) => crate::local_program::open_repository(&url)?,
            Event::CopyDraft if self.has_draft() => {
                return Ok(iced::clipboard::write(
                    self.draft.as_ref().unwrap().2.editor.text(),
                ));
            }
            Event::RestoreDraft if self.has_draft() => {
                let (_, page, form) = self.draft.take().unwrap();
                self.offset = if page.kind == Kind::PinLive {
                    page.item["offset"].as_u64().unwrap_or(0) as usize
                } else {
                    0
                };
                self.tag = string(&page.item, "tag");
                self.page = page;
                self.form = Some(form);
                self.error.clear();
                self.value = Value::Null;
                return Ok(self.load(core));
            }

            Event::Password(p) => {
                if let Some(c) = &mut self.confirm {
                    c.password = Zeroizing::new(p);
                }
            }
            Event::Sudo(p) => {
                if let Some(c) = &mut self.confirm {
                    c.sudo_password = Zeroizing::new(p);
                }
            }
            Event::Confirm if !self.busy => {
                let c = self.confirm.as_mut().context("没有待确认操作")?;
                ensure!(!c.expired(), "确认已过期，请重新准备操作");
                if let Some(work) = c.work.take() {
                    self.confirm = None;
                    return Ok(self.write(work, core));
                }
                let id = string(&c.prepared, "task_id");
                let mut body = json!({"challenge":c.prepared["challenge"]});
                if c.step_up {
                    ensure!(!c.password.is_empty(), "请输入工作区口令");
                    body["password"] = json!(c.password.as_str());
                    c.password = Default::default();
                }
                if c.sudo && !c.sudo_password.is_empty() {
                    body["sudo_password"] = json!(c.sudo_password.as_str());
                    c.sudo_password = Default::default();
                }
                if let Some(content) = &c.content {
                    body["content"] = json!(content);
                }
                let path = format!("/operations/{id}/confirm");
                return Ok(self.run(Pending::Confirm(id), async move {
                    core.api(Method::POST, &path, body).await
                }));
            }
            Event::CopyEditor => {
                if let Some(f) = &self.form {
                    return Ok(iced::clipboard::write(f.editor.text()));
                }
            }
            Event::ReloadFile if !self.busy => {
                if self.dirty() {
                    self.reload_after_discard = true;
                    self.discard = true;
                } else {
                    return self.reload_file(core);
                }
            }
            Event::Dismiss if !self.busy => {
                self.reload_after_discard = false;
                if self.confirm.take().is_none() {
                    if self.form.as_ref().is_some_and(|f| f.dirty) {
                        self.discard = true;
                    } else {
                        self.form = None;
                    }
                }
                self.error.clear();
            }
            Event::Keep => {
                self.discard = false;
                self.reload_after_discard = false;
            }
            Event::Discard if !self.busy => {
                if std::mem::take(&mut self.reload_after_discard) {
                    self.discard = false;
                    return self.reload_file(core);
                }
                self.form = None;
                self.discard = false;
                self.error.clear();
            }
            Event::Choose(e) => return self.choose(e, core),
            Event::NodePage(id, name, page) => {
                self.set_node(id, name);
                return Ok(self.go(page, false, core));
            }
            Event::Tag(t) => self.tag = t,
            Event::Edit(mut spec) => {
                if matches!(spec.kind, forms::FormKind::Asset | forms::FormKind::Rule)
                    && spec.data["node_id"].is_null()
                {
                    spec.data["node_id"] = json!(self.node);
                    spec.data["node_name"] = json!(self.name);
                }
                self.form = Some(Form::new(spec));
                self.error.clear();
            }
            Event::Field(i, v) => {
                if let Some(f) = &mut self.form {
                    f.change(i, v);
                }
            }
            Event::Editor(action) if !self.busy => {
                if let Some(f) = &mut self.form {
                    f.dirty |= action.is_edit();
                    f.editor.perform(action);
                }
            }
            Event::Submit if !self.busy => {
                let form = self.form.as_ref().context("表单已关闭")?;
                match form.submit(&self.node)? {
                    Submit::Write(work) => return Ok(self.write(work, core)),
                    Submit::Prepare(a, c) => return self.prepare(a, c, core),
                }
            }
            Event::Prepare(a) if !self.busy => return self.prepare(a, None, core),
            Event::Mutation(method, path, body, title, after) => {
                self.confirm = Some(Confirm {
                    title,
                    step_up: false,
                    sudo: false,
                    action: Value::Null,
                    password: Default::default(),
                    sudo_password: Default::default(),
                    prepared: Value::Null,
                    content: None,
                    work: Some(Write {
                        method,
                        path,
                        body,
                        after,
                    }),
                });
            }
            Event::Go(page) => return Ok(self.go(page, true, core)),
            Event::SelectNode(id, name) => {
                self.set_node(id, name);
                return Ok(self.go(Page::new(Kind::Node), false, core));
            }
            Event::Back => {
                if let Some((p, q, o)) = self.back.pop() {
                    self.tag = string(&p.item, "tag");
                    self.page = p;
                    self.search = q;
                    self.offset = o;
                    self.value = Value::Null;
                    return Ok(self.load(core));
                }
            }
            Event::Filter(k, v) => {
                if k == "node_id" {
                    self.page.item["node_name"] = Value::Null;
                }
                self.page.item[k] = json!(v);
                self.offset = 0;
                return Ok(self.load(core));
            }
            Event::Search(v) => {
                self.search = v;
                if self.page.kind == Kind::Accounts {
                    self.offset = 0;
                }
            }
            Event::Offset(o) => {
                self.offset = o;
                if self.local_rows().is_some()
                    && !matches!(self.page.kind, Kind::Docker | Kind::Units)
                {
                    return Ok(Task::none());
                }
                return Ok(self.load(core));
            }
            Event::Refresh => return Ok(self.load(core)),
            _ => {}
        }
        Ok(Task::none())
    }
}

#[cfg(test)]
mod tests;
