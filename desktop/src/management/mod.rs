mod facts;
pub mod forms;
pub mod pages;
mod resources;
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
    ContainerTerminal(Value),
    Connect,
    Reconnect,
    Disconnect,
    Terminal,
    Lock,
}
enum Pending {
    Read,
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
pub struct Manager {
    pub page: Page,
    pub node: String,
    pub name: String,
    pub search: String,
    pub offset: usize,
    pub value: Value,
    pub samples: Vec<Sample>,
    pub network: Vec<crate::chart::Network>,
    pub error: String,
    pub busy: bool,
    pub form: Option<Form>,
    pub confirm: Option<Confirm>,
    pub discard: bool,
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
            value: Value::Null,
            samples: vec![],
            network: vec![],
            error: String::new(),
            busy: false,
            form: None,
            confirm: None,
            discard: false,
            draft: None,
            back: vec![],
            serial: 0,
            pending: Pending::Read,
            catalog: Value::Null,
        }
    }
}
impl Manager {
    pub fn view(&self, active: bool) -> Element<'_, Event> {
        view::page(self, active)
    }
    pub fn has_draft(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|(id, _, _)| self.page.kind == Kind::Job && self.page.key == *id)
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
        let needs_catalog = matches!(
            self.page.kind,
            Kind::Process | Kind::Container | Kind::Unit | Kind::Accounts | Kind::Keys
        );
        let node = self.node.clone();
        self.run(Pending::Read, async move {
            let primary = core.api(Method::GET, &path, Value::Null);
            if needs_catalog {
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
        self.page = page;
        self.value = Value::Null;
        self.samples.clear();
        self.form = None;
        self.confirm = None;
        self.offset = 0;
        self.search.clear();
        self.load(core)
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
                    Pending::Read => {
                        let mut data = value["data"].clone();
                        self.value = Value::Null;
                        self.catalog = value["catalog"].clone();
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
                                data["start_ticks"] == self.page.item["start_ticks"],
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
            Event::CopyDraft if self.has_draft() => {
                return Ok(iced::clipboard::write(
                    self.draft.as_ref().unwrap().2.editor.text(),
                ));
            }
            Event::RestoreDraft if self.has_draft() => {
                let (_, page, form) = self.draft.take().unwrap();
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
            Event::Dismiss if !self.busy => {
                if self.confirm.take().is_none() {
                    if self.form.as_ref().is_some_and(|f| f.dirty) {
                        self.discard = true;
                    } else {
                        self.form = None;
                    }
                }
                self.error.clear();
            }
            Event::Keep => self.discard = false,
            Event::Discard => {
                self.form = None;
                self.discard = false;
                self.error.clear();
            }
            Event::Edit(spec) => {
                self.form = Some(Form::new(spec));
                self.error.clear();
            }
            Event::Field(i, v) => {
                if let Some(f) = &mut self.form
                    && let Some(field) = f.fields.get_mut(i)
                {
                    field.value = v;
                    f.dirty = true;
                }
            }
            Event::Editor(action) => {
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
                    self.page = p;
                    self.search = q;
                    self.offset = o;
                    self.value = Value::Null;
                    return Ok(self.load(core));
                }
            }
            Event::Filter(k, v) => {
                self.page.item[k] = json!(v);
                self.offset = 0;
                return Ok(self.load(core));
            }
            Event::Search(v) => self.search = v,
            Event::Offset(o) => {
                self.offset = o;
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
