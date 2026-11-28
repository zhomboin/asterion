use crate::{
    App, Message,
    management::{
        Event, Manager,
        pages::{Kind, Page},
    },
    model::Node,
    ssh::Session,
    store::Store,
    workspace::{Running, Workspace, remote::Remote},
};
use anyhow::{Context, Result, ensure};
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, text, text_input},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use zeroize::Zeroizing;

#[derive(Clone)]
pub enum Action {
    Show,
    Hide,
    Password(String),
    Repeat(String),
    Unlock,
    Unlocked(u64, Result<Arc<Workspace>, String>),
    Attached(
        u64,
        Node,
        Session,
        Result<(String, Arc<Running>, Remote), String>,
    ),
    Manage(Event),
    Finished(u64, Event),
}
pub struct Connection {
    pub node: Node,
    pub session: Session,
    _worker: Arc<Running>,
    pub remote: Remote,
}

pub struct Desk {
    pub visible: bool,
    pub active: HashMap<String, Connection>,
    pub container: Option<(Remote, Value)>,
    core: Option<Arc<Workspace>>,
    manager: Manager,
    password: Zeroizing<String>,
    repeat: Zeroizing<String>,
    first: bool,
    busy: bool,
    generation: u64,
    error: String,
}
impl Default for Desk {
    fn default() -> Self {
        Self {
            visible: false,
            active: HashMap::new(),
            container: None,
            core: None,
            manager: Default::default(),
            password: Default::default(),
            repeat: Default::default(),
            first: true,
            busy: false,
            generation: 0,
            error: String::new(),
        }
    }
}
fn msg(e: Event) -> Message {
    Message::Desk(Action::Manage(e))
}
pub fn view(app: &App) -> Element<'_, Message> {
    let d = &app.desk;
    if d.core.is_some() {
        return d
            .manager
            .view(
                d.active
                    .get(&d.manager.node)
                    .is_some_and(|c| !c.session.is_closed()),
            )
            .map(msg);
    }
    let mut form = column![
        text(if d.first {
            "设置工作区口令"
        } else {
            "解锁管理工作区"
        })
        .size(24),
        text_input(
            if d.first {
                "12 至 128 字节的口令"
            } else {
                "工作区口令"
            },
            &d.password
        )
        .secure(true)
        .on_input(|v| Message::Desk(Action::Password(v)))
        .on_submit(Message::Desk(Action::Unlock))
        .padding(9)
    ]
    .spacing(12)
    .max_width(430);
    if d.first {
        form = form.push(
            text_input("再次输入口令", &d.repeat)
                .secure(true)
                .on_input(|v| Message::Desk(Action::Repeat(v)))
                .on_submit(Message::Desk(Action::Unlock))
                .padding(9),
        );
    }
    form = form.push(
        row![
            button(if d.busy {
                "正在解锁…"
            } else {
                "进入工作区"
            })
            .on_press_maybe((!d.busy).then_some(Message::Desk(Action::Unlock))),
            button("返回")
                .style(button::secondary)
                .on_press(Message::Desk(Action::Hide))
        ]
        .spacing(8),
    );
    if !d.error.is_empty() {
        form = form.push(text(&d.error));
    }
    container(form).center(Length::Fill).into()
}
impl App {
    pub(super) fn attach_workspace(&mut self) -> Result<Task<Message>> {
        let (Some(core), Some(session)) = (self.desk.core.clone(), self.session.clone()) else {
            return Ok(Task::none());
        };
        let node = self.node()?;
        let epoch = self.epoch;
        let n = node.clone();
        let s = session.clone();
        Ok(Task::perform(
            async move { core.attach(n, s).await.map_err(|e| format!("{e:#}")) },
            move |r| Message::Desk(Action::Attached(epoch, node.clone(), session.clone(), r)),
        ))
    }
    pub(super) fn refresh_management(&mut self) -> Task<Message> {
        let d = &mut self.desk;
        let m = &mut d.manager;
        if !d.visible || m.busy || m.form.is_some() || m.confirm.is_some() || !m.error.is_empty() {
            return Task::none();
        }
        if let Some(core) = &d.core
            && matches!(
                m.page.kind,
                Kind::Job
                    | Kind::Alerts
                    | Kind::Metrics
                    | Kind::Trends
                    | Kind::Monitors
                    | Kind::Stats
                    | Kind::PinLive
                    | Kind::Jobs
            )
        {
            let generation = d.generation;
            return m
                .load(core.clone())
                .map(move |e| Message::Desk(Action::Finished(generation, e)));
        }
        Task::none()
    }
    pub(super) fn desktop(&mut self, action: Action) -> Result<Task<Message>> {
        let result = self.desktop_action(action);
        match result {
            Ok(t) => Ok(t),
            Err(e) => {
                if !self.desk.visible {
                    self.notice = format!("{e:#}");
                }
                if self.desk.core.is_some() {
                    self.desk.manager.error = format!("{e:#}");
                } else {
                    self.desk.error = format!("{e:#}");
                }
                Ok(Task::none())
            }
        }
    }
    fn desktop_action(&mut self, action: Action) -> Result<Task<Message>> {
        let generation = self.desk.generation;
        let done = move |e| Message::Desk(Action::Finished(generation, e));
        let action = match action {
            Action::Finished(g, e) if g == generation => Action::Manage(e),
            Action::Finished(..) => return Ok(Task::none()),
            other => other,
        };
        match action {
            Action::Show => {
                self.close_term();
                self.desk.visible = true;
                self.desk.error.clear();
                if let Some(core) = self.desk.core.clone() {
                    return Ok(self.desk.manager.load(core).map(done));
                }
                self.desk.first = !Store::workspace_configured()?;
            }
            Action::Hide => {
                self.desk.visible = false;
                self.desk.generation += 1;
                self.desk.busy = false;
                self.desk.password = Default::default();
                self.desk.repeat = Default::default();
            }
            Action::Password(v) => self.desk.password = Zeroizing::new(v),
            Action::Repeat(v) => self.desk.repeat = Zeroizing::new(v),
            Action::Unlock if !self.desk.busy => {
                if self.desk.first {
                    ensure!(
                        !self.desk.password.is_empty() && self.desk.password == self.desk.repeat,
                        "两次口令必须相同"
                    );
                }
                self.desk.busy = true;
                self.desk.error.clear();
                self.desk.generation += 1;
                let generation = self.desk.generation;
                let password = std::mem::take(&mut self.desk.password);
                self.desk.repeat = Default::default();
                return Ok(Task::perform(Workspace::unlock(password), move |r| {
                    Message::Desk(Action::Unlocked(
                        generation,
                        r.map_err(|e| format!("{e:#}")),
                    ))
                }));
            }
            Action::Unlocked(g, r) if g == self.desk.generation => {
                self.desk.busy = false;
                let core = r.map_err(anyhow::Error::msg)?;
                self.desk.first = false;
                self.desk.core = Some(core.clone());
                return Ok(Task::batch([
                    self.desk.manager.load(core).map(done),
                    self.attach_workspace()?,
                ]));
            }
            Action::Attached(e, node, session, r)
                if e == self.epoch && self.desk.core.is_some() =>
            {
                let (id, worker, remote) = r.map_err(anyhow::Error::msg)?;
                self.desk.active.insert(
                    id.clone(),
                    Connection {
                        node: node.clone(),
                        session,
                        _worker: worker,
                        remote,
                    },
                );
                self.desk.manager.set_node(id, node.to_string());
                self.desk.manager.page = Page::new(Kind::Node);
                if self.desk.visible {
                    return Ok(self
                        .desk
                        .manager
                        .load(self.desk.core.clone().unwrap())
                        .map(done));
                }
            }
            Action::Manage(Event::Disconnect) => {
                let id = self.desk.manager.node.clone();
                if let Some(conn) = self.desk.active.remove(&id) {
                    if self.node().is_ok_and(|n| Workspace::id(&n) == id) {
                        self.session = None;
                        self.close_term();
                        self.epoch += 1;
                        self.sampling = false;
                        self.history.clear();
                        self.node_id = None;
                    }
                    drop(conn._worker);
                    return Ok(Task::perform(
                        async move {
                            let _ = conn
                                .session
                                .disconnect(russh::Disconnect::ByApplication, "", "")
                                .await;
                        },
                        |_| Message::Idle,
                    ));
                }
            }
            Action::Manage(e @ (Event::Connect | Event::Reconnect)) => {
                if let Some(store) = &self.store {
                    self.nodes = store.nodes()?;
                }
                if matches!(e, Event::Reconnect) {
                    let node = self
                        .nodes
                        .iter()
                        .find(|n| Workspace::id(n) == self.desk.manager.node)
                        .cloned()
                        .context("没有找到已保存的 SSH 地址")?;
                    let _ = self.handle(Message::Select(node))?;
                }
                // 管理工作区已登记的连接继续采样，新的连接有独立的身份和请求代次。
                self.close_term();
                self.session = None;
                self.epoch += 1;
                self.sampling = false;
                self.node_id = None;
                self.history.clear();
                self.desk.visible = false;
                self.notice.clear();
                self.metrics_error.clear();
            }
            Action::Manage(Event::Lock) => {
                self.close_term();
                self.epoch += 1;
                self.session = None;
                self.sampling = false;
                self.node_id = None;
                self.history.clear();
                let core = self.desk.core.take();
                let active = std::mem::take(&mut self.desk.active);
                let old_generation = self.desk.generation;
                self.desk = Desk {
                    visible: true,
                    first: false,
                    generation: old_generation + 1,
                    ..Desk::default()
                };
                return Ok(Task::perform(
                    async move {
                        if let Some(core) = core {
                            let _ = core
                                .api(axum::http::Method::POST, "/auth/logout", Value::Null)
                                .await;
                        }
                        for (_, entry) in active {
                            let session = entry.session;
                            drop(entry._worker);
                            let _ = session
                                .disconnect(russh::Disconnect::ByApplication, "", "")
                                .await;
                        }
                    },
                    |_| Message::Idle,
                ));
            }
            Action::Manage(e @ (Event::Terminal | Event::ContainerTerminal(_))) => {
                let d = &self.desk;
                let conn = d
                    .active
                    .get(&d.manager.node)
                    .filter(|c| !c.session.is_closed())
                    .context("请先通过 SSH 连接这个节点")?;
                let session = conn.session.clone();
                let node = conn.node.clone();
                let target = if let Event::ContainerTerminal(v) = e {
                    Some((conn.remote.clone(), v))
                } else {
                    None
                };
                self.close_term();
                self.epoch += 1;
                self.sampling = false;
                self.session = Some(session);
                self.host = node.host.clone();
                self.user = node.user.clone();
                self.port = node.port.to_string();
                let store = self.store.as_ref().context("本地数据不可用")?;
                let id = store.connected(&node)?;
                self.history = store.history(id)?;
                self.node_id = Some(id);
                self.desk.container = target;
                self.desk.visible = false;
                self.notice.clear();
                return self.handle(Message::Open);
            }
            Action::Manage(event) => {
                let core = self.desk.core.clone().context("请先解锁管理工作区")?;
                return Ok(self.desk.manager.update(event, core)?.map(done));
            }
            _ => {}
        }
        Ok(Task::none())
    }
}
