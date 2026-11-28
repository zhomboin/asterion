use crate::Message;
use iced::{
    keyboard::{Key, key::Named},
    widget::text_editor::{Action, Binding, Edit, KeyPress, Status},
};

pub fn edit(epoch: u64, action: Action) -> Message {
    match action {
        Action::Edit(Edit::Insert(c)) => Message::Text(c.to_string()),
        Action::Edit(Edit::Paste(s)) => Message::Clipboard(epoch, Some(s.to_string())),
        _ => Message::Text(String::new()),
    }
}

pub fn binding(press: KeyPress) -> Option<Binding<Message>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    let key = &press.key;
    let modifiers = press.modifiers;
    let bytes = match key.as_ref() {
        Key::Named(n) => match n {
            Named::Space => b" ",
            Named::Enter => b"\r".as_slice(),
            Named::Backspace => b"\x7f",
            Named::Tab if modifiers.shift() => return Binding::from_key_press(press),
            Named::Tab => b"\t",
            Named::Escape => b"\x1b",
            Named::ArrowUp => b"\x1b[A",
            Named::ArrowDown => b"\x1b[B",
            Named::ArrowRight => b"\x1b[C",
            Named::ArrowLeft => b"\x1b[D",
            Named::Home => b"\x1b[H",
            Named::End => b"\x1b[F",
            Named::PageUp => b"\x1b[5~",
            Named::PageDown => b"\x1b[6~",
            Named::Insert => b"\x1b[2~",
            Named::F1 => b"\x1bOP",
            Named::F2 => b"\x1bOQ",
            Named::F3 => b"\x1bOR",
            Named::F4 => b"\x1bOS",
            Named::F5 => b"\x1b[15~",
            Named::F6 => b"\x1b[17~",
            Named::F7 => b"\x1b[18~",
            Named::F8 => b"\x1b[19~",
            Named::F9 => b"\x1b[20~",
            Named::F10 => b"\x1b[21~",
            Named::F11 => b"\x1b[23~",
            Named::F12 => b"\x1b[24~",
            Named::Delete => b"\x1b[3~",
            _ => return Binding::from_key_press(press),
        }
        .to_vec(),
        Key::Character(s) if modifiers.control() && !modifiers.alt() && s.len() == 1 => {
            let ch = s.as_bytes()[0].to_ascii_lowercase();
            if ch == b'v' {
                return Some(Binding::Custom(Message::Paste));
            }
            if !ch.is_ascii_lowercase() && !b"@[\\]^_".contains(&ch) {
                return None;
            }
            vec![ch.to_ascii_uppercase() & 31]
        }
        Key::Character(s) if modifiers.alt() && !modifiers.control() && s.chars().count() == 1 => {
            [vec![0x1b], s.as_bytes().to_vec()].concat()
        }
        _ => return Binding::from_key_press(press),
    };
    Some(Binding::Custom(Message::Key(bytes)))
}
