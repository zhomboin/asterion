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
pub struct Chart<'a, P>(pub &'a [P], pub i64, pub f64);
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
            let span = (last.time() - first.time()).max(5) as f32;
            for (idx, color) in [CPU, MEMORY, DISK].into_iter().enumerate() {
                let mut previous: Option<(i64, Point)> = None;
                for s in self.0 {
                    let Some(value) = s.series()[idx] else {
                        previous = None;
                        continue;
                    };
                    let p = Point::new(
                        ((s.time() - first.time()) as f32 / span) * (w - 4.) + 2.,
                        4. + (1. - (value / self.2.max(1.0)) as f32) * h,
                    );
                    if let Some((time, old)) = previous
                        && s.time() - time <= self.1
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
