#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod chart;
mod connect;
mod connection_settings;
mod desk;
mod host_trust;
mod jump;
mod keys;
mod local_program;
mod management;
mod model;
mod screen;
mod ssh;
mod ssh_config;
mod store;
mod terminal;
mod ui;
mod vault;
mod workspace;

use iced::{Font, Size, Subscription, Task, Theme, widget::operation};
use model::{Node, Sample};
use ssh::Session;
use std::{sync::Arc, time::Duration};
use store::Store;
use terminal::{Event, Input};
use tokio::sync::mpsc;
use zeroize::{Zeroize, Zeroizing};

fn main() -> iced::Result {
    iced::application(App::new, App::update, ui::view)
        .title("Asterion · 节点工作台")
        .theme(|_: &App| Theme::Light)
        .default_font(Font::with_name("Microsoft YaHei UI"))
        .window(iced::window::Settings {
            size: Size::new(1060., 790.),
            exit_on_close_request: false,
            min_size: Some(Size::new(780., 620.)),
            ..Default::default()
        })
        .subscription(App::subscription)
        .run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Config,
    Password,
    Key,
}
#[derive(Clone)]
enum Message {
    Desk(desk::Action),
    CloseWindow(iced::window::Id),
    KeepWindow,
    DiscardWindow,
    Idle,
    Focus,
    Host(String),
    Port(String),
    User(String),
    Secret(String),
    KeyPath(String),
    Method(Method),
    Select(Node),
    Connect,
    Trust,
    VerifyHost,
    ConfigName(String),
    Remember(bool),
    Auto(bool),
    ForgetSecret,
    Resolved(u64, Result<Arc<ssh_config::Config>, String>),
    Probed(u64, Result<String, String>),
    Cancel,
    Login(u64, Result<ssh::Login, String>),
    Tick,
    Sample(u64, Result<Sample, String>),
    Open,
    Close,
    Term(u64, Event),
    Text(String),
    Key(Vec<u8>),
    Geometry(u16, u16),
    Scroll(i32),
    Copy,
    Paste,
    Clipboard(u64, Option<String>),
    PasteConfirm,
    PasteCancel,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", std::mem::discriminant(self))
    }
}

struct App {
    desk: desk::Desk,
    terminal_allowed: bool,
    connection: connect::Options,
    closing: Option<iced::window::Id>,
    store: Option<Store>,
    nodes: Vec<Node>,
    host: String,
    port: String,
    user: String,
    secret: Zeroizing<String>,
    key_path: String,
    method: Method,
    notice: String,
    busy: bool,
    verify: Option<String>,
    epoch: u64,
    session: Option<Session>,
    node_id: Option<i64>,
    history: Vec<Sample>,
    sampling: bool,
    metrics_error: String,
    term_epoch: u64,
    input: Option<mpsc::Sender<Input>>,
    ready: bool,
    screen_cache: iced::widget::canvas::Cache,
    editor: iced::widget::text_editor::Content,
    parser: vt100::Parser,
    geometry: (u16, u16),
    term_status: String,
    paste: Option<String>,
}

impl App {
    fn new() -> Self {
        let store = Store::open();
        let mut notice = String::new();
        let store = store.map_err(|e| notice = format!("{e:#}")).ok();
        let nodes = store
            .as_ref()
            .map(|s| {
                s.nodes().unwrap_or_else(|e| {
                    notice = format!("无法读取节点：{e}");
                    Vec::new()
                })
            })
            .unwrap_or_default();
        let first = nodes.first();
        let mut app = Self {
            connection: Default::default(),
            desk: Default::default(),
            terminal_allowed: asterion_server::config::MonitoringConfig::from_env()
                .is_ok_and(|c| c.terminal_enabled),
            closing: None,
            host: first.map(|n| n.host.clone()).unwrap_or_default(),
            port: first
                .map(|n| n.port.to_string())
                .unwrap_or_else(|| "22".into()),
            user: first.map(|n| n.user.clone()).unwrap_or_default(),
            store,
            nodes,
            notice,
            secret: Zeroizing::default(),
            key_path: String::new(),
            method: Method::Password,
            busy: false,
            verify: None,
            epoch: 0,
            session: None,
            node_id: None,
            history: Vec::new(),
            sampling: false,
            metrics_error: String::new(),
            term_epoch: 0,
            input: None,
            ready: false,
            screen_cache: Default::default(),
            editor: Default::default(),
            parser: vt100::Parser::new(12, 80, 0),
            geometry: (80, 12),
            term_status: String::new(),
            paste: None,
        };
        if let Some(first) = app.nodes.first().cloned()
            && let Err(e) = app.restore_choice(&first)
        {
            app.notice = e.to_string();
        }
        app
    }

    fn node(&self) -> anyhow::Result<Node> {
        Node::parse(&self.host, &self.port, &self.user)
    }
    fn connect(&mut self) -> anyhow::Result<Task<Message>> {
        let node = self.node()?;
        anyhow::ensure!(
            !self.desk.restoring.contains_key(&node.to_string()),
            "正在恢复此节点的 SSH 连接"
        );
        let expected = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("本机数据不可用"))?
            .fingerprint(&node)?;
        let auth = self.credentials(&node)?;
        self.busy = true;
        self.notice.clear();
        self.epoch += 1;
        let epoch = self.epoch;
        Ok(Task::perform(ssh::login(node, expected, auth), move |r| {
            Message::Login(epoch, r.map_err(|e| format!("{e:#}")))
        }))
    }

    fn connected(&mut self, node: &Node, session: ssh::Session) -> anyhow::Result<()> {
        let remembered = self.remember_choice(node);
        self.secret.zeroize();
        remembered?;
        let db = self.store.as_ref().unwrap();
        let id = db.connected(node)?;
        self.history = db.history(id)?;
        self.nodes = db.nodes()?;
        self.session = Some(session);
        self.node_id = Some(id);
        self.notice.clear();
        self.term_status.clear();
        Ok(())
    }

    fn poll(&mut self) -> Task<Message> {
        let Some(session) = self.session.clone() else {
            return Task::none();
        };
        if self.sampling {
            return Task::none();
        }
        self.sampling = true;
        let epoch = self.epoch;
        Task::perform(
            async move {
                tokio::time::timeout(Duration::from_secs(16), ssh::sample(session))
                    .await
                    .map_err(|_| "采样超时".to_owned())?
                    .map_err(|e| format!("{e:#}"))
            },
            move |r| Message::Sample(epoch, r),
        )
    }

    fn close_term(&mut self) {
        self.term_epoch += 1;
        self.input = None;
        self.ready = false;
        self.paste = None;
        self.term_status = "终端已关闭".into();
    }

    fn send(&mut self, input: Input) {
        if let Some(tx) = &self.input
            && tx.try_send(input).is_err()
        {
            self.term_status = "终端繁忙或已断开，请重试".into();
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match self.handle(msg) {
            Ok(t) => t,
            Err(e) => {
                self.notice = format!("{e:#}");
                Task::none()
            }
        }
    }

    fn handle(&mut self, msg: Message) -> anyhow::Result<Task<Message>> {
        match msg {
            Message::Desk(action) => return self.desktop(action),
            Message::CloseWindow(id) if self.desk.dirty() => self.closing = Some(id),
            Message::CloseWindow(id) => return Ok(iced::window::close(id)),
            Message::KeepWindow => self.closing = None,
            Message::DiscardWindow => {
                if let Some(id) = self.closing.take() {
                    return Ok(iced::window::close(id));
                }
            }
            Message::Host(v) => {
                self.changed_destination();
                self.host = v;
            }
            Message::Port(v) => {
                self.changed_destination();
                self.port = v;
            }
            Message::User(v) => {
                self.changed_destination();
                self.user = v;
            }
            Message::Secret(v) => self.secret = Zeroizing::new(v),
            Message::KeyPath(v) => self.key_path = v,
            Message::Method(m) => {
                self.changed_destination();
                self.method = m;
                if m == Method::Config {
                    self.connection.profiles = ssh_config::names()?;
                }
                self.secret.zeroize();
            }
            Message::Select(n) => {
                self.key_path.clear();
                self.restore_choice(&n)?;
                self.host = n.host;
                self.port = n.port.to_string();
                self.user = n.user;
                self.secret.zeroize();
                self.notice.clear();
            }
            Message::Connect if !self.busy && self.session.is_none() && self.verify.is_none() => {
                return self.begin(connect::Intent::Login);
            }
            Message::VerifyHost if !self.busy && self.session.is_none() => {
                return self.begin(connect::Intent::Verify);
            }
            Message::ConfigName(name) => {
                self.changed_destination();
                self.connection.name = name;
            }
            Message::Remember(v) => {
                self.connection.remember = v;
                if !v {
                    self.connection.auto = false;
                }
            }
            Message::Auto(v) => self.connection.auto = v,
            Message::ForgetSecret => {
                let n = self.node()?;
                self.store.as_ref().unwrap().forget_login(&n)?;
                self.changed_destination();
                self.notice = "已清除保存的登录信息".into();
            }
            Message::Resolved(e, r) if self.epoch == e => {
                self.busy = false;
                return self.accept_profile(r.map_err(anyhow::Error::msg)?);
            }
            Message::Probed(e, r) if self.epoch == e => {
                self.busy = false;
                let key = r.map_err(anyhow::Error::msg)?;
                match self
                    .store
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("本机数据不可用"))?
                    .fingerprint(&self.node()?)?
                {
                    Some(old) if old == key => self.notice = format!("已核对主机指纹 · {key}"),
                    Some(_) => anyhow::bail!("服务器指纹与已信任记录不符"),
                    None => self.verify = Some(key),
                }
            }
            Message::Trust => {
                if let Some(key) = self.verify.take() {
                    self.store.as_ref().unwrap().trust(&self.node()?, &key)?;
                    if self.connection.verify_only {
                        return self.perform_connection();
                    }
                    return self.connect();
                }
            }
            Message::Cancel if !self.connection.committing => {
                self.close_term();
                if self.session.is_some()
                    && let Ok(node) = self.node()
                {
                    self.desk.active.retain(|_, c| c.node != node);
                }
                let session = self.session.take();
                self.epoch += 1;
                self.verify = None;
                self.connection.rebind = None;
                self.busy = false;
                self.secret.zeroize();
                self.node_id = None;
                self.history.clear();
                self.sampling = false;
                self.notice.clear();
                self.metrics_error.clear();
                if let Some(session) = session {
                    return Ok(Task::perform(
                        async move {
                            let _ = session
                                .disconnect(russh::Disconnect::ByApplication, "", "")
                                .await;
                        },
                        |_| Message::Idle,
                    ));
                }
            }
            Message::Login(epoch, r) if self.epoch == epoch => {
                self.busy = false;
                match r {
                    Ok(ssh::Login::Verify(key)) => self.verify = Some(key),
                    Ok(ssh::Login::Connected(session)) => {
                        let node = self.node()?;
                        if self.connection.rebind.is_some() {
                            return self.finish_rebind(node, session);
                        }
                        self.connected(&node, session)?;
                        return Ok(Task::batch([self.poll(), self.attach_workspace()?]));
                    }
                    Err(e) => {
                        self.secret.zeroize();
                        self.notice = e
                    }
                }
            }
            Message::Tick if self.desk.expired() => {
                return self.desktop(desk::Action::Manage(management::Event::Lock));
            }
            Message::Tick => {
                let refresh = self.refresh_management();
                return Ok(Task::batch([self.poll(), refresh]));
            }
            Message::Sample(epoch, r) if epoch == self.epoch => {
                self.sampling = false;
                match r {
                    Ok(s) => {
                        let id = self.node_id.unwrap();
                        let db = self.store.as_mut().unwrap();
                        db.save(id, &s)?;
                        self.history = db.history(id)?;
                        self.metrics_error.clear();
                    }
                    Err(e) => self.metrics_error = e,
                }
            }
            Message::Open if self.input.is_none() => {
                anyhow::ensure!(self.terminal_allowed, "本机已停用交互终端");
                if let Some(session) = self.session.clone() {
                    self.term_epoch += 1;
                    let epoch = self.term_epoch;
                    let (tx, rx) = mpsc::channel(64);
                    self.input = Some(tx);
                    let (c, r) = self.geometry;
                    self.parser = vt100::Parser::new(r, c, 400);
                    self.screen_cache.clear();
                    self.ready = false;
                    self.term_status = "正在打开终端…".into();
                    return Ok(terminal::run(session, c, r, rx, self.desk.container.take())
                        .map(move |e| Message::Term(epoch, e)));
                }
            }
            Message::Close => self.close_term(),
            Message::Focus if self.ready && self.paste.is_none() => {
                return Ok(operation::focus("terminal-input"));
            }
            Message::Term(epoch, event) if self.term_epoch == epoch => match event {
                Event::Open => {
                    self.ready = true;
                    self.term_status.clear();
                    let (c, r) = self.geometry;
                    self.send(Input::Size(c, r));
                    return Ok(operation::focus("terminal-input"));
                }
                Event::Bytes(bytes) => {
                    self.parser.process(&bytes);
                    self.screen_cache.clear();
                }
                Event::End(reason) => {
                    self.close_term();
                    self.term_status = reason;
                }
            },
            Message::Scroll(rows) => {
                let current = self.parser.screen().scrollback();
                self.parser
                    .screen_mut()
                    .set_scrollback(current.saturating_add_signed(rows as isize));
                self.screen_cache.clear();
            }
            Message::Geometry(c, r) => {
                self.geometry = (c, r);
                self.parser.screen_mut().set_size(r, c);
                self.screen_cache.clear();
                if self.ready {
                    self.send(Input::Size(c, r));
                }
            }
            Message::Text(s) if self.ready && self.paste.is_none() && !s.is_empty() => {
                if s.len() <= 4096 && !s.chars().any(char::is_control) {
                    self.send(Input::Bytes(s.into_bytes()));
                }
            }
            Message::Key(b) if self.ready && self.paste.is_none() => self.send(Input::Bytes(b)),
            Message::Copy => return Ok(iced::clipboard::write(self.parser.screen().contents())),
            Message::Paste if self.ready => {
                let epoch = self.term_epoch;
                return Ok(iced::clipboard::read().map(move |s| Message::Clipboard(epoch, s)));
            }
            Message::Clipboard(e, Some(v)) if e == self.term_epoch && self.ready => {
                let bytes = terminal::paste_bytes(&v, self.parser.screen().bracketed_paste())?;
                if v.contains(['\r', '\n', '\t']) {
                    self.paste = Some(v)
                } else {
                    self.send(Input::Bytes(bytes))
                }
            }
            Message::PasteConfirm => {
                if let Some(v) = self.paste.take() {
                    let bytes = terminal::paste_bytes(&v, self.parser.screen().bracketed_paste())?;
                    self.send(Input::Bytes(bytes));
                    return Ok(operation::focus("terminal-input"));
                }
            }
            Message::PasteCancel => {
                self.paste = None;
                return Ok(operation::focus("terminal-input"));
            }
            _ => {}
        }
        Ok(Task::none())
    }

    fn subscription(&self) -> Subscription<Message> {
        let closing = iced::window::close_requests().map(Message::CloseWindow);
        if self.session.is_none() && !self.desk.unlocked() && !self.desk.visible {
            return closing;
        }
        Subscription::batch([
            closing,
            iced::time::every(Duration::from_secs(5)).map(|_| Message::Tick),
        ])
    }
}
