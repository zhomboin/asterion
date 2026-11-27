use crate::Message;
use iced::{
    Color, Font, Point, Rectangle, Renderer, Size, Theme, mouse,
    widget::canvas::{self, Geometry, Text},
};

const CELL: f32 = 9.0;
const LINE: f32 = 19.0;
const PAD: f32 = 8.0;

pub struct Screen<'a>(pub &'a vt100::Screen, pub &'a canvas::Cache);
impl canvas::Program<Message> for Screen<'_> {
    type State = (u16, u16);
    fn update(
        &self,
        state: &mut Self::State,
        _: &iced::Event,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let size = (
            ((bounds.width - 2. * PAD) / CELL).floor().clamp(10., 350.) as u16,
            ((bounds.height - 2. * PAD) / LINE).floor().clamp(2., 120.) as u16,
        );
        if size == *state {
            return None;
        }
        *state = size;
        Some(canvas::Action::publish(Message::Geometry(size.0, size.1)))
    }
    fn draw(
        &self,
        _: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        vec![self.1.draw(renderer, bounds.size(), |f| {
            f.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb8(20, 32, 48));
            let (rows, cols) = self.0.size();
            for r in 0..rows {
                for c in 0..cols {
                    let Some(cell) = self.0.cell(r, c) else {
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut fg = color(cell.fgcolor(), Color::from_rgb8(225, 234, 244));
                    let mut bg = color(cell.bgcolor(), Color::from_rgb8(20, 32, 48));
                    if cell.inverse() {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    let pos = Point::new(PAD + f32::from(c) * CELL, PAD + f32::from(r) * LINE);
                    if cell.inverse() || cell.bgcolor() != vt100::Color::Default {
                        f.fill_rectangle(
                            pos,
                            Size::new(if cell.is_wide() { 2. * CELL } else { CELL }, LINE),
                            bg,
                        );
                    }
                    if cell.has_contents() {
                        f.fill_text(Text {
                            content: cell.contents().to_string(),
                            position: pos,
                            color: fg,
                            size: 15.into(),
                            font: Font::MONOSPACE,
                            shaping: iced::advanced::text::Shaping::Advanced,
                            ..Default::default()
                        });
                    }
                }
            }
            if !self.0.hide_cursor() {
                let (r, c) = self.0.cursor_position();
                f.fill_rectangle(
                    Point::new(
                        PAD + f32::from(c) * CELL,
                        PAD + f32::from(r) * LINE + LINE - 2.,
                    ),
                    Size::new(CELL, 2.),
                    Color::WHITE,
                );
            }
        })]
    }
}

fn color(value: vt100::Color, default: Color) -> Color {
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
        vt100::Color::Idx(n) => {
            const BASE: [[u8; 3]; 16] = [
                [24, 31, 40],
                [210, 74, 75],
                [73, 172, 114],
                [218, 178, 73],
                [76, 142, 220],
                [170, 111, 201],
                [75, 178, 190],
                [207, 216, 225],
                [99, 109, 120],
                [249, 103, 100],
                [100, 216, 149],
                [245, 208, 110],
                [122, 178, 255],
                [207, 148, 242],
                [124, 222, 227],
                [250, 250, 250],
            ];
            let [r, g, b] = match n {
                0..=15 => BASE[n as usize],
                16..=231 => {
                    let v = n - 16;
                    let scale = |i: u8| if i == 0 { 0 } else { 55 + i * 40 };
                    [scale(v / 36), scale(v / 6 % 6), scale(v % 6)]
                }
                _ => [8 + (n - 232) * 10; 3],
            };
            Color::from_rgb8(r, g, b)
        }
    }
}
