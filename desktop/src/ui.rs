use crate::{
    App, Message, Method,
    chart::{self, Chart},
    screen::Screen,
};
use iced::{
    Alignment, Color, Element, Length,
    widget::{
        button, canvas, column, container, pick_list, row, scrollable, space, text, text_input,
    },
};

const INK: Color = Color::from_rgb(0.12, 0.18, 0.25);
const MUTED: Color = Color::from_rgb(0.33, 0.39, 0.46);
const ERROR: Color = Color::from_rgb(0.68, 0.16, 0.16);

fn heading(label: &str) -> iced::widget::Text<'_> {
    text(label).size(24).color(INK)
}
fn label(s: &str) -> iced::widget::Text<'_> {
    text(s).size(14).color(MUTED)
}

pub(super) fn view(app: &App) -> Element<'_, Message> {
    if app.closing.is_some() {
        return container(
            column![
                text("关闭窗口并放弃未保存的修改？").size(20),
                row![
                    button("继续编辑").on_press(Message::KeepWindow),
                    button("放弃并关闭").on_press(Message::DiscardWindow)
                ]
                .spacing(10)
            ]
            .spacing(18),
        )
        .center(Length::Fill)
        .into();
    }
    if app.desk.visible {
        return crate::desk::view(app);
    }
    let connected = app.session.is_some();
    let offline = app.session.as_ref().is_some_and(|s| s.is_closed());
    let header = row![
        text("Asterion").size(25).color(INK),
        text("节点工作台").size(16).color(MUTED),
        space::horizontal(),
        button("管理工作区").on_press_maybe(
            (!app.busy && app.verify.is_none()).then_some(Message::Desk(crate::desk::Action::Show))
        ),
        text(if offline {
            "SSH 已断开"
        } else if connected {
            "SSH 已连接"
        } else {
            "本地工作区"
        })
        .size(14)
        .color(if offline {
            ERROR
        } else if connected {
            chart::MEMORY
        } else {
            MUTED
        })
    ]
    .align_y(Alignment::Center)
    .spacing(16);
    let body = if connected { detail(app) } else { connect(app) };
    let mut page = column![header, iced::widget::rule::horizontal(1), body]
        .spacing(14)
        .padding(20);
    if !app.notice.is_empty() {
        page = page.push(text(&app.notice).size(14).color(ERROR));
    }
    container(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn connect(app: &App) -> Element<'_, Message> {
    let mut form = column![heading("连接 Linux 节点")]
        .spacing(12)
        .max_width(640);
    if let Some(fp) = &app.verify {
        form = form
            .push(text(app.node().map(|n| n.to_string()).unwrap_or_default()))
            .push(label("首次连接，请与服务器管理员核对主机指纹。"))
            .push(text(fp).size(16))
            .push(
                row![
                    button("信任并连接").on_press(Message::Trust),
                    button("取消")
                        .on_press(Message::Cancel)
                        .style(button::secondary)
                ]
                .spacing(10),
            );
    } else if app.busy {
        form = form
            .push(text("正在连接…"))
            .push(button("取消连接").on_press(Message::Cancel));
    } else {
        if !app.nodes.is_empty() {
            form = form.push(label("已保存节点")).push(
                pick_list(
                    app.nodes.as_slice(),
                    None::<crate::model::Node>,
                    Message::Select,
                )
                .placeholder("选择节点")
                .width(Length::Fill),
            );
        }
        form = form.push(
            row![
                column![
                    label("主机地址"),
                    text_input("域名或 IP", &app.host)
                        .on_input(Message::Host)
                        .padding(9)
                ]
                .spacing(4)
                .width(Length::Fill),
                column![
                    label("端口"),
                    text_input("端口", &app.port)
                        .on_input(Message::Port)
                        .padding(9)
                ]
                .spacing(4)
                .width(96)
            ]
            .spacing(12),
        );
        form = form.push(
            column![
                label("SSH 用户名"),
                text_input("用户名", &app.user)
                    .on_input(Message::User)
                    .padding(9)
            ]
            .spacing(4),
        );
        let method_button = |title, method| {
            button(text(title))
                .style(if app.method == method {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::Method(method))
        };
        form = form.push(label("认证方式")).push(
            row![
                method_button("密码", Method::Password),
                method_button("私钥", Method::Key)
            ]
            .spacing(6),
        );
        if app.method == Method::Key {
            form = form.push(
                column![
                    label("私钥文件"),
                    text_input("本机私钥的完整路径", &app.key_path)
                        .on_input(Message::KeyPath)
                        .padding(9)
                ]
                .spacing(4),
            );
        }
        let prompt = if app.method == Method::Key {
            "私钥口令（可留空）"
        } else {
            "SSH 密码"
        };
        form = form
            .push(
                column![
                    label(prompt),
                    text_input(prompt, &app.secret)
                        .secure(true)
                        .on_input(Message::Secret)
                        .on_submit(Message::Connect)
                        .padding(9)
                ]
                .spacing(4),
            )
            .push(
                button("连接节点")
                    .padding([8, 18])
                    .on_press_maybe(app.store.is_some().then_some(Message::Connect)),
            );
    }
    scrollable(container(form).padding([10, 16]))
        .height(Length::Fill)
        .into()
}

fn detail(app: &App) -> Element<'_, Message> {
    let heading = row![
        heading("节点详情"),
        text(app.node().map(|n| n.to_string()).unwrap_or_default()).size(17),
        space::horizontal(),
        button("断开连接")
            .style(button::secondary)
            .on_press(Message::Cancel)
    ]
    .align_y(Alignment::Center)
    .spacing(14);
    let last = app.history.last();
    let metric = |title: &'static str, v: Option<f64>, c| {
        column![
            label(title),
            text(v.map(|n| format!("{n:.1}%")).unwrap_or_else(|| "—".into()))
                .size(26)
                .color(c)
        ]
        .spacing(2)
        .width(Length::Fill)
    };
    let cards = row![
        metric("CPU", last.and_then(|s| s.cpu), chart::CPU),
        metric("内存", last.and_then(|s| s.memory), chart::MEMORY),
        metric("根分区", last.and_then(|s| s.disk), chart::DISK)
    ];
    let mut page = column![heading, cards].spacing(8);
    if !app.metrics_error.is_empty() {
        page = page.push(text(&app.metrics_error).color(ERROR).size(13));
    }
    let trend = match (app.history.first(), last) {
        (Some(first), Some(last)) => format!(
            "最近 {} 个采样 · 跨度 {} 秒",
            app.history.len(),
            last.at - first.at
        ),
        _ => "等待首次采样".into(),
    };
    page = page
        .push(row![
            text("使用率趋势").size(17),
            space::horizontal(),
            text(trend).size(13).color(MUTED)
        ])
        .push(
            canvas(Chart(&app.history, 15, 100.))
                .width(Length::Fill)
                .height(80),
        )
        .push(iced::widget::rule::horizontal(1));
    let mut bar = row![
        text("SSH 终端").size(18),
        text(&app.term_status).size(13).color(MUTED),
        space::horizontal()
    ]
    .align_y(Alignment::Center)
    .spacing(8);
    if app.input.is_some() {
        bar = bar
            .push(
                button("复制屏幕")
                    .style(button::secondary)
                    .on_press(Message::Copy),
            )
            .push(
                button("粘贴")
                    .style(button::secondary)
                    .on_press_maybe(app.ready.then_some(Message::Paste)),
            )
            .push(button("关闭终端").on_press(Message::Close));
    } else {
        bar = bar.push(button("打开终端").on_press(Message::Open));
    }
    page = page.push(bar);
    if app.input.is_some() || !app.term_status.is_empty() {
        page = page.push(
            iced::widget::mouse_area(
                canvas(Screen(app.parser.screen(), &app.screen_cache))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Focus),
        );
    }
    if let Some(v) = &app.paste {
        page = page
            .push(label("粘贴包含换行或制表符，可能执行命令。"))
            .push(scrollable(text(v).size(14)).height(58))
            .push(
                row![
                    button("确认粘贴").on_press(Message::PasteConfirm),
                    button("取消")
                        .style(button::secondary)
                        .on_press(Message::PasteCancel)
                ]
                .spacing(8),
            );
    } else if app.ready {
        let epoch = app.term_epoch;
        page = page.push(
            iced::widget::text_editor(&app.editor)
                .id("terminal-input")
                .placeholder("终端输入")
                .on_action(move |action| crate::keys::edit(epoch, action))
                .key_binding(crate::keys::binding)
                .height(42)
                .padding(8),
        );
    }
    page.height(Length::Fill).into()
}
