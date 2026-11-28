use crate::model::Sample;
use iced::{
    Color, Point, Rectangle, Renderer, Theme, mouse,
    widget::canvas::{self, Frame, Geometry, Path, Stroke},
};

pub const CPU: Color = Color::from_rgb(0.10, 0.43, 0.82);
pub const MEMORY: Color = Color::from_rgb(0.05, 0.56, 0.45);
pub const DISK: Color = Color::from_rgb(0.72, 0.40, 0.10);

pub trait PointData {
    fn time(&self) -> i64;
    fn series(&self) -> [Option<f64>; 3];
}
impl PointData for Sample {
    fn time(&self) -> i64 {
        self.at
    }
    fn series(&self) -> [Option<f64>; 3] {
        [self.cpu, self.memory, self.disk]
    }
}
pub struct Network {
    pub at: i64,
    pub rx: Option<f64>,
    pub tx: Option<f64>,
}
impl PointData for Network {
    fn time(&self) -> i64 {
        self.at
    }
    fn series(&self) -> [Option<f64>; 3] {
        [self.rx, self.tx, None]
    }
}
#[derive(Clone, Copy)]
pub struct Window {
    pub seconds: i64,
    pub bucket: i64,
}
impl Window {
    pub fn range(self, now: i64, last: i64) -> (i64, i64) {
        let end = now.max(last);
        (end - self.seconds, end)
    }
    pub fn adjacent(self, a: i64, b: i64) -> bool {
        self.bucket <= 0 || b.div_euclid(self.bucket) - a.div_euclid(self.bucket) <= 1
    }
}
pub struct Chart<'a, P>(pub &'a [P], pub i64, pub f64, pub Option<Window>);
impl<Message, P: PointData> canvas::Program<Message> for Chart<'_, P> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let w = f.width();
        let h = (f.height() - 8.).max(1.);
        for level in [0., 0.5, 1.] {
            let y = 4. + level * h;
            f.stroke(
                &Path::line(Point::new(0., y), Point::new(w, y)),
                Stroke::default().with_color(Color::from_rgb8(218, 228, 239)),
            );
        }
        if let (Some(first), Some(last)) = (self.0.first(), self.0.last()) {
            let (start, end) = self.3.map_or((first.time(), last.time()), |win| {
                win.range(chrono::Utc::now().timestamp(), last.time())
            });
            let span = (end - start).max(5) as f32;
            for (idx, color) in [CPU, MEMORY, DISK].into_iter().enumerate() {
                let mut previous: Option<(i64, Point)> = None;
                for s in self.0 {
                    if s.time() < start {
                        continue;
                    }
                    let Some(value) = s.series()[idx] else {
                        previous = None;
                        continue;
                    };
                    let p = Point::new(
                        ((s.time() - start) as f32 / span) * (w - 4.) + 2.,
                        4. + (1. - (value / self.2.max(1.0)) as f32) * h,
                    );
                    if let Some((time, old)) = previous
                        && s.time() - time <= self.1
                        && self.3.is_none_or(|win| win.adjacent(time, s.time()))
                    {
                        f.stroke(
                            &Path::line(old, p),
                            Stroke::default().with_width(2.).with_color(color),
                        );
                    } else {
                        f.fill(&Path::circle(p, 2.), color);
                    }
                    previous = Some((s.time(), p));
                }
            }
        }
        vec![f.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::Window;
    #[test]
    fn preserves_the_requested_window_and_breaks_across_missing_buckets() {
        let window = Window {
            seconds: 3600,
            bucket: 60,
        };
        assert_eq!(window.range(4000, 3000), (400, 4000));
        assert!(window.adjacent(61, 179));
        assert!(!window.adjacent(119, 180));
    }
}
