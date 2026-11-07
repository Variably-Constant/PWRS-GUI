//! SVG path data, as a canvas's Path item and a Group's clip give it, read
//! into segments in absolute coordinates, or refused at the first
//! character that is not path data as SVG's grammar reads it; an arc's
//! center form as SVG's implementation notes find it, and the cubic pieces
//! that draw it; and the exact box of what segments draw under a plane
//! transform. egui builds its paths from these segments, and the helper
//! sends an HTML page the segments to draw, so both draw one path.

/// One step of a path, in absolute coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Starts a subpath at the point.
    Move([f64; 2]),
    /// A straight line to the point.
    Line([f64; 2]),
    /// A quadratic Bézier curve through the control point to the point.
    Quad([f64; 2], [f64; 2]),
    /// A cubic Bézier curve through the two control points to the point.
    Cubic([f64; 2], [f64; 2], [f64; 2]),
    /// An elliptical arc of the radii, its x axis turned `rotation`
    /// degrees, taking the large arc or the small one and sweeping
    /// clockwise or not, to the point.
    Arc { radii: [f64; 2], rotation: f64, large: bool, sweep: bool, to: [f64; 2] },
    /// Closes the subpath with a line back to its start.
    Close,
}

/// The command letters of SVG path data.
const COMMANDS: &[u8] = b"MmZzLlHhVvCcSsQqTtAa";

/// The segments `data` gives, or where it stops being path data.
pub fn segments(data: &str) -> Result<Vec<Segment>, String> {
    let mut r = Reader { text: data.as_bytes(), at: 0 };
    let mut out = Vec::new();
    let (mut current, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    // The control point a following S or T reflects, while the segment
    // before is a cubic or a quadratic curve.
    let (mut cubic, mut quad): (Option<[f64; 2]>, Option<[f64; 2]>) = (None, None);
    let mut previous: Option<u8> = None;
    r.space();
    if r.done() {
        return Err(String::from("it has no path data"));
    }
    while !r.done() {
        let command = match r.peek() {
            Some(c) if COMMANDS.contains(&c) => {
                if previous.is_none() && !matches!(c, b'M' | b'm') {
                    return Err(r.wrong("M or m, which starts a path"));
                }
                r.at += 1;
                c
            }
            // Numbers after a command are another of its argument groups,
            // and those after a moveto are linetos.
            Some(_) if r.number_next() => match previous {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(c) if c != b'Z' && c != b'z' => c,
                _ => return Err(r.wrong("a command letter")),
            },
            _ => return Err(r.wrong(if previous.is_none() { "M or m, which starts a path" } else { "a command letter or a number" })),
        };
        r.space();
        let relative = command.is_ascii_lowercase();
        let from = current;
        let place = |p: [f64; 2]| if relative { [from[0] + p[0], from[1] + p[1]] } else { p };
        let segment = match command.to_ascii_uppercase() {
            b'M' => {
                let p = place(r.pair()?);
                start = p;
                Segment::Move(p)
            }
            b'Z' => Segment::Close,
            b'L' => Segment::Line(place(r.pair()?)),
            b'H' => {
                let x = r.number()?;
                Segment::Line([if relative { from[0] + x } else { x }, from[1]])
            }
            b'V' => {
                let y = r.number()?;
                Segment::Line([from[0], if relative { from[1] + y } else { y }])
            }
            b'C' => {
                let first = place(r.pair()?);
                r.between();
                let second = place(r.pair()?);
                r.between();
                Segment::Cubic(first, second, place(r.pair()?))
            }
            b'S' => {
                let first = cubic.map_or(from, |c| reflect(c, from));
                let second = place(r.pair()?);
                r.between();
                Segment::Cubic(first, second, place(r.pair()?))
            }
            b'Q' => {
                let control = place(r.pair()?);
                r.between();
                Segment::Quad(control, place(r.pair()?))
            }
            b'T' => Segment::Quad(quad.map_or(from, |q| reflect(q, from)), place(r.pair()?)),
            _ => {
                let rx = r.number()?.abs();
                r.between();
                let ry = r.number()?.abs();
                r.between();
                let rotation = r.number()?;
                r.between();
                let large = r.flag()?;
                r.between();
                let sweep = r.flag()?;
                r.between();
                Segment::Arc { radii: [rx, ry], rotation, large, sweep, to: place(r.pair()?) }
            }
        };
        (cubic, quad) = match &segment {
            Segment::Cubic(_, second, _) => (Some(*second), None),
            Segment::Quad(control, _) => (None, Some(*control)),
            _ => (None, None),
        };
        current = match &segment {
            Segment::Move(p) | Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) | Segment::Arc { to: p, .. } => *p,
            Segment::Close => start,
        };
        out.push(segment);
        previous = Some(command);
        r.after_group()?;
    }
    Ok(out)
}

/// The point `control` reflected through `about`.
fn reflect(control: [f64; 2], about: [f64; 2]) -> [f64; 2] {
    [2.0 * about[0] - control[0], 2.0 * about[1] - control[1]]
}

/// An elliptical arc in center form: its ellipse's center and radii, its
/// x axis turned `rotation` radians, and the angle in radians the arc
/// starts at and how far it sweeps, positive angles turning clockwise as
/// the y axis grows downwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CenterArc {
    pub center: [f64; 2],
    pub radii: [f64; 2],
    pub rotation: f64,
    pub start: f64,
    pub sweep: f64,
}

impl CenterArc {
    /// The point of the arc's ellipse at `angle` radians.
    pub fn at(&self, angle: f64) -> [f64; 2] {
        let (s, c) = self.rotation.sin_cos();
        let (x, y) = (self.radii[0] * angle.cos(), self.radii[1] * angle.sin());
        [self.center[0] + c * x - s * y, self.center[1] + s * x + c * y]
    }

    /// Whether the arc passes through `angle` radians on its ellipse.
    fn passes(&self, angle: f64) -> bool {
        let tau = std::f64::consts::TAU;
        if self.sweep >= 0.0 { (angle - self.start).rem_euclid(tau) <= self.sweep } else { (self.start - angle).rem_euclid(tau) <= -self.sweep }
    }
}

/// The arc an Arc segment draws from `from` to `to`, in center form, as
/// SVG's implementation notes find it (F.6.5), radii too small to reach
/// `to` scaled up until they do (F.6.6); none where SVG draws no arc
/// (F.6.2): a straight line where a radius is zero, nothing where the two
/// ends are one point.
pub fn center_arc(from: [f64; 2], radii: [f64; 2], rotation: f64, large: bool, sweep: bool, to: [f64; 2]) -> Option<CenterArc> {
    let (mut rx, mut ry) = (radii[0].abs(), radii[1].abs());
    if from == to || rx == 0.0 || ry == 0.0 {
        return None;
    }
    let phi = rotation.to_radians();
    let (s, c) = phi.sin_cos();
    let (dx, dy) = ((from[0] - to[0]) / 2.0, (from[1] - to[1]) / 2.0);
    let (x1, y1) = (c * dx + s * dy, -s * dx + c * dy);
    let reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if reach > 1.0 {
        rx *= reach.sqrt();
        ry *= reach.sqrt();
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    let mut share = ((rx2 * ry2 - rx2 * y1 * y1 - ry2 * x1 * x1) / (rx2 * y1 * y1 + ry2 * x1 * x1)).max(0.0).sqrt();
    if large == sweep {
        share = -share;
    }
    let (cx1, cy1) = (share * rx * y1 / ry, -share * ry * x1 / rx);
    let center = [c * cx1 - s * cy1 + (from[0] + to[0]) / 2.0, s * cx1 + c * cy1 + (from[1] + to[1]) / 2.0];
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let mut turn = angle(ux, uy, vx, vy);
    if !sweep && turn > 0.0 {
        turn -= std::f64::consts::TAU;
    } else if sweep && turn < 0.0 {
        turn += std::f64::consts::TAU;
    }
    Some(CenterArc { center, radii: [rx, ry], rotation: phi, start: angle(1.0, 0.0, ux, uy), sweep: turn })
}

/// The cubic Bézier pieces that draw `arc`, each its two control points and
/// its end, none sweeping more than an eighth of a turn, so none strays
/// from the ellipse by more than 4.2e-6 of its larger radius.
pub fn arc_cubics(arc: &CenterArc) -> Vec<[[f64; 2]; 3]> {
    let pieces = (arc.sweep.abs() / std::f64::consts::FRAC_PI_4 - 1e-9).ceil().max(1.0);
    let step = arc.sweep / pieces;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let (s, c) = arc.rotation.sin_cos();
    let [rx, ry] = arc.radii;
    let place = |x: f64, y: f64| [arc.center[0] + c * x - s * y, arc.center[1] + s * x + c * y];
    (0..pieces as usize)
        .map(|i| {
            let (sa, ca) = (arc.start + step * i as f64).sin_cos();
            let (sb, cb) = (arc.start + step * (i + 1) as f64).sin_cos();
            [place(rx * (ca - k * sa), ry * (sa + k * ca)), place(rx * (cb + k * sb), ry * (sb - k * cb)), place(rx * cb, ry * sb)]
        })
        .collect()
}

/// `at` placed by `transform`, `[a, b, c, d, e, f]` making `x' = a x + c y
/// + e` and `y' = b x + d y + f`.
pub fn transformed(transform: [f64; 6], at: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d, e, f] = transform;
    [a * at[0] + c * at[1] + e, b * at[0] + d * at[1] + f]
}

/// The transform that leaves every point where it is.
pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The smallest box, `[x, y, width, height]`, around everything
/// `segments` draw once `transform` places them ([`transformed`]): each
/// line's ends, each curve's ends and its turning points between them, and
/// each arc's ends and the points of its ellipse it passes that lie
/// farthest each way; none when they draw nothing, as a lone move does.
pub fn bounds(segments: &[Segment], transform: [f64; 6]) -> Option<[f64; 4]> {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    let mut take = |p: [f64; 2]| {
        low = [low[0].min(p[0]), low[1].min(p[1])];
        high = [high[0].max(p[0]), high[1].max(p[1])];
    };
    let place = |p: [f64; 2]| transformed(transform, p);
    let (mut current, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    for segment in segments {
        let from = place(current);
        match segment {
            Segment::Move(p) => {
                (current, start) = (*p, *p);
                continue;
            }
            Segment::Line(p) => {
                take(from);
                take(place(*p));
            }
            Segment::Quad(control, p) => {
                let (q1, q2) = (place(*control), place(*p));
                take(from);
                take(q2);
                for axis in 0..2 {
                    let bend = from[axis] - 2.0 * q1[axis] + q2[axis];
                    if bend != 0.0 {
                        let t = (from[axis] - q1[axis]) / bend;
                        if t > 0.0 && t < 1.0 {
                            take(quad_at(from, q1, q2, t));
                        }
                    }
                }
            }
            Segment::Cubic(first, second, p) => {
                let (c1, c2, c3) = (place(*first), place(*second), place(*p));
                take(from);
                take(c3);
                for axis in 0..2 {
                    for t in cubic_turns(from[axis], c1[axis], c2[axis], c3[axis]) {
                        take(cubic_at(from, c1, c2, c3, t));
                    }
                }
            }
            Segment::Arc { radii, rotation, large, sweep, to } => {
                if current == *to {
                    continue;
                }
                take(from);
                take(place(*to));
                if let Some(arc) = center_arc(current, *radii, *rotation, *large, *sweep, *to) {
                    let [a, b, c, d, ..] = transform;
                    let (s, co) = arc.rotation.sin_cos();
                    let [rx, ry] = arc.radii;
                    let across = [(a * co + c * s) * rx, (-a * s + c * co) * ry];
                    let down = [(b * co + d * s) * rx, (-b * s + d * co) * ry];
                    for [u, v] in [across, down] {
                        let turn = v.atan2(u);
                        for angle in [turn, turn + std::f64::consts::PI] {
                            if arc.passes(angle) {
                                take(place(arc.at(angle)));
                            }
                        }
                    }
                }
            }
            Segment::Close => {
                take(from);
                take(place(start));
            }
        }
        current = match segment {
            Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) | Segment::Arc { to: p, .. } => *p,
            Segment::Close => start,
            Segment::Move(_) => current,
        };
    }
    (low[0] <= high[0]).then(|| [low[0], low[1], high[0] - low[0], high[1] - low[1]])
}

/// The subpaths of `segments` that end in a Close, in their order, each
/// starting with its Move, a subpath begun after a Close without one moving
/// from where the Close ended: the part of an outline with an inside
/// whether or not it is filled, as a canvas's hit test counts it.
pub fn closed(segments: &[Segment]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut open: Vec<Segment> = Vec::new();
    let (mut current, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    for segment in segments {
        match segment {
            Segment::Move(p) => {
                open = vec![Segment::Move(*p)];
                (current, start) = (*p, *p);
                continue;
            }
            Segment::Close => {
                if !open.is_empty() {
                    open.push(Segment::Close);
                    out.append(&mut open);
                }
                current = start;
                continue;
            }
            Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) | Segment::Arc { to: p, .. } => {
                if open.is_empty() {
                    open.push(Segment::Move(current));
                    start = current;
                }
                open.push(segment.clone());
                current = *p;
            }
        }
    }
    out
}

/// The quadratic Bézier from `p0` through `p1` to `p2` at `t`.
fn quad_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    [u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1]]
}

/// The cubic Bézier from `p0` through `p1` and `p2` to `p3` at `t`.
fn cubic_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let at = |i: usize| u * u * u * p0[i] + 3.0 * u * u * t * p1[i] + 3.0 * u * t * t * p2[i] + t * t * t * p3[i];
    [at(0), at(1)]
}

/// Where between its ends, `t` from 0 to 1 exclusive, a cubic Bézier of
/// one axis's values `p0` to `p3` turns back.
fn cubic_turns(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let roots = if a.abs() < 1e-12 * (b.abs() + c.abs()).max(1e-300) {
        if b == 0.0 { Vec::new() } else { vec![-c / b] }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            Vec::new()
        } else {
            let root = disc.sqrt();
            vec![(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
        }
    };
    roots.into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect()
}

/// Path data read from its start, `at` the byte it has reached.
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn done(&self) -> bool {
        self.at >= self.text.len()
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    /// Passes over white space, as SVG counts it.
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0C)) {
            self.at += 1;
        }
    }

    /// Passes over what may stand between two numbers: white space, with
    /// one comma among it.
    fn between(&mut self) {
        self.space();
        if self.peek() == Some(b',') {
            self.at += 1;
            self.space();
        }
    }

    /// Passes over what may follow an argument group: white space, and a
    /// comma only where another group follows it.
    fn after_group(&mut self) -> Result<(), String> {
        self.space();
        if self.peek() == Some(b',') {
            self.at += 1;
            self.space();
            if !self.number_next() {
                return Err(self.wrong("a number after the comma"));
            }
        }
        Ok(())
    }

    /// Whether a number starts here.
    fn number_next(&self) -> bool {
        matches!(self.peek(), Some(b'0'..=b'9' | b'.' | b'+' | b'-'))
    }

    fn digits(&mut self) -> usize {
        let from = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - from
    }

    /// A number as SVG writes one: a sign, digits with a decimal point
    /// among or before them, and an exponent.
    fn number(&mut self) -> Result<f64, String> {
        let from = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let mut count = self.digits();
        if self.peek() == Some(b'.') {
            self.at += 1;
            count += self.digits();
        }
        if count == 0 {
            self.at = from;
            return Err(self.wrong("a number"));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                self.at = mark;
            }
        }
        let written = std::str::from_utf8(&self.text[from..self.at]).map_err(|e| format!("the number at character {} is not text: {e}", self.position(from)))?;
        let n: f64 = written.parse().map_err(|e| format!("{written} at character {} is no number: {e}", self.position(from)))?;
        if n.is_finite() { Ok(n) } else { Err(format!("{written} at character {} is too large a number", self.position(from))) }
    }

    /// An x and a y.
    fn pair(&mut self) -> Result<[f64; 2], String> {
        let x = self.number()?;
        self.between();
        Ok([x, self.number()?])
    }

    /// An arc's flag, 0 or 1 alone.
    fn flag(&mut self) -> Result<bool, String> {
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.at += 1;
                Ok(true)
            }
            _ => Err(self.wrong("an arc's flag, 0 or 1")),
        }
    }

    /// The character `at` reaches, counted from 1.
    fn position(&self, at: usize) -> usize {
        String::from_utf8_lossy(&self.text[..at]).chars().count() + 1
    }

    /// Says what stands here where `expected` was wanted.
    fn wrong(&self, expected: &str) -> String {
        match String::from_utf8_lossy(&self.text[self.at..]).chars().next() {
            Some(c) => format!("at character {} it has '{c}' where it needs {expected}", self.position(self.at)),
            None => format!("it ends where it needs {expected}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(data: &str) -> Vec<Segment> {
        match segments(data) {
            Ok(s) => s,
            Err(why) => panic!("{data} reads: {why}"),
        }
    }

    #[test]
    fn path_data_reads_into_absolute_segments_with_implicit_linetos_and_reflected_controls() {
        assert_eq!(
            read("M10 10 20 10 l0,10 h-5 V30 z"),
            vec![Segment::Move([10.0, 10.0]), Segment::Line([20.0, 10.0]), Segment::Line([20.0, 20.0]), Segment::Line([15.0, 20.0]), Segment::Line([15.0, 30.0]), Segment::Close]
        );
        assert_eq!(
            read("M0,0 C 0,10 10,10 10,0 S 20,-10 20,0"),
            vec![Segment::Move([0.0, 0.0]), Segment::Cubic([0.0, 10.0], [10.0, 10.0], [10.0, 0.0]), Segment::Cubic([10.0, -10.0], [20.0, -10.0], [20.0, 0.0])],
            "S reflects the control point before it"
        );
        assert_eq!(read("M0 0Q5 5 10 0t10 0"), vec![Segment::Move([0.0, 0.0]), Segment::Quad([5.0, 5.0], [10.0, 0.0]), Segment::Quad([15.0, -5.0], [20.0, 0.0])]);
        assert_eq!(read("M0 0S5 5 10 0")[1], Segment::Cubic([0.0, 0.0], [5.0, 5.0], [10.0, 0.0]), "an S after no cubic takes the current point");
        assert_eq!(read("m1 1 2 2z m1 1")[3], Segment::Move([2.0, 2.0]), "a relative moveto after a close starts from the subpath's start");
        assert_eq!(read("M-1-2.5.5.5"), vec![Segment::Move([-1.0, -2.5]), Segment::Line([0.5, 0.5])], "a sign or a second point starts the next number");
    }

    #[test]
    fn numbers_and_arc_flags_need_no_separators_where_svg_needs_none() {
        assert_eq!(read("M.5.5L1e1-1E-1"), vec![Segment::Move([0.5, 0.5]), Segment::Line([10.0, -0.1])]);
        assert_eq!(
            read("M0 0a5,5 0 1110,0"),
            vec![Segment::Move([0.0, 0.0]), Segment::Arc { radii: [5.0, 5.0], rotation: 0.0, large: true, sweep: true, to: [10.0, 0.0] }],
            "flags are one character each"
        );
        assert_eq!(read("M0 0 A-3 4 30 0 0 6 8")[1], Segment::Arc { radii: [3.0, 4.0], rotation: 30.0, large: false, sweep: false, to: [6.0, 8.0] }, "a negative radius counts as its size");
    }

    fn near(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    fn boxed(data: &str, transform: [f64; 6]) -> [f64; 4] {
        match bounds(&read(data), transform) {
            Some(b) => b,
            None => panic!("{data} draws nothing"),
        }
    }

    #[test]
    fn an_arc_finds_its_center_as_svg_does_and_scales_radii_too_small_to_reach() {
        let Some(arc) = center_arc([0.0, 0.0], [5.0, 5.0], 0.0, false, true, [10.0, 0.0]) else {
            panic!("an arc");
        };
        assert!(near([arc.center[0], arc.center[1], arc.radii[0], arc.radii[1]], [5.0, 0.0, 5.0, 5.0]), "{arc:?}");
        assert!((arc.start - std::f64::consts::PI).abs() < 1e-12 && (arc.sweep - std::f64::consts::PI).abs() < 1e-12, "half a turn clockwise from the left: {arc:?}");
        let small = center_arc([0.0, 0.0], [1.0, 1.0], 0.0, false, true, [10.0, 0.0]);
        assert!(small.is_some_and(|a| (a.radii[0] - 5.0).abs() < 1e-12 && (a.radii[1] - 5.0).abs() < 1e-12), "radii too small to reach are scaled up: {small:?}");
        assert_eq!(center_arc([0.0, 0.0], [0.0, 5.0], 0.0, false, true, [10.0, 0.0]), None, "a zero radius draws a line");
        assert_eq!(center_arc([3.0, 3.0], [5.0, 5.0], 0.0, false, true, [3.0, 3.0]), None, "ends at one point draw nothing");
    }

    #[test]
    fn a_box_holds_what_the_segments_draw_and_no_control_point_beyond() {
        assert!(near(boxed("M0 0A5 5 0 0 1 10 0", IDENTITY), [0.0, -5.0, 10.0, 5.0]), "clockwise, the arc bulges up");
        assert!(near(boxed("M0 0A5 5 0 0 0 10 0", IDENTITY), [0.0, 0.0, 10.0, 5.0]), "the other way, down");
        assert!(near(boxed("M0 0C0 -100 100 -100 100 0", IDENTITY), [0.0, -75.0, 100.0, 75.0]), "a cubic's top, not its control points'");
        assert!(near(boxed("M0 0Q50 100 100 0", IDENTITY), [0.0, 0.0, 100.0, 50.0]));
        assert!(near(boxed("M0 0L10 0", [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]), [0.0, 0.0, 0.0, 10.0]), "a quarter turn clockwise points the line down");
        assert!(near(boxed("M0 0A5 5 0 0 1 10 0", [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]), [0.0, 0.0, 5.0, 10.0]), "turned, the arc bulges right");
        assert!(near(boxed("M0 0A10 5 0 0 1 20 0", [2.0, 0.0, 0.0, 3.0, 1.0, 1.0]), [1.0, -14.0, 40.0, 15.0]), "scaled and moved");
        assert!(near(boxed("M0 0A5 5 0 0 1 10 0M50 50", IDENTITY), [0.0, -5.0, 10.0, 5.0]), "a lone move draws nothing");
        assert_eq!(bounds(&read("M3 4"), IDENTITY), None);
    }

    #[test]
    fn the_closed_part_keeps_each_subpath_that_ends_in_a_close() {
        assert_eq!(closed(&read("M0 0 L10 0 L10 10 Z")), read("M0 0 L10 0 L10 10 Z"), "a closed outline is all its own inside");
        assert_eq!(closed(&read("M0 0 L10 0 L10 10")), Vec::new(), "an open one has none");
        assert_eq!(closed(&read("M0 0 L5 5 M20 0 L30 0 L30 10 Z M40 0 L50 0")), read("M20 0 L30 0 L30 10 Z"), "only the closed subpath of three");
        assert_eq!(
            closed(&read("M0 0 L10 0 L10 10 Z L0 20 L20 20 Z")),
            read("M0 0 L10 0 L10 10 Z M0 0 L0 20 L20 20 Z"),
            "a subpath begun after a close without a move starts where the close ended"
        );
        assert_eq!(closed(&read("M0 0 L10 0 Z Z")), read("M0 0 L10 0 Z"), "a second close adds nothing");
    }

    #[test]
    fn an_arcs_cubic_pieces_stay_on_its_ellipse() {
        let full = CenterArc { center: [0.0, 0.0], radii: [1.0, 1.0], rotation: 0.0, start: 0.0, sweep: std::f64::consts::TAU };
        let pieces = arc_cubics(&full);
        assert_eq!(pieces.len(), 8, "an eighth of a turn each");
        assert_eq!(arc_cubics(&CenterArc { sweep: std::f64::consts::PI, ..full }).len(), 4, "half a turn in four, not five");
        let mut from = [1.0, 0.0];
        for [c1, c2, to] in &pieces {
            for t in [0.25, 0.5, 0.75] {
                let at = cubic_at(from, *c1, *c2, *to, t);
                let off = (at[0].hypot(at[1]) - 1.0).abs();
                assert!(off < 4.3e-6, "{off} off the circle at {t} of a piece ending at {to:?}");
            }
            from = *to;
        }
        assert!((from[0] - 1.0).abs() < 1e-12 && from[1].abs() < 1e-12, "the last piece ends where the first began: {from:?}");
    }

    #[test]
    fn what_is_not_path_data_is_refused_where_it_stands() {
        let refused = |data: &str, want: &str| match segments(data) {
            Ok(s) => panic!("{data} read as {s:?}"),
            Err(why) => assert!(why.contains(want), "{data}: {why}"),
        };
        refused("", "has no path data");
        refused("   ", "has no path data");
        refused("L0 0", "at character 1 it has 'L' where it needs M or m");
        refused("M0 0 L5", "it ends where it needs a number");
        refused("M0 0 X5 5", "at character 6 it has 'X'");
        refused("M0 0 Z 5 5", "where it needs a command letter");
        refused("M0 0 A5 5 0 2 0 1 1", "an arc's flag, 0 or 1");
        refused("M0 0, L1 1", "a number after the comma");
        refused("M0 0 L1e 1", "at character 8 it has 'e'");
        refused("M0 0 L 1é", "at character 9 it has 'é'");
    }
}
