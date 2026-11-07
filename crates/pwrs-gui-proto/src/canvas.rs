//! A canvas's scene as both renderers draw it: the items a `Canvas` node's
//! `items` gives, each a map of its kind, its name and its properties under
//! the lowercase names of the keys a script writes, a group's items under
//! `children`, read into [`Item`]s with every kind, key and value checked;
//! the item a change names, found by that name; and an item read again
//! with a change's properties. A refusal names the item, and the key as a
//! script writes it.

use crate::svg_path::{self, Segment};
use crate::{Node, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

/// What an item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Rect,
    RoundedRect,
    Ellipse,
    Line,
    Polyline,
    Polygon,
    Path,
    Text,
    Image,
    Group,
}

impl ItemKind {
    /// Every kind, in the order an error naming them lists them.
    pub const ALL: [ItemKind; 10] = [
        ItemKind::Rect,
        ItemKind::RoundedRect,
        ItemKind::Ellipse,
        ItemKind::Line,
        ItemKind::Polyline,
        ItemKind::Polygon,
        ItemKind::Path,
        ItemKind::Text,
        ItemKind::Image,
        ItemKind::Group,
    ];

    /// The kind's name, as a script writes it.
    pub fn name(self) -> &'static str {
        match self {
            ItemKind::Rect => "Rect",
            ItemKind::RoundedRect => "RoundedRect",
            ItemKind::Ellipse => "Ellipse",
            ItemKind::Line => "Line",
            ItemKind::Polyline => "Polyline",
            ItemKind::Polygon => "Polygon",
            ItemKind::Path => "Path",
            ItemKind::Text => "Text",
            ItemKind::Image => "Image",
            ItemKind::Group => "Group",
        }
    }

    /// The kind's name after its article, as a refusal says it.
    pub fn article(self) -> String {
        match self {
            ItemKind::Ellipse | ItemKind::Image => format!("an {}", self.name()),
            _ => format!("a {}", self.name()),
        }
    }

    /// The kind named `written`, compared without regard to case.
    pub fn named(written: &str) -> Result<ItemKind, String> {
        ItemKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(written)).ok_or_else(|| {
            let names: Vec<&str> = ItemKind::ALL.iter().map(|k| k.name()).collect();
            format!("{written} is no item kind; it is one of {}", names.join(", "))
        })
    }

    /// The geometry keys the kind needs and those it takes besides, by
    /// their lowercase names. An Image's picture comes as `image`, which
    /// the module makes of its `Path` or `Bytes`, as the Image widget's.
    fn geometry(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            ItemKind::Rect | ItemKind::Ellipse => (&["x", "y", "width", "height"], &[]),
            ItemKind::RoundedRect => (&["x", "y", "width", "height", "radius"], &[]),
            ItemKind::Line => (&["x1", "y1", "x2", "y2"], &[]),
            ItemKind::Polyline | ItemKind::Polygon => (&["points"], &[]),
            ItemKind::Path => (&["data"], &[]),
            ItemKind::Text => (&["x", "y", "text"], &["size"]),
            ItemKind::Image => (&["x", "y", "width", "height", "image"], &[]),
            ItemKind::Group => (&[], &["x", "y", "clip"]),
        }
    }
}

/// The keys every item takes besides its geometry, by their lowercase
/// names.
const STYLE: [&str; 14] = ["fill", "stroke", "strokewidth", "dash", "cap", "join", "opacity", "rotate", "scale", "skew", "origin", "visible", "order", "animate"];

/// Each lowercase key with the name a script writes it by.
const SHOWN: [(&str, &str); 45] = [
    ("x", "X"),
    ("y", "Y"),
    ("width", "Width"),
    ("height", "Height"),
    ("radius", "Radius"),
    ("x1", "X1"),
    ("y1", "Y1"),
    ("x2", "X2"),
    ("y2", "Y2"),
    ("points", "Points"),
    ("data", "Data"),
    ("text", "Text"),
    ("size", "Size"),
    ("image", "Path or Bytes"),
    ("path", "Path"),
    ("bytes", "Bytes"),
    ("clip", "Clip"),
    ("children", "Children"),
    ("fill", "Fill"),
    ("stroke", "Stroke"),
    ("strokewidth", "StrokeWidth"),
    ("dash", "Dash"),
    ("cap", "Cap"),
    ("join", "Join"),
    ("opacity", "Opacity"),
    ("rotate", "Rotate"),
    ("scale", "Scale"),
    ("skew", "Skew"),
    ("origin", "Origin"),
    ("visible", "Visible"),
    ("order", "Order"),
    ("animate", "Animate"),
    ("linear", "Linear"),
    ("radial", "Radial"),
    ("from", "From"),
    ("to", "To"),
    ("center", "Center"),
    ("stops", "Stops"),
    ("property", "Property"),
    ("duration", "Duration"),
    ("easing", "Easing"),
    ("repeat", "Repeat"),
    ("reverse", "Reverse"),
    ("kind", "Kind"),
    ("name", "Name"),
];

/// The name a script writes `key` by.
fn shown(key: &str) -> &str {
    SHOWN.iter().find(|(k, _)| *k == key).map_or(key, |(_, s)| s)
}

/// `v` as a refusal shows it.
fn written(v: &Value) -> String {
    match v {
        Value::Null => String::from("nothing"),
        Value::Bool(b) => format!("${b}"),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => format!("'{s}'"),
        Value::List(items) => format!("a list of {}", items.len()),
        Value::Map(_) => String::from("a hashtable"),
        Value::Bytes(b) => format!("{} bytes", b.len()),
    }
}

/// A color's red, green, blue and alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// The color `written` names, without regard to case: one of CSS's
    /// named colors, `transparent`, or `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(written: &str) -> Result<Rgba, String> {
        let given = written.trim();
        let refused = || format!("'{written}' is no color; a color is one of CSS's names, such as Tomato, Transparent, or #rgb, #rrggbb or #rrggbbaa");
        if let Some(digits) = given.strip_prefix('#') {
            let nibbles = digits.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<Vec<u8>>>().ok_or_else(refused)?;
            let byte = |high: u8, low: u8| (high << 4) | low;
            return match nibbles.as_slice() {
                [r, g, b] => Ok(Rgba([byte(*r, *r), byte(*g, *g), byte(*b, *b), 255])),
                [r1, r2, g1, g2, b1, b2] => Ok(Rgba([byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), 255])),
                [r1, r2, g1, g2, b1, b2, a1, a2] => Ok(Rgba([byte(*r1, *r2), byte(*g1, *g2), byte(*b1, *b2), byte(*a1, *a2)])),
                _ => Err(refused()),
            };
        }
        if given.eq_ignore_ascii_case("transparent") {
            return Ok(Rgba([0, 0, 0, 0]));
        }
        NAMED.iter().find(|(name, _)| name.eq_ignore_ascii_case(given)).map(|(_, rgb)| Rgba([(rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8, 255])).ok_or_else(refused)
    }

    /// The color in the protocol's form, `#rrggbbaa` in lower case.
    pub fn hex(self) -> String {
        let [r, g, b, a] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// CSS's named colors, as CSS Color Module Level 4 lists them.
const NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

/// What fills an item: one color, or a gradient whose colors stand at
/// their stops, from 0 to 1, along a line from `from` to `to` or out from
/// `center` to `radius`, each in fractions of the item's box.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Color(Rgba),
    Linear { colors: Vec<Rgba>, stops: Vec<f64>, from: [f64; 2], to: [f64; 2] },
    Radial { colors: Vec<Rgba>, stops: Vec<f64>, center: [f64; 2], radius: f64 },
}

/// How a stroke's open ends are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

/// How a stroke's corners are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// An item's look and placing: its fill, its stroke's color, width, dash,
/// caps and joins, each none when the item leaves it to its group; its
/// opacity; its turn and skew in degrees clockwise and its scale, about
/// its pivot, in fractions of its box, when it gives one; whether it is
/// drawn, and its order among the items beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub fill: Option<Paint>,
    pub stroke: Option<Rgba>,
    pub stroke_width: Option<f64>,
    pub dash: Option<Vec<f64>>,
    pub cap: Option<Cap>,
    pub join: Option<Join>,
    pub opacity: f64,
    pub rotate: f64,
    pub scale: [f64; 2],
    pub skew: [f64; 2],
    pub origin: Option<[f64; 2]>,
    pub visible: bool,
    pub order: i64,
}

impl Default for Style {
    fn default() -> Style {
        Style { fill: None, stroke: None, stroke_width: None, dash: None, cap: None, join: None, opacity: 1.0, rotate: 0.0, scale: [1.0, 1.0], skew: [0.0, 0.0], origin: None, visible: true, order: 0 }
    }
}

/// A box: its left, its top, its width and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A picture's bytes and their format, as the Image widget's.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub bytes: Arc<[u8]>,
    pub format: String,
}

/// What an item draws, in its own coordinates: a box, a rounded box with
/// its corners' radius, an ellipse in a box, a line between two points,
/// points joined open or closed, a path's segments, text with its top left
/// and size, a picture in a box, or a group's offset and the shape it
/// clips its items to.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(Bounds),
    RoundedRect(Bounds, f64),
    Ellipse(Bounds),
    Line([f64; 2], [f64; 2]),
    Polyline(Vec<[f64; 2]>),
    Polygon(Vec<[f64; 2]>),
    Path(Vec<Segment>),
    Text { at: [f64; 2], text: String, size: Option<f64> },
    Image(Bounds, Picture),
    Group { at: [f64; 2], clip: Option<Box<Shape>> },
}

impl Shape {
    /// The outline the shape draws, as segments in its own coordinates,
    /// each subpath turning clockwise as the y axis grows downwards: a box
    /// from its top left; a rounded box from the top of its left side, its
    /// corners quarter circles of the radius, at most half the box's width
    /// and height; an ellipse from its right end, as two half arcs, or the
    /// line it flattens to without width or height; a line, points joined,
    /// closed for a Polygon; and a path's own segments. None for Text, an
    /// Image and a Group, which have no outline.
    pub fn outline(&self) -> Option<Vec<Segment>> {
        let rect = |b: &Bounds| vec![Segment::Move([b.x, b.y]), Segment::Line([b.x + b.width, b.y]), Segment::Line([b.x + b.width, b.y + b.height]), Segment::Line([b.x, b.y + b.height]), Segment::Close];
        Some(match self {
            Shape::Rect(b) => rect(b),
            Shape::RoundedRect(b, radius) => {
                let r = radius.min(b.width / 2.0).min(b.height / 2.0);
                if r <= 0.0 {
                    return Some(rect(b));
                }
                let corner = |to: [f64; 2]| Segment::Arc { radii: [r, r], rotation: 0.0, large: false, sweep: true, to };
                let (right, bottom) = (b.x + b.width, b.y + b.height);
                vec![
                    Segment::Move([b.x, b.y + r]),
                    corner([b.x + r, b.y]),
                    Segment::Line([right - r, b.y]),
                    corner([right, b.y + r]),
                    Segment::Line([right, bottom - r]),
                    corner([right - r, bottom]),
                    Segment::Line([b.x + r, bottom]),
                    corner([b.x, bottom - r]),
                    Segment::Close,
                ]
            }
            Shape::Ellipse(b) => {
                if b.width == 0.0 || b.height == 0.0 {
                    return Some(vec![Segment::Move([b.x, b.y]), Segment::Line([b.x + b.width, b.y + b.height]), Segment::Close]);
                }
                let cy = b.y + b.height / 2.0;
                let half = |to: [f64; 2]| Segment::Arc { radii: [b.width / 2.0, b.height / 2.0], rotation: 0.0, large: false, sweep: true, to };
                vec![Segment::Move([b.x + b.width, cy]), half([b.x, cy]), half([b.x + b.width, cy]), Segment::Close]
            }
            Shape::Line(a, b) => vec![Segment::Move(*a), Segment::Line(*b)],
            Shape::Polyline(points) | Shape::Polygon(points) => {
                let mut out: Vec<Segment> = points.iter().enumerate().map(|(i, p)| if i == 0 { Segment::Move(*p) } else { Segment::Line(*p) }).collect();
                if matches!(self, Shape::Polygon(_)) {
                    out.push(Segment::Close);
                }
                out
            }
            Shape::Path(segments) => segments.clone(),
            Shape::Text { .. } | Shape::Image(..) | Shape::Group { .. } => return None,
        })
    }
}

macro_rules! easings {
    ($($name:ident),* $(,)?) => {
        /// The curve an animation moves along: CSS's five, then Penner's,
        /// as easings.net names them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub enum Easing {
            $($name),*
        }

        impl Easing {
            /// Every curve.
            pub const ALL: &'static [Easing] = &[$(Easing::$name),*];

            /// The curve's name, as a script writes it.
            pub fn name(self) -> &'static str {
                match self {
                    $(Easing::$name => stringify!($name)),*
                }
            }
        }
    };
}

easings!(
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    EaseInSine,
    EaseOutSine,
    EaseInOutSine,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInQuart,
    EaseOutQuart,
    EaseInOutQuart,
    EaseInQuint,
    EaseOutQuint,
    EaseInOutQuint,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    EaseInCirc,
    EaseOutCirc,
    EaseInOutCirc,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    EaseInElastic,
    EaseOutElastic,
    EaseInOutElastic,
    EaseInBounce,
    EaseOutBounce,
    EaseInOutBounce,
);

impl Easing {
    /// The curve named `written`, compared without regard to case.
    pub fn named(written: &str) -> Result<Easing, String> {
        Easing::ALL.iter().copied().find(|e| e.name().eq_ignore_ascii_case(written.trim())).ok_or_else(|| {
            format!(
                "'{written}' is no easing; it is Linear, Ease, EaseIn, EaseOut or EaseInOut, or one of Penner's curves, EaseIn, EaseOut or EaseInOut followed by Sine, Quad, Cubic, Quart, Quint, Expo, Circ, Back, Elastic or Bounce, such as EaseInOutCubic"
            )
        })
    }

    /// How far along the curve is `t` of the way through, `t` from 0 to 1:
    /// 0 at 0 and 1 at 1. CSS's curves are its cubic Béziers; Penner's are
    /// easings.net's formulas, whose Back and Elastic curves go past 0 and
    /// 1 between the ends. The page's script defines the same curves.
    pub fn at(self, t: f64) -> f64 {
        use std::f64::consts::PI;
        let t = t.clamp(0.0, 1.0);
        let back = 1.70158;
        let elastic = 2.0 * PI / 3.0;
        match self {
            Easing::Linear => t,
            Easing::Ease => bezier(0.25, 0.1, 0.25, 1.0, t),
            Easing::EaseIn => bezier(0.42, 0.0, 1.0, 1.0, t),
            Easing::EaseOut => bezier(0.0, 0.0, 0.58, 1.0, t),
            Easing::EaseInOut => bezier(0.42, 0.0, 0.58, 1.0, t),
            Easing::EaseInSine => 1.0 - (t * PI / 2.0).cos(),
            Easing::EaseOutSine => (t * PI / 2.0).sin(),
            Easing::EaseInOutSine => -((PI * t).cos() - 1.0) / 2.0,
            Easing::EaseInQuad => t.powi(2),
            Easing::EaseOutQuad => 1.0 - (1.0 - t).powi(2),
            Easing::EaseInOutQuad => in_out(t, 2),
            Easing::EaseInCubic => t.powi(3),
            Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOutCubic => in_out(t, 3),
            Easing::EaseInQuart => t.powi(4),
            Easing::EaseOutQuart => 1.0 - (1.0 - t).powi(4),
            Easing::EaseInOutQuart => in_out(t, 4),
            Easing::EaseInQuint => t.powi(5),
            Easing::EaseOutQuint => 1.0 - (1.0 - t).powi(5),
            Easing::EaseInOutQuint => in_out(t, 5),
            Easing::EaseInExpo if t == 0.0 => 0.0,
            Easing::EaseInExpo => 2f64.powf(10.0 * t - 10.0),
            Easing::EaseOutExpo if t == 1.0 => 1.0,
            Easing::EaseOutExpo => 1.0 - 2f64.powf(-10.0 * t),
            Easing::EaseInOutExpo if t == 0.0 || t == 1.0 => t,
            Easing::EaseInOutExpo if t < 0.5 => 2f64.powf(20.0 * t - 10.0) / 2.0,
            Easing::EaseInOutExpo => (2.0 - 2f64.powf(-20.0 * t + 10.0)) / 2.0,
            Easing::EaseInCirc => 1.0 - (1.0 - t.powi(2)).sqrt(),
            Easing::EaseOutCirc => (1.0 - (t - 1.0).powi(2)).sqrt(),
            Easing::EaseInOutCirc if t < 0.5 => (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0,
            Easing::EaseInOutCirc => ((1.0 - (-2.0 * t + 2.0).powi(2)).sqrt() + 1.0) / 2.0,
            Easing::EaseInBack => (back + 1.0) * t.powi(3) - back * t.powi(2),
            Easing::EaseOutBack => 1.0 + (back + 1.0) * (t - 1.0).powi(3) + back * (t - 1.0).powi(2),
            Easing::EaseInOutBack => {
                let c = back * 1.525;
                if t < 0.5 { (2.0 * t).powi(2) * ((c + 1.0) * 2.0 * t - c) / 2.0 } else { ((2.0 * t - 2.0).powi(2) * ((c + 1.0) * (t * 2.0 - 2.0) + c) + 2.0) / 2.0 }
            }
            Easing::EaseInElastic if t == 0.0 || t == 1.0 => t,
            Easing::EaseInElastic => -(2f64.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * elastic).sin(),
            Easing::EaseOutElastic if t == 0.0 || t == 1.0 => t,
            Easing::EaseOutElastic => 2f64.powf(-10.0 * t) * ((t * 10.0 - 0.75) * elastic).sin() + 1.0,
            Easing::EaseInOutElastic if t == 0.0 || t == 1.0 => t,
            Easing::EaseInOutElastic => {
                let c = 2.0 * PI / 4.5;
                if t < 0.5 { -(2f64.powf(20.0 * t - 10.0) * ((20.0 * t - 11.125) * c).sin()) / 2.0 } else { 2f64.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * c).sin() / 2.0 + 1.0 }
            }
            Easing::EaseInBounce => 1.0 - bounce(1.0 - t),
            Easing::EaseOutBounce => bounce(t),
            Easing::EaseInOutBounce if t < 0.5 => (1.0 - bounce(1.0 - 2.0 * t)) / 2.0,
            Easing::EaseInOutBounce => (1.0 + bounce(2.0 * t - 1.0)) / 2.0,
        }
    }
}

/// Penner's in-out curve of power `n` at `t`.
fn in_out(t: f64, n: i32) -> f64 {
    if t < 0.5 { 2f64.powi(n - 1) * t.powi(n) } else { 1.0 - (-2.0 * t + 2.0).powi(n) / 2.0 }
}

/// easings.net's out-bounce at `t`.
fn bounce(t: f64) -> f64 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let u = t - 1.5 / d;
        n * u * u + 0.75
    } else if t < 2.5 / d {
        let u = t - 2.25 / d;
        n * u * u + 0.9375
    } else {
        let u = t - 2.625 / d;
        n * u * u + 0.984375
    }
}

/// The cubic Bézier from 0,0 to 1,1 through `x1`,`y1` and `x2`,`y2`, as
/// CSS's cubic-bezier() takes it, at `x`: its y where its x is `x`, found by
/// Newton's method from `x`, then by halving where that does not settle.
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let along = |a: f64, b: f64, s: f64| 3.0 * a * s * (1.0 - s).powi(2) + 3.0 * b * s * s * (1.0 - s) + s.powi(3);
    let slope = |a: f64, b: f64, s: f64| 3.0 * a * (1.0 - s).powi(2) + 6.0 * (b - a) * s * (1.0 - s) + 3.0 * (1.0 - b) * s * s;
    let mut s = x;
    for _ in 0..8 {
        let gap = along(x1, x2, s) - x;
        if gap.abs() < 1e-9 {
            return along(y1, y2, s);
        }
        let d = slope(x1, x2, s);
        if d.abs() < 1e-9 {
            break;
        }
        s -= gap / d;
    }
    let (mut low, mut high) = (0.0, 1.0);
    s = x;
    for _ in 0..64 {
        let at = along(x1, x2, s);
        if (at - x).abs() < 1e-9 {
            break;
        }
        if at < x {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    along(y1, y2, s)
}

/// Where an animation moves its property: a number, a pair, or a color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Number(f64),
    Pair([f64; 2]),
    Color(Rgba),
}

impl Target {
    /// The value `share` of the way from `self` to `to`, a share past 0 or
    /// 1 going beyond them as Back and Elastic curves do: numbers and pairs
    /// straight, colors in premultiplied form, as CSS blends them, each
    /// channel limited to 0 to 255. Values of two kinds give `to`.
    pub fn toward(self, to: Target, share: f64) -> Target {
        let mix = |a: f64, b: f64| a + (b - a) * share;
        match (self, to) {
            (Target::Number(a), Target::Number(b)) => Target::Number(mix(a, b)),
            (Target::Pair(a), Target::Pair(b)) => Target::Pair([mix(a[0], b[0]), mix(a[1], b[1])]),
            (Target::Color(Rgba(a)), Target::Color(Rgba(b))) => {
                let alpha = |c: [u8; 4]| f64::from(c[3]) / 255.0;
                let (aa, ba) = (alpha(a), alpha(b));
                let out_alpha = mix(aa, ba).clamp(0.0, 1.0);
                let channel = |i: usize| {
                    let premultiplied = mix(f64::from(a[i]) * aa, f64::from(b[i]) * ba);
                    if out_alpha > 0.0 { (premultiplied / out_alpha).round().clamp(0.0, 255.0) as u8 } else { 0 }
                };
                Target::Color(Rgba([channel(0), channel(1), channel(2), (out_alpha * 255.0).round() as u8]))
            }
            (_, to) => to,
        }
    }
}

/// The size, in points, of a Text that gives none.
pub const TEXT_SIZE: f64 = 14.0;

impl Item {
    /// The item's own value of `property`, a key an animation moves, by its
    /// lowercase name: a geometry number or a style key's value, the
    /// default where the item gives none (a Fill or Stroke it leaves to its
    /// group reads as transparent). `None` for a key the item's kind does
    /// not move, or a Fill that is a gradient.
    pub fn value_of(&self, property: &str) -> Option<Target> {
        let s = &self.style;
        let number = |n: f64| Some(Target::Number(n));
        let clear = Rgba([0, 0, 0, 0]);
        match (property, &self.shape) {
            ("fill", _) => match &s.fill {
                None => Some(Target::Color(clear)),
                Some(Paint::Color(c)) => Some(Target::Color(*c)),
                Some(_) => None,
            },
            ("stroke", _) => Some(Target::Color(s.stroke.unwrap_or(clear))),
            ("strokewidth", _) => number(s.stroke_width.unwrap_or(1.0)),
            ("opacity", _) => number(s.opacity),
            ("rotate", _) => number(s.rotate),
            ("scale", _) => Some(Target::Pair(s.scale)),
            ("skew", _) => Some(Target::Pair(s.skew)),
            ("origin", _) => Some(Target::Pair(s.origin.unwrap_or([0.5, 0.5]))),
            ("x", Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => number(b.x),
            ("y", Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => number(b.y),
            ("width", Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => number(b.width),
            ("height", Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => number(b.height),
            ("radius", Shape::RoundedRect(_, r)) => number(*r),
            ("x1", Shape::Line(a, _)) => number(a[0]),
            ("y1", Shape::Line(a, _)) => number(a[1]),
            ("x2", Shape::Line(_, b)) => number(b[0]),
            ("y2", Shape::Line(_, b)) => number(b[1]),
            ("x", Shape::Text { at, .. } | Shape::Group { at, .. }) => number(at[0]),
            ("y", Shape::Text { at, .. } | Shape::Group { at, .. }) => number(at[1]),
            ("size", Shape::Text { size, .. }) => number(size.unwrap_or(TEXT_SIZE)),
            _ => None,
        }
    }

    /// Sets the item's drawn `property`, as [`Item::value_of`] names it, to
    /// `value`; a value of the wrong kind for the property, and a property
    /// the kind does not move, change nothing. A size, width or opacity an
    /// easing takes past its bounds is set to the bound it passed.
    pub fn set_value(&mut self, property: &str, value: Target) {
        let s = &mut self.style;
        match (property, value, &mut self.shape) {
            ("fill", Target::Color(c), _) => s.fill = Some(Paint::Color(c)),
            ("stroke", Target::Color(c), _) => s.stroke = Some(c),
            ("strokewidth", Target::Number(n), _) => s.stroke_width = Some(n.max(0.0)),
            ("opacity", Target::Number(n), _) => s.opacity = n.clamp(0.0, 1.0),
            ("rotate", Target::Number(n), _) => s.rotate = n,
            ("scale", Target::Pair(p), _) => s.scale = p,
            ("skew", Target::Pair(p), _) => s.skew = p.map(|a| a.clamp(-89.9, 89.9)),
            ("origin", Target::Pair(p), _) => s.origin = Some(p),
            ("x", Target::Number(n), Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => b.x = n,
            ("y", Target::Number(n), Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => b.y = n,
            ("width", Target::Number(n), Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => b.width = n.max(0.0),
            ("height", Target::Number(n), Shape::Rect(b) | Shape::RoundedRect(b, _) | Shape::Ellipse(b) | Shape::Image(b, _)) => b.height = n.max(0.0),
            ("radius", Target::Number(n), Shape::RoundedRect(_, r)) => *r = n.max(0.0),
            ("x1", Target::Number(n), Shape::Line(a, _)) => a[0] = n,
            ("y1", Target::Number(n), Shape::Line(a, _)) => a[1] = n,
            ("x2", Target::Number(n), Shape::Line(_, b)) => b[0] = n,
            ("y2", Target::Number(n), Shape::Line(_, b)) => b[1] = n,
            ("x", Target::Number(n), Shape::Text { at, .. } | Shape::Group { at, .. }) => at[0] = n,
            ("y", Target::Number(n), Shape::Text { at, .. } | Shape::Group { at, .. }) => at[1] = n,
            ("size", Target::Number(n), Shape::Text { size, .. }) => *size = Some(n.max(f64::MIN_POSITIVE)),
            _ => {}
        }
    }
}

impl Animation {
    /// How long its passes take together, in seconds; `None` for one that
    /// repeats forever.
    pub fn length(&self) -> Option<f64> {
        let pass = if self.reverse { 2.0 * self.duration } else { self.duration };
        match self.repeat {
            Repeat::Forever => None,
            Repeat::Count(n) => Some(pass * n as f64),
        }
    }

    /// How far along the way from its start to `To` the property is,
    /// `elapsed` seconds after the animation began, its easing applied: a
    /// pass forward, then with Reverse the same pass played back. Once its
    /// passes are done it stays where the last ended: at `To`, or back at
    /// its start after a Reverse pass.
    pub fn share(&self, elapsed: f64) -> f64 {
        let elapsed = elapsed.max(0.0);
        if self.length().is_some_and(|l| elapsed >= l) {
            return if self.reverse { 0.0 } else { 1.0 };
        }
        let pass = if self.reverse { 2.0 * self.duration } else { self.duration };
        let into = elapsed % pass;
        let t = if into <= self.duration { into / self.duration } else { 1.0 - (into - self.duration) / self.duration };
        self.easing.at(t)
    }
}

/// How many passes an animation makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Count(u64),
    Forever,
}

/// One animation: the property it moves, by its lowercase name, where it
/// moves it, in how many seconds, along which curve, how many passes it
/// makes, and whether each pass runs back again.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub property: String,
    pub to: Target,
    pub duration: f64,
    pub easing: Easing,
    pub repeat: Repeat,
    pub reverse: bool,
}

/// The keys an animation takes, by their lowercase names.
const ANIMATE: [&str; 6] = ["property", "to", "duration", "easing", "repeat", "reverse"];

/// The style keys an animation moves.
const MOVED_STYLE: [&str; 8] = ["fill", "stroke", "strokewidth", "opacity", "rotate", "scale", "skew", "origin"];

/// One item: its name when it has one, what it draws, its look, the
/// animations it starts, a group's items in drawing order, and its
/// properties as the session gave them, which a change merges into.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: Option<String>,
    pub shape: Shape,
    pub style: Style,
    pub animations: Vec<Animation>,
    pub children: Vec<Item>,
    pub props: BTreeMap<String, Value>,
}

impl Item {
    /// What the item is.
    pub fn kind(&self) -> ItemKind {
        match &self.shape {
            Shape::Rect(_) => ItemKind::Rect,
            Shape::RoundedRect(..) => ItemKind::RoundedRect,
            Shape::Ellipse(_) => ItemKind::Ellipse,
            Shape::Line(..) => ItemKind::Line,
            Shape::Polyline(_) => ItemKind::Polyline,
            Shape::Polygon(_) => ItemKind::Polygon,
            Shape::Path(_) => ItemKind::Path,
            Shape::Text { .. } => ItemKind::Text,
            Shape::Image(..) => ItemKind::Image,
            Shape::Group { .. } => ItemKind::Group,
        }
    }

    /// The item with `changes` merged into its properties and read again,
    /// a null taking a property away. Its kind, name and items stay as they
    /// are: a change finds its item by name, and a group's items change
    /// through Add-GuiItem and Remove-GuiItem.
    pub fn changed(&self, changes: &BTreeMap<String, Value>) -> Result<Item, String> {
        let here = match &self.name {
            Some(n) => format!("{} {n}", self.kind().name()),
            None => self.kind().article(),
        };
        let mut props = self.props.clone();
        for (key, value) in changes {
            if matches!(key.as_str(), "kind" | "name" | "children") {
                return Err(format!("{here}: a change sets no {}; an item's kind and name never change, and a Group's items change through Add-GuiItem and Remove-GuiItem", shown(key)));
            }
            if matches!(value, Value::Null) {
                props.remove(key);
            } else {
                props.insert(key.clone(), value.clone());
            }
        }
        read(self.kind(), self.name.clone(), props, self.children.clone()).map_err(|e| format!("{here}: {e}"))
    }
}

/// A canvas's items in drawing order, the first drawn first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub items: Vec<Item>,
}

impl Scene {
    /// The scene in `node`'s `items`.
    pub fn of(node: &Node) -> Result<Scene, String> {
        match node.props.get("items") {
            None | Some(Value::Null) => Ok(Scene::default()),
            Some(items) => Scene::of_items(items),
        }
    }

    /// The scene a list of item maps gives, one map alone being a list of
    /// one, as PowerShell binds one value to an array parameter. A name
    /// given twice anywhere in the scene, without regard to case, is
    /// refused, so a change finds one item.
    pub fn of_items(items: &Value) -> Result<Scene, String> {
        let mut names = Vec::new();
        let items = values(items).into_iter().enumerate().map(|(i, v)| item_of(v, &format!("item {}", i + 1), &mut names)).collect::<Result<Vec<_>, _>>()?;
        Ok(Scene { items })
    }

    /// The item named `name`, compared without regard to case, at any depth.
    pub fn find(&self, name: &str) -> Option<&Item> {
        find_in(&self.items, name)
    }
}

/// The values in `v`: a list's items, or `v` alone.
fn values(v: &Value) -> Vec<&Value> {
    match v {
        Value::List(list) => list.iter().collect(),
        other => vec![other],
    }
}

/// The item map `v` gives, `place` naming it in an error until it has a
/// name of its own; its name and its children's join `names`.
fn item_of(v: &Value, place: &str, names: &mut Vec<String>) -> Result<Item, String> {
    let map = v.as_map().ok_or_else(|| format!("{place} is {}, not a hashtable of its Kind and keys", written(v)))?;
    let kind = match map.get("kind") {
        Some(Value::Str(k)) => ItemKind::named(k).map_err(|e| format!("{place}: {e}"))?,
        Some(other) => return Err(format!("{place}'s Kind is {}, not a kind's name", written(other))),
        None => return Err(format!("{place} names no Kind")),
    };
    let name = match map.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::Str(n)) if !n.is_empty() => Some(n.clone()),
        Some(other) => return Err(format!("{place}'s Name is {}, not text", written(other))),
    };
    let here = match &name {
        Some(n) => format!("{} {n}", kind.name()),
        None => format!("{place}, {}", kind.article()),
    };
    if let Some(n) = &name {
        if names.iter().any(|seen| seen.eq_ignore_ascii_case(n)) {
            return Err(format!("{here}: the name {n} is given twice, and a name finds one item"));
        }
        names.push(n.clone());
    }
    let children = match (kind, map.get("children")) {
        (_, None | Some(Value::Null)) => Vec::new(),
        (ItemKind::Group, Some(c)) => values(c).into_iter().enumerate().map(|(i, child)| item_of(child, &format!("{here}'s item {}", i + 1), names)).collect::<Result<_, _>>()?,
        (other, Some(_)) => return Err(format!("{here}: {} has no items; a Group does", other.article())),
    };
    let props = map.iter().filter(|(k, _)| !matches!(k.as_str(), "kind" | "name" | "children")).map(|(k, v)| (k.clone(), v.clone())).collect();
    read(kind, name, props, children).map_err(|e| format!("{here}: {e}"))
}

/// The item of `kind` its properties give, every key and value checked.
fn read(kind: ItemKind, name: Option<String>, props: BTreeMap<String, Value>, children: Vec<Item>) -> Result<Item, String> {
    let (required, optional) = kind.geometry();
    if let Some(key) = props.keys().find(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()) && !STYLE.contains(&k.as_str())) {
        let takes: Vec<&str> = required.iter().chain(optional).chain(STYLE.iter()).map(|k| shown(k)).collect();
        return Err(format!("{} takes no {}; it takes {}", kind.article(), shown(key), takes.join(", ")));
    }
    let keys = Keys(&props);
    let shape = shape_of(kind, &keys)?;
    let style = style_of(&keys)?;
    let animations = match keys.get("animate") {
        None => Vec::new(),
        Some(v) => animations_of(kind, v)?,
    };
    if animations.iter().any(|a| a.property == "fill") && matches!(style.fill, Some(Paint::Linear { .. } | Paint::Radial { .. })) {
        return Err(String::from("Animate moves Fill, which is a gradient; an animation moves a Fill that is a color"));
    }
    Ok(Item { name, shape, style, animations, children, props })
}

/// An item's properties, a null standing for none.
struct Keys<'a>(&'a BTreeMap<String, Value>);

impl Keys<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        match self.0.get(key) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v),
        }
    }

    fn need(&self, key: &str, kind: ItemKind) -> Result<&Value, String> {
        self.get(key).ok_or_else(|| format!("{} needs {}", kind.article(), shown(key)))
    }
}

/// The finite number in `v`, under `key`.
fn finite(key: &str, v: &Value) -> Result<f64, String> {
    v.as_number().filter(|n| n.is_finite()).ok_or_else(|| format!("{} is {}; it is a finite number", shown(key), written(v)))
}

fn not_negative(key: &str, v: &Value) -> Result<f64, String> {
    let n = finite(key, v)?;
    if n >= 0.0 { Ok(n) } else { Err(format!("{} is {n}; it is 0 or more", shown(key))) }
}

fn positive(key: &str, v: &Value) -> Result<f64, String> {
    let n = finite(key, v)?;
    if n > 0.0 { Ok(n) } else { Err(format!("{} is {n}; it is more than 0", shown(key))) }
}

fn fraction(key: &str, v: &Value) -> Result<f64, String> {
    let n = finite(key, v)?;
    if (0.0..=1.0).contains(&n) { Ok(n) } else { Err(format!("{} is {n}; it is from 0 to 1", shown(key))) }
}

/// A pair of finite numbers, x then y.
fn pair(key: &str, v: &Value) -> Result<[f64; 2], String> {
    let refused = || format!("{} is {}; it is a pair of finite numbers, such as 0,0", shown(key), written(v));
    match v.as_list() {
        Some([x, y]) => match (x.as_number().filter(|n| n.is_finite()), y.as_number().filter(|n| n.is_finite())) {
            (Some(x), Some(y)) => Ok([x, y]),
            _ => Err(refused()),
        },
        _ => Err(refused()),
    }
}

/// A pair, or one number that `one` makes a pair of.
fn number_or_pair(key: &str, v: &Value, one: fn(f64) -> [f64; 2]) -> Result<[f64; 2], String> {
    match v {
        Value::Int(_) | Value::Float(_) => finite(key, v).map(one),
        _ => pair(key, v),
    }
}

fn whole(key: &str, v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 => Ok(*f as i64),
        _ => Err(format!("{} is {}; it is a whole number", shown(key), written(v))),
    }
}

fn text<'a>(key: &str, v: &'a Value) -> Result<&'a str, String> {
    v.as_str().ok_or_else(|| format!("{} is {}; it is text", shown(key), written(v)))
}

fn color(key: &str, v: &Value) -> Result<Rgba, String> {
    match v {
        Value::Str(s) => Rgba::parse(s).map_err(|e| format!("{}: {e}", shown(key))),
        other => Err(format!("{} is {}; it is a color, such as 'Tomato' or '#ff6347'", shown(key), written(other))),
    }
}

/// What `kind` draws, from its geometry keys.
fn shape_of(kind: ItemKind, keys: &Keys) -> Result<Shape, String> {
    let number = |key: &str| keys.need(key, kind).and_then(|v| finite(key, v));
    let size = |key: &str| keys.need(key, kind).and_then(|v| not_negative(key, v));
    let offset = |key: &str| match keys.get(key) {
        None => Ok(0.0),
        Some(v) => finite(key, v),
    };
    let bounds = || -> Result<Bounds, String> { Ok(Bounds { x: number("x")?, y: number("y")?, width: size("width")?, height: size("height")? }) };
    Ok(match kind {
        ItemKind::Rect => Shape::Rect(bounds()?),
        ItemKind::RoundedRect => Shape::RoundedRect(bounds()?, size("radius")?),
        ItemKind::Ellipse => Shape::Ellipse(bounds()?),
        ItemKind::Line => Shape::Line([number("x1")?, number("y1")?], [number("x2")?, number("y2")?]),
        ItemKind::Polyline => Shape::Polyline(points(kind, keys.need("points", kind)?)?),
        ItemKind::Polygon => Shape::Polygon(points(kind, keys.need("points", kind)?)?),
        ItemKind::Path => Shape::Path(svg_path::segments(text("data", keys.need("data", kind)?)?).map_err(|e| format!("Data: {e}"))?),
        ItemKind::Text => Shape::Text {
            at: [number("x")?, number("y")?],
            text: text("text", keys.need("text", kind)?)?.to_string(),
            size: match keys.get("size") {
                None => None,
                Some(v) => Some(positive("size", v)?),
            },
        },
        ItemKind::Image => Shape::Image(bounds()?, picture(keys.need("image", kind)?)?),
        ItemKind::Group => Shape::Group {
            at: [offset("x")?, offset("y")?],
            clip: match keys.get("clip") {
                None => None,
                Some(v) => Some(Box::new(clip_of(v)?)),
            },
        },
    })
}

/// A Polyline's or a Polygon's points: one list of numbers, x then y.
fn points(kind: ItemKind, v: &Value) -> Result<Vec<[f64; 2]>, String> {
    let list = v.as_list().ok_or_else(|| format!("Points is {}; it is one list of numbers, x then y, such as 0,0, 40,0, 20,30", written(v)))?;
    let numbers = list
        .iter()
        .map(|n| n.as_number().filter(|n| n.is_finite()))
        .collect::<Option<Vec<f64>>>()
        .ok_or_else(|| String::from("Points contains a value that is not a finite number; it is one list of numbers, x then y, such as 0,0, 40,0, 20,30"))?;
    if numbers.len() % 2 != 0 {
        return Err(format!("Points has {} numbers; they are x and y pairs, so they come in twos", numbers.len()));
    }
    let fewest = if kind == ItemKind::Polygon { 3 } else { 2 };
    if numbers.len() / 2 < fewest {
        return Err(format!("{} has {} points; it needs {fewest} at least", kind.article(), numbers.len() / 2));
    }
    Ok(numbers.as_chunks::<2>().0.to_vec())
}

/// An Image's picture, the map of bytes and format the module makes of
/// its Path or Bytes.
fn picture(v: &Value) -> Result<Picture, String> {
    let refused = || format!("Path or Bytes gives {}, not a picture", written(v));
    let map = v.as_map().ok_or_else(refused)?;
    match (map.get("bytes"), map.get("format")) {
        (Some(Value::Bytes(bytes)), Some(Value::Str(format))) => Ok(Picture { bytes: bytes.clone(), format: format.clone() }),
        _ => Err(refused()),
    }
}

/// The shape a Group's Clip gives: a Rect, RoundedRect, Ellipse, Polygon
/// or Path, by its Kind and its geometry keys alone, in the group's own
/// coordinates.
fn clip_of(v: &Value) -> Result<Shape, String> {
    let map = v.as_map().ok_or_else(|| format!("Clip is {}; it is a shape's hashtable, such as @{{ Kind = 'Ellipse'; X = 0; Y = 0; Width = 80; Height = 80 }}", written(v)))?;
    let kind = match map.get("kind") {
        Some(Value::Str(k)) => ItemKind::named(k).map_err(|e| format!("Clip: {e}"))?,
        _ => return Err(String::from("Clip names no Kind")),
    };
    if !matches!(kind, ItemKind::Rect | ItemKind::RoundedRect | ItemKind::Ellipse | ItemKind::Polygon | ItemKind::Path) {
        return Err(format!("Clip is {}; a clip is a Rect, RoundedRect, Ellipse, Polygon or Path", kind.article()));
    }
    let (required, optional) = kind.geometry();
    let props: BTreeMap<String, Value> = map.iter().filter(|(k, _)| k.as_str() != "kind").map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(key) = props.keys().find(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str())) {
        let takes: Vec<&str> = required.iter().chain(optional).map(|k| shown(k)).collect();
        return Err(format!("Clip, {}, takes no {}; it takes {}", kind.article(), shown(key), takes.join(", ")));
    }
    shape_of(kind, &Keys(&props)).map_err(|e| format!("Clip: {e}"))
}

/// An item's look, from its style keys.
fn style_of(keys: &Keys) -> Result<Style, String> {
    let mut style = Style::default();
    if let Some(v) = keys.get("fill") {
        style.fill = Some(paint(v)?);
    }
    if let Some(v) = keys.get("stroke") {
        style.stroke = Some(color("stroke", v)?);
    }
    if let Some(v) = keys.get("strokewidth") {
        style.stroke_width = Some(not_negative("strokewidth", v)?);
    }
    if let Some(v) = keys.get("dash") {
        style.dash = Some(dash(v)?);
    }
    if let Some(v) = keys.get("cap") {
        style.cap = Some(cap(text("cap", v)?)?);
    }
    if let Some(v) = keys.get("join") {
        style.join = Some(join(text("join", v)?)?);
    }
    if let Some(v) = keys.get("opacity") {
        style.opacity = fraction("opacity", v)?;
    }
    if let Some(v) = keys.get("rotate") {
        style.rotate = finite("rotate", v)?;
    }
    if let Some(v) = keys.get("scale") {
        style.scale = number_or_pair("scale", v, |n| [n, n])?;
    }
    if let Some(v) = keys.get("skew") {
        style.skew = skew(v)?;
    }
    if let Some(v) = keys.get("origin") {
        style.origin = Some(pair("origin", v)?);
    }
    if let Some(v) = keys.get("visible") {
        style.visible = v.as_bool().ok_or_else(|| format!("Visible is {}; it is $true or $false", written(v)))?;
    }
    if let Some(v) = keys.get("order") {
        style.order = whole("order", v)?;
    }
    Ok(style)
}

/// A skew in degrees, x alone or x and y, each less than a right angle
/// either way.
fn skew(v: &Value) -> Result<[f64; 2], String> {
    let angles = number_or_pair("skew", v, |n| [n, 0.0])?;
    match angles.iter().find(|a| a.abs() >= 90.0) {
        Some(a) => Err(format!("Skew gives {a} degrees; a skew is less than 90 degrees either way")),
        None => Ok(angles),
    }
}

/// A stroke's dash, lengths of dash then gap repeated, none for a solid
/// stroke.
fn dash(v: &Value) -> Result<Vec<f64>, String> {
    let lengths = values(v)
        .into_iter()
        .map(|d| d.as_number().filter(|n| n.is_finite() && *n >= 0.0))
        .collect::<Option<Vec<f64>>>()
        .ok_or_else(|| format!("Dash is {}; it is lengths of 0 or more, dash then gap, such as 6,3", written(v)))?;
    if !lengths.is_empty() && lengths.iter().all(|d| *d == 0.0) {
        return Err(String::from("Dash's lengths are all 0, which would draw nothing; leave Dash out for a solid stroke"));
    }
    Ok(lengths)
}

fn cap(written: &str) -> Result<Cap, String> {
    match written.trim().to_ascii_lowercase().as_str() {
        "butt" => Ok(Cap::Butt),
        "round" => Ok(Cap::Round),
        "square" => Ok(Cap::Square),
        _ => Err(format!("Cap is '{written}'; it is Butt, Round or Square")),
    }
}

fn join(written: &str) -> Result<Join, String> {
    match written.trim().to_ascii_lowercase().as_str() {
        "miter" => Ok(Join::Miter),
        "round" => Ok(Join::Round),
        "bevel" => Ok(Join::Bevel),
        _ => Err(format!("Join is '{written}'; it is Miter, Round or Bevel")),
    }
}

/// A Fill: a color, or a gradient's hashtable.
fn paint(v: &Value) -> Result<Paint, String> {
    match v {
        Value::Str(s) => Rgba::parse(s).map(Paint::Color).map_err(|e| format!("Fill: {e}")),
        Value::Map(m) => gradient(m).map_err(|e| format!("Fill: {e}")),
        other => Err(format!("Fill is {}; it is a color, such as 'Tomato' or '#ff6347', or a gradient's hashtable", written(other))),
    }
}

/// A gradient: its colors under Linear or Radial, their Stops, and From
/// and To, or Center and Radius, in fractions of the item's box, which
/// default as SVG's do.
fn gradient(m: &BTreeMap<String, Value>) -> Result<Paint, String> {
    let (given, linear) = match (m.get("linear"), m.get("radial")) {
        (Some(c), None) => (c, true),
        (None, Some(c)) => (c, false),
        (Some(_), Some(_)) => return Err(String::from("a gradient is Linear or Radial, not both")),
        (None, None) => return Err(String::from("a gradient gives its colors under Linear or Radial")),
    };
    let (which, allowed) = if linear { ("linear", ["linear", "from", "to", "stops"]) } else { ("radial", ["radial", "center", "radius", "stops"]) };
    if let Some(key) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        let takes: Vec<&str> = allowed.iter().map(|k| shown(k)).collect();
        return Err(format!("a {} gradient takes no {}; it takes {}", shown(which), shown(key), takes.join(", ")));
    }
    let colors = values(given).into_iter().map(|c| color(which, c)).collect::<Result<Vec<Rgba>, String>>()?;
    if colors.len() < 2 {
        return Err(format!("{} gives {} color; a gradient has 2 at least", shown(which), colors.len()));
    }
    let stops = match m.get("stops") {
        None | Some(Value::Null) => (0..colors.len()).map(|i| i as f64 / (colors.len() - 1) as f64).collect(),
        Some(v) => stops(v, colors.len())?,
    };
    let point = |key: &str, default: [f64; 2]| match m.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => pair(key, v),
    };
    if linear {
        Ok(Paint::Linear { colors, stops, from: point("from", [0.0, 0.0])?, to: point("to", [1.0, 0.0])? })
    } else {
        let radius = match m.get("radius") {
            None | Some(Value::Null) => 0.5,
            Some(v) => positive("radius", v)?,
        };
        Ok(Paint::Radial { colors, stops, center: point("center", [0.5, 0.5])?, radius })
    }
}

/// A gradient's stops: one offset from 0 to 1 for each of its `count`
/// colors, none before the one ahead of it.
fn stops(v: &Value, count: usize) -> Result<Vec<f64>, String> {
    let offsets = values(v)
        .into_iter()
        .map(|s| s.as_number().filter(|n| (0.0..=1.0).contains(n)))
        .collect::<Option<Vec<f64>>>()
        .ok_or_else(|| format!("Stops is {}; it is an offset from 0 to 1 for each color", written(v)))?;
    if offsets.len() != count {
        let given = if offsets.len() == 1 { String::from("1 offset") } else { format!("{} offsets", offsets.len()) };
        return Err(format!("Stops has {given} for {count} colors; it has one for each"));
    }
    if offsets.windows(2).any(|w| w[1] < w[0]) {
        return Err(String::from("Stops goes back; each offset is at or past the one before it"));
    }
    Ok(offsets)
}

/// The keys, in lower case, that take one number on some kind of item, as a
/// column of a canvas's shared memory sets them: the geometry's numbers, and
/// the style's, Scale taking one number for both its parts and Skew for its
/// first.
pub const ONE_NUMBER: [&str; 16] = ["x", "y", "width", "height", "radius", "x1", "y1", "x2", "y2", "size", "strokewidth", "opacity", "rotate", "scale", "skew", "order"];

/// The name a script writes `key` by, for a refusal outside this module.
pub fn shown_key(key: &str) -> &str {
    shown(key)
}

/// Whether an item of `kind` moves `property`: its geometry's numbers, and
/// the style keys that are numbers, pairs or colors.
fn movable(kind: ItemKind, property: &str) -> bool {
    let (required, optional) = kind.geometry();
    MOVED_STYLE.contains(&property) || ((required.contains(&property) || optional.contains(&property)) && !matches!(property, "points" | "data" | "text" | "image" | "clip"))
}

/// The animations an Animate gives, one hashtable alone being a list of
/// one; each property moves in one of them at most.
fn animations_of(kind: ItemKind, v: &Value) -> Result<Vec<Animation>, String> {
    let mut out: Vec<Animation> = Vec::new();
    for a in values(v) {
        let animation = animation_of(kind, a).map_err(|e| format!("Animate: {e}"))?;
        if out.iter().any(|o| o.property == animation.property) {
            return Err(format!("Animate moves {} twice; a property moves one way at a time", shown(&animation.property)));
        }
        out.push(animation);
    }
    Ok(out)
}

fn animation_of(kind: ItemKind, v: &Value) -> Result<Animation, String> {
    let m = v.as_map().ok_or_else(|| format!("{} is no animation; an animation is a hashtable of Property, To, Duration, Easing, Repeat and Reverse", written(v)))?;
    if let Some(key) = m.keys().find(|k| !ANIMATE.contains(&k.as_str())) {
        return Err(format!("an animation takes no {}; it takes Property, To, Duration, Easing, Repeat and Reverse", shown(key)));
    }
    let property = match m.get("property") {
        Some(Value::Str(p)) => p.trim().to_ascii_lowercase(),
        Some(other) => return Err(format!("Property is {}; it names the key the animation moves, such as 'X'", written(other))),
        None => return Err(String::from("an animation names the key it moves in Property")),
    };
    if !movable(kind, &property) {
        let (required, optional) = kind.geometry();
        let moves: Vec<&str> = required.iter().chain(optional).chain(MOVED_STYLE.iter()).filter(|k| movable(kind, k)).map(|k| shown(k)).collect();
        return Err(format!("{} moves no {}; it moves {}", kind.article(), shown(&property), moves.join(", ")));
    }
    let to = match m.get("to") {
        None | Some(Value::Null) => return Err(format!("an animation of {} gives where it moves it in To", shown(&property))),
        Some(v) => target(&property, v).map_err(|e| format!("To: {e}"))?,
    };
    let duration = match m.get("duration") {
        None | Some(Value::Null) => return Err(String::from("an animation gives its seconds in Duration")),
        Some(v) => positive("duration", v)?,
    };
    let easing = match m.get("easing") {
        None | Some(Value::Null) => Easing::Ease,
        Some(v) => Easing::named(text("easing", v)?)?,
    };
    let repeat = match m.get("repeat") {
        None | Some(Value::Null) => Repeat::Count(1),
        Some(Value::Str(s)) if s.trim().eq_ignore_ascii_case("forever") => Repeat::Forever,
        Some(v @ (Value::Int(_) | Value::Float(_))) => match whole("repeat", v)? {
            n if n >= 1 => Repeat::Count(n as u64),
            n => return Err(format!("Repeat is {n}; it is a count of passes, 1 or more, or 'Forever'")),
        },
        Some(other) => return Err(format!("Repeat is {}; it is a count of passes, 1 or more, or 'Forever'", written(other))),
    };
    let reverse = match m.get("reverse") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(format!("Reverse is {}; it is $true or $false", written(other))),
    };
    Ok(Animation { property, to, duration, easing, repeat, reverse })
}

/// Where an animation moves `property`, checked as the property itself.
fn target(property: &str, v: &Value) -> Result<Target, String> {
    Ok(match property {
        "fill" | "stroke" => Target::Color(color(property, v)?),
        "opacity" => Target::Number(fraction(property, v)?),
        "strokewidth" | "width" | "height" | "radius" => Target::Number(not_negative(property, v)?),
        "size" => Target::Number(positive(property, v)?),
        "scale" => Target::Pair(number_or_pair(property, v, |n| [n, n])?),
        "skew" => Target::Pair(skew(v)?),
        "origin" => Target::Pair(pair(property, v)?),
        _ => Target::Number(finite(property, v)?),
    })
}

/// The item named `name` among `items` or their children, depth first.
fn find_in<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().find_map(|i| if i.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)) { Some(i) } else { find_in(&i.children, name) })
}

/// The id a [`Board`] gives each item on it, the same for as long as the
/// item is on the board.
pub type ItemId = u64;

/// One item on a [`Board`]: the item as read, its own items in drawing
/// order, the Group it is in, and its revision, which each change to it
/// moves on by one.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The item, its `children` empty: a Group's items are [`Entry::children`].
    pub item: Item,
    pub children: Vec<ItemId>,
    pub parent: Option<ItemId>,
    pub revision: u64,
}

/// A canvas's items, ready for change, each read once and given an id that
/// lasts as long as it is on the board, named items found by name without
/// regard to case: a change, an addition or a removal costs what it
/// touches, and copying the board costs a reference per item. Each refuses
/// what [`Scene::of_items`] would refuse of the items it leaves, and leaves
/// the board as it was when it refuses.
#[derive(Clone, Debug, Default)]
pub struct Board {
    entries: HashMap<ItemId, Arc<Entry>>,
    top: Vec<ItemId>,
    names: HashMap<Arc<str>, ItemId>,
    next: ItemId,
}

impl Board {
    /// The board a list of item maps gives, one map alone being a list of
    /// one, as [`Scene::of_items`] reads them.
    pub fn of_items(items: &Value) -> Result<Board, String> {
        let mut board = Board::default();
        if matches!(items, Value::Null) {
            return Ok(board);
        }
        for item in Scene::of_items(items)?.items {
            let id = board.put(item, None);
            board.top.push(id);
        }
        Ok(board)
    }

    /// The board `node`'s `items` give.
    pub fn of(node: &Node) -> Result<Board, String> {
        Board::of_items(node.props.get("items").unwrap_or(&Value::Null))
    }

    /// Puts `item` and its items on the board inside `parent`, and returns
    /// its id.
    fn put(&mut self, mut item: Item, parent: Option<ItemId>) -> ItemId {
        let id = self.next;
        self.next += 1;
        let children = std::mem::take(&mut item.children);
        if let Some(n) = &item.name {
            self.names.insert(Arc::from(n.to_ascii_lowercase()), id);
        }
        let children = children.into_iter().map(|c| self.put(c, Some(id))).collect();
        self.entries.insert(id, Arc::new(Entry { item, children, parent, revision: 0 }));
        id
    }

    /// The ids of the board's own items, in drawing order.
    pub fn top(&self) -> &[ItemId] {
        &self.top
    }

    pub fn entry(&self, id: ItemId) -> Option<&Entry> {
        self.entries.get(&id).map(Arc::as_ref)
    }

    /// The id of the item named `name`, compared without regard to case.
    pub fn find(&self, name: &str) -> Option<ItemId> {
        self.names.get(name.to_ascii_lowercase().as_str()).copied()
    }

    /// How many items are on the board, at every depth.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The ids of every item on the board, each followed by its own items',
    /// in drawing order.
    pub fn all(&self) -> Vec<ItemId> {
        let mut out = Vec::with_capacity(self.entries.len());
        for id in &self.top {
            self.under(*id, &mut out);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merges `props` into the item named `name`, a null taking a key away,
    /// and reads it again; returns its id. A change sets no Kind, Name or
    /// Children.
    pub fn set(&mut self, name: &str, props: &BTreeMap<String, Value>) -> Result<ItemId, String> {
        if let Some(key) = props.keys().find(|k| matches!(k.as_str(), "kind" | "name" | "children")) {
            return Err(format!(
                "{name}: a change sets no {}; an item's kind and name never change, and a Group's items change through Add-GuiItem and Remove-GuiItem",
                shown(key)
            ));
        }
        let id = self.find(name).ok_or_else(|| format!("no item is named {name}"))?;
        let entry = self.entries.get_mut(&id).ok_or_else(|| format!("no item is named {name}"))?;
        let changed = entry.item.changed(props)?;
        let entry = Arc::make_mut(entry);
        entry.item = changed;
        entry.revision += 1;
        Ok(id)
    }

    /// Adds `new`, item maps, above the board's own items, or with `group`
    /// above the items of the Group that name gives; returns the ids of the
    /// items added, in order, each followed by its own items'. A name
    /// already on the board is refused.
    pub fn add(&mut self, group: Option<&str>, new: &[Value]) -> Result<Vec<ItemId>, String> {
        let parent = match group {
            None => None,
            Some(g) => {
                let id = self.find(g).ok_or_else(|| format!("no item is named {g}"))?;
                if self.entry(id).map(|e| e.item.kind()) != Some(ItemKind::Group) {
                    return Err(format!("{g} is no Group; items go into a Group"));
                }
                Some(id)
            }
        };
        let mut names = Vec::new();
        let items = new.iter().enumerate().map(|(i, v)| item_of(v, &format!("item {}", i + 1), &mut names)).collect::<Result<Vec<_>, _>>()?;
        if let Some((kind, n)) = items.iter().find_map(|i| self.named_twice(i)) {
            return Err(format!("{} {n}: the name {n} is given twice, and a name finds one item", kind.name()));
        }
        let mut added = Vec::new();
        for item in items {
            let id = self.put(item, parent);
            match parent.and_then(|p| self.entries.get_mut(&p)) {
                Some(p) => Arc::make_mut(p).children.push(id),
                None => self.top.push(id),
            }
            self.under(id, &mut added);
        }
        Ok(added)
    }

    /// The kind and name of the first item among `item` and its items whose
    /// name is already on the board.
    fn named_twice(&self, item: &Item) -> Option<(ItemKind, String)> {
        if let Some(n) = &item.name
            && self.find(n).is_some()
        {
            return Some((item.kind(), n.clone()));
        }
        item.children.iter().find_map(|c| self.named_twice(c))
    }

    /// `id` and the ids of every item inside it, in drawing order.
    fn under(&self, id: ItemId, out: &mut Vec<ItemId>) {
        out.push(id);
        if let Some(e) = self.entry(id) {
            for c in &e.children {
                self.under(*c, out);
            }
        }
    }

    /// Takes the items `names` name out of the board, at any depth, a Group
    /// with its items; returns the ids of every item taken. A name no item
    /// has is refused, and nothing is taken.
    pub fn remove(&mut self, names: &[String]) -> Result<Vec<ItemId>, String> {
        let ids = names.iter().map(|n| self.find(n).ok_or_else(|| format!("no item is named {n}"))).collect::<Result<Vec<_>, _>>()?;
        let mut gone = Vec::new();
        for id in ids {
            let Some(parent) = self.entry(id).map(|e| e.parent) else {
                continue;
            };
            match parent.and_then(|p| self.entries.get_mut(&p)) {
                Some(p) => Arc::make_mut(p).children.retain(|c| *c != id),
                None => self.top.retain(|c| *c != id),
            }
            self.take(id, &mut gone);
        }
        Ok(gone)
    }

    /// Takes `id` and every item inside it off the board, their ids joining
    /// `gone`.
    fn take(&mut self, id: ItemId, gone: &mut Vec<ItemId>) {
        let Some(entry) = self.entries.remove(&id) else {
            return;
        };
        if let Some(n) = &entry.item.name {
            self.names.remove(n.to_ascii_lowercase().as_str());
        }
        gone.push(id);
        for c in &entry.children {
            self.take(*c, gone);
        }
    }

    /// The board's items as item maps, as a canvas's `items` lists them.
    pub fn to_value(&self) -> Value {
        Value::List(self.top.iter().filter_map(|id| self.map(*id)).collect())
    }

    /// The item map of `id` and its items, as a canvas's `items` lists it;
    /// none for an id not on the board.
    pub fn map(&self, id: ItemId) -> Option<Value> {
        let e = self.entry(id)?;
        let mut map = e.item.props.clone();
        map.insert(String::from("kind"), Value::Str(e.item.kind().name().to_string()));
        if let Some(n) = &e.item.name {
            map.insert(String::from("name"), Value::Str(n.clone()));
        }
        if e.item.kind() == ItemKind::Group {
            map.insert(String::from("children"), Value::List(e.children.iter().filter_map(|c| self.map(*c)).collect()));
        }
        Some(Value::Map(map))
    }
}

/// One animation of an item's property, under way or stopped at its end:
/// what it does, when it began, and the value it began from, none until a
/// frame reads it where the theme sets it.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub animation: Animation,
    pub began: Instant,
    pub from: Option<Target>,
}

impl Motion {
    /// Its value at `now`; none until a frame has read the value it began
    /// from.
    pub fn value(&self, now: Instant) -> Option<Target> {
        let from = self.from?;
        Some(from.toward(self.animation.to, self.animation.share(self.elapsed(now))))
    }

    /// Whether it is still moving at `now`, not yet stopped at its end.
    pub fn moving(&self, now: Instant) -> bool {
        self.animation.length().is_none_or(|l| self.elapsed(now) < l)
    }

    fn elapsed(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.began).as_secs_f64()
    }
}

/// The animations of a board's items, by item and by the property each
/// moves: started as items come onto the board with an Animate, and by a
/// change's Animate, which replaces only animations of the properties it
/// names; ended by a change that sets a property directly, and all at once
/// by an Animate of null.
#[derive(Clone, Debug, Default)]
pub struct Motions(HashMap<ItemId, BTreeMap<String, Motion>>);

impl Motions {
    /// The animations of `id`, by property.
    pub fn of(&self, id: ItemId) -> Option<&BTreeMap<String, Motion>> {
        self.0.get(&id)
    }

    pub fn of_mut(&mut self, id: ItemId) -> Option<&mut BTreeMap<String, Motion>> {
        self.0.get_mut(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.0.values().all(BTreeMap::is_empty)
    }

    /// Whether any animation is still moving at `now`.
    pub fn moving(&self, now: Instant) -> bool {
        self.0.values().flat_map(BTreeMap::values).any(|m| m.moving(now))
    }

    /// Starts the animations of the items `ids` name, as they come onto
    /// `board`, each from the value drawn at `now`.
    pub fn started(&mut self, board: &Board, ids: &[ItemId], now: Instant) {
        for id in ids {
            let Some(entry) = board.entry(*id) else {
                continue;
            };
            for animation in &entry.item.animations {
                self.start(board, *id, animation, now);
            }
        }
    }

    /// What the change `props` made to the item `id` of `board` does to
    /// its animations, at `now`: each property it sets directly ends its
    /// animation; an Animate of null ends them all, and any other starts
    /// each animation the item now gives, from the value drawn now.
    pub fn changed(&mut self, board: &Board, id: ItemId, props: &BTreeMap<String, Value>, now: Instant) {
        if let Some(own) = self.0.get_mut(&id) {
            for key in props.keys().filter(|k| k.as_str() != "animate") {
                own.remove(key);
            }
        }
        match props.get("animate") {
            None => {}
            Some(Value::Null) => {
                self.0.remove(&id);
            }
            Some(_) => {
                if let Some(entry) = board.entry(id) {
                    for animation in &entry.item.animations {
                        self.start(board, id, animation, now);
                    }
                }
            }
        }
    }

    fn start(&mut self, board: &Board, id: ItemId, animation: &Animation, now: Instant) {
        let from = self.drawn(board, id, &animation.property, now);
        self.0.entry(id).or_default().insert(animation.property.clone(), Motion { animation: animation.clone(), began: now, from });
    }

    /// Forgets the animations of `ids`, items gone from the board.
    pub fn forget(&mut self, ids: &[ItemId]) {
        for id in ids {
            self.0.remove(id);
        }
    }

    /// The value of `property` drawn on the item `id` at `now`: its
    /// animation's, else its own, else for a Fill, Stroke or StrokeWidth it
    /// leaves to its groups the nearest group's; none where only the theme
    /// sets it.
    pub fn drawn(&self, board: &Board, id: ItemId, property: &str, now: Instant) -> Option<Target> {
        if let Some(v) = self.0.get(&id).and_then(|m| m.get(property)).and_then(|m| m.value(now)) {
            return Some(v);
        }
        let entry = board.entry(id)?;
        let s = &entry.item.style;
        let left = match property {
            "fill" => s.fill.is_none(),
            "stroke" => s.stroke.is_none(),
            "strokewidth" => s.stroke_width.is_none(),
            _ => false,
        };
        if !left {
            return entry.item.value_of(property);
        }
        self.drawn(board, entry.parent?, property, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    fn text(s: &str) -> Value {
        Value::Str(s.into())
    }

    fn num(n: f64) -> Value {
        Value::Float(n)
    }

    fn list(items: &[Value]) -> Value {
        Value::List(items.to_vec())
    }

    fn rect(extra: &[(&str, Value)]) -> Value {
        let mut pairs = vec![("kind", text("Rect")), ("x", num(10.0)), ("y", num(20.0)), ("width", num(80.0)), ("height", num(40.0))];
        pairs.extend(extra.iter().cloned());
        map(&pairs)
    }

    fn one(item: &Value) -> Item {
        match Scene::of_items(item) {
            Ok(mut scene) => scene.items.remove(0),
            Err(why) => panic!("the item reads: {why}"),
        }
    }

    fn refusal(item: &Value) -> String {
        match Scene::of_items(item) {
            Ok(scene) => panic!("{item:?} read as {scene:?}"),
            Err(why) => why,
        }
    }

    #[test]
    fn a_scene_reads_its_items_and_a_group_s_children_and_finds_an_item_by_name() {
        let items = list(&[
            rect(&[("name", text("Box"))]),
            map(&[("kind", text("group")), ("x", num(5.0)), ("children", map(&[("kind", text("Ellipse")), ("name", text("Dot")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(4.0)), ("height", num(4.0))]))]),
        ]);
        let scene = match Scene::of_items(&items) {
            Ok(s) => s,
            Err(why) => panic!("the scene reads: {why}"),
        };
        assert_eq!(scene.items.len(), 2);
        assert_eq!(scene.items[0].shape, Shape::Rect(Bounds { x: 10.0, y: 20.0, width: 80.0, height: 40.0 }));
        assert_eq!(scene.items[1].kind(), ItemKind::Group, "a kind's name is read without regard to case");
        assert_eq!(scene.items[1].shape, Shape::Group { at: [5.0, 0.0], clip: None }, "a Group's Y is 0 when it gives none");
        assert!(!scene.items[0].props.contains_key("kind"), "the kind, name and children are not properties");
        assert_eq!(scene.items[1].children.len(), 1, "one map is a list of one");
        assert_eq!(scene.find("dot").map(Item::kind), Some(ItemKind::Ellipse), "a name is found at any depth without regard to case");
        assert!(scene.find("missing").is_none());
        assert_eq!(scene.items[0].style, Style::default(), "an item with no style keys leaves its fill and stroke to its group");
    }

    #[test]
    fn a_scene_refuses_an_unknown_kind_a_name_given_twice_and_items_outside_a_group() {
        let unknown = refusal(&map(&[("kind", text("Star"))]));
        assert!(unknown.contains("Star is no item kind") && unknown.contains("Rect, RoundedRect"), "{unknown}");
        let twice = list(&[rect(&[("name", text("A"))]), map(&[("kind", text("Group")), ("children", rect(&[("name", text("a"))]))])]);
        assert!(refusal(&twice).contains("given twice"));
        let held = refusal(&rect(&[("children", rect(&[]))]));
        assert!(held.contains("a Rect has no items"), "{held}");
        assert!(refusal(&map(&[("name", text("x"))])).contains("names no Kind"));
    }

    #[test]
    fn each_kind_takes_its_own_keys_and_needs_its_geometry() {
        let radius = refusal(&rect(&[("radius", num(4.0))]));
        assert!(radius.starts_with("item 1, a Rect: a Rect takes no Radius; it takes X, Y, Width, Height, Fill,"), "{radius}");
        assert!(refusal(&map(&[("kind", text("Ellipse")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(4.0))])).contains("an Ellipse needs Height"));
        assert!(refusal(&rect(&[("width", num(-1.0))])).contains("Width is -1; it is 0 or more"));
        assert!(refusal(&rect(&[("x", text("left"))])).contains("X is 'left'; it is a finite number"));
        let polygon = |points: &[f64]| map(&[("kind", text("Polygon")), ("points", list(&points.iter().map(|p| num(*p)).collect::<Vec<_>>()))]);
        assert_eq!(one(&polygon(&[0.0, 0.0, 4.0, 0.0, 2.0, 3.0])).shape, Shape::Polygon(vec![[0.0, 0.0], [4.0, 0.0], [2.0, 3.0]]));
        assert!(refusal(&polygon(&[0.0, 0.0, 4.0, 0.0, 2.0])).contains("Points has 5 numbers"));
        assert!(refusal(&polygon(&[0.0, 0.0, 4.0, 0.0])).contains("a Polygon has 2 points; it needs 3 at least"));
        let path = refusal(&map(&[("kind", text("Path")), ("name", text("Wave")), ("data", text("M0 0 Q5"))]));
        assert!(path.starts_with("Path Wave: Data: it ends where it needs a number"), "{path}");
        assert!(refusal(&map(&[("kind", text("Text")), ("x", num(0.0)), ("y", num(0.0)), ("text", text("hi")), ("size", num(0.0))])).contains("Size is 0; it is more than 0"));
        assert!(refusal(&map(&[("kind", text("Image")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(4.0)), ("height", num(4.0))])).contains("an Image needs Path or Bytes"));
        let picture = map(&[("bytes", Value::Bytes(Arc::from(vec![1u8, 2, 3]))), ("format", text("png"))]);
        let image = one(&map(&[("kind", text("Image")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(4.0)), ("height", num(4.0)), ("image", picture)]));
        assert!(matches!(&image.shape, Shape::Image(_, p) if p.format == "png" && p.bytes.len() == 3));
    }

    #[test]
    fn colors_are_css_names_or_hex_in_any_case() {
        assert_eq!(NAMED.len(), 148);
        let mut names: Vec<&str> = NAMED.iter().map(|(n, _)| *n).collect();
        names.dedup();
        assert_eq!(names.len(), 148, "the names are sorted and none comes twice");
        let parsed = |s: &str| match Rgba::parse(s) {
            Ok(c) => c.0,
            Err(why) => panic!("{s}: {why}"),
        };
        assert_eq!(parsed("Tomato"), [255, 99, 71, 255]);
        assert_eq!(parsed("REBECCAPURPLE"), [0x66, 0x33, 0x99, 255]);
        assert_eq!(parsed(" grey "), [128, 128, 128, 255]);
        assert_eq!(parsed("transparent"), [0, 0, 0, 0]);
        assert_eq!(parsed("#0F0"), [0, 255, 0, 255]);
        assert_eq!(parsed("#ff6347"), [255, 99, 71, 255]);
        assert_eq!(parsed("#FF634780"), [255, 99, 71, 128]);
        assert_eq!(Rgba([255, 99, 71, 128]).hex(), "#ff634780");
        for bad in ["#12345", "#ggg", "nocolor", "", "#"] {
            assert!(Rgba::parse(bad).is_err(), "{bad} is no color");
        }
        assert!(refusal(&rect(&[("stroke", num(3.0))])).contains("Stroke is 3; it is a color"));
        assert!(refusal(&rect(&[("fill", text("Tomatoe"))])).contains("Fill: 'Tomatoe' is no color"));
    }

    #[test]
    fn a_gradient_fill_reads_its_colors_stops_and_points_in_fractions_of_the_box() {
        let linear = one(&rect(&[("fill", map(&[("linear", list(&[text("Tomato"), text("Gold"), text("#00f")]))]))]));
        assert_eq!(
            linear.style.fill,
            Some(Paint::Linear { colors: vec![Rgba([255, 99, 71, 255]), Rgba([255, 215, 0, 255]), Rgba([0, 0, 255, 255])], stops: vec![0.0, 0.5, 1.0], from: [0.0, 0.0], to: [1.0, 0.0] }),
            "colors spread evenly along SVG's default line"
        );
        let radial = one(&rect(&[("fill", map(&[("radial", list(&[text("White"), text("Navy")])), ("stops", list(&[num(0.2), num(0.9)])), ("radius", num(0.25))]))]));
        assert_eq!(radial.style.fill, Some(Paint::Radial { colors: vec![Rgba([255, 255, 255, 255]), Rgba([0, 0, 128, 255])], stops: vec![0.2, 0.9], center: [0.5, 0.5], radius: 0.25 }));
        let refused = |fill: Value, want: &str| {
            let why = refusal(&rect(&[("fill", fill)]));
            assert!(why.contains(want), "{why}");
        };
        refused(map(&[("linear", list(&[text("Red")]))]), "Linear gives 1 color; a gradient has 2 at least");
        refused(map(&[("linear", list(&[text("Red"), text("Blue")])), ("radial", list(&[text("Red"), text("Blue")]))]), "Linear or Radial, not both");
        refused(map(&[("linear", list(&[text("Red"), text("Blue")])), ("center", list(&[num(0.0), num(0.0)]))]), "a Linear gradient takes no Center");
        refused(map(&[("linear", list(&[text("Red"), text("Blue")])), ("stops", list(&[num(0.5), num(0.2)]))]), "Stops goes back");
        refused(map(&[("linear", list(&[text("Red"), text("Blue")])), ("stops", list(&[num(0.0)]))]), "Stops has 1 offset for 2 colors");
        refused(map(&[("linear", list(&[text("Red"), text("Blue")])), ("stops", list(&[num(0.0), num(0.5), num(1.0)]))]), "Stops has 3 offsets for 2 colors");
        refused(map(&[("radial", list(&[text("Red"), text("Blue")])), ("radius", num(0.0))]), "Radius is 0; it is more than 0");
    }

    #[test]
    fn strokes_transforms_and_order_read_as_numbers_pairs_and_names() {
        let item = one(&rect(&[
            ("stroke", text("Black")),
            ("strokewidth", num(2.0)),
            ("dash", list(&[num(6.0), num(3.0)])),
            ("cap", text("round")),
            ("join", text("BEVEL")),
            ("opacity", num(0.5)),
            ("rotate", num(45.0)),
            ("scale", num(2.0)),
            ("skew", num(10.0)),
            ("origin", list(&[num(0.0), num(0.0)])),
            ("visible", Value::Bool(false)),
            ("order", Value::Int(-3)),
        ]));
        let s = &item.style;
        assert_eq!((s.stroke, s.stroke_width, s.dash.as_deref(), s.cap, s.join), (Some(Rgba([0, 0, 0, 255])), Some(2.0), Some(&[6.0, 3.0][..]), Some(Cap::Round), Some(Join::Bevel)));
        assert_eq!((s.opacity, s.rotate, s.scale, s.skew, s.origin, s.visible, s.order), (0.5, 45.0, [2.0, 2.0], [10.0, 0.0], Some([0.0, 0.0]), false, -3), "one number scales both ways and skews along x");
        assert_eq!(one(&rect(&[("dash", list(&[]))])).style.dash, Some(Vec::new()), "an empty Dash is a solid stroke");
        let refused = |pairs: &[(&str, Value)], want: &str| {
            let why = refusal(&rect(pairs));
            assert!(why.contains(want), "{why}");
        };
        refused(&[("opacity", num(1.5))], "Opacity is 1.5; it is from 0 to 1");
        refused(&[("skew", list(&[num(0.0), num(90.0)]))], "Skew gives 90 degrees");
        refused(&[("scale", list(&[num(1.0)]))], "Scale is a list of 1; it is a pair of finite numbers");
        refused(&[("dash", list(&[num(0.0), num(0.0)]))], "Dash's lengths are all 0");
        refused(&[("cap", text("flat"))], "Cap is 'flat'; it is Butt, Round or Square");
        refused(&[("visible", text("yes"))], "Visible is 'yes'; it is $true or $false");
        refused(&[("order", num(1.5))], "Order is 1.5; it is a whole number");
    }

    #[test]
    fn a_group_clips_to_a_shape_of_its_own() {
        let ellipse = map(&[("kind", text("ellipse")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(80.0)), ("height", num(60.0))]);
        let group = one(&map(&[("kind", text("Group")), ("clip", ellipse)]));
        assert_eq!(group.shape, Shape::Group { at: [0.0, 0.0], clip: Some(Box::new(Shape::Ellipse(Bounds { x: 0.0, y: 0.0, width: 80.0, height: 60.0 }))) });
        let group_of = |clip: Value| map(&[("kind", text("Group")), ("clip", clip)]);
        assert!(refusal(&group_of(map(&[("kind", text("Line"))]))).contains("Clip is a Line; a clip is a Rect, RoundedRect, Ellipse, Polygon or Path"));
        let styled = refusal(&group_of(map(&[("kind", text("Path")), ("data", text("M0 0 L5 5 L0 5 Z")), ("fill", text("Red"))])));
        assert!(styled.contains("Clip, a Path, takes no Fill; it takes Data"), "{styled}");
    }

    #[test]
    fn each_shape_s_outline_turns_clockwise_and_its_box_is_the_shape_s_own() {
        use crate::svg_path::{IDENTITY, bounds};
        let b = Bounds { x: 10.0, y: 20.0, width: 40.0, height: 30.0 };
        let boxed = |shape: Shape| shape.outline().and_then(|s| bounds(&s, IDENTITY));
        for shape in [Shape::Rect(b), Shape::RoundedRect(b, 8.0), Shape::RoundedRect(b, 100.0), Shape::Ellipse(b)] {
            let got = boxed(shape.clone());
            assert!(got.is_some_and(|g| g.iter().zip([10.0, 20.0, 40.0, 30.0]).all(|(a, w)| (a - w).abs() < 1e-9)), "{shape:?} boxes as {got:?}");
        }
        let Some(rounded) = Shape::RoundedRect(b, 100.0).outline() else { panic!("an outline") };
        assert_eq!(rounded[0], Segment::Move([10.0, 35.0]), "the radius is cut to half the height, and the outline starts at the top of the left side");
        assert_eq!(rounded[1], Segment::Arc { radii: [15.0, 15.0], rotation: 0.0, large: false, sweep: true, to: [25.0, 20.0] }, "the top left corner, clockwise");
        let Some(ellipse) = Shape::Ellipse(b).outline() else { panic!("an outline") };
        assert_eq!(ellipse.first(), Some(&Segment::Move([50.0, 35.0])), "from its right end");
        assert_eq!(ellipse.len(), 4, "two half arcs and a close");
        let flat = Shape::Ellipse(Bounds { height: 0.0, ..b }).outline();
        assert_eq!(flat, Some(vec![Segment::Move([10.0, 20.0]), Segment::Line([50.0, 20.0]), Segment::Close]), "an ellipse without height is the line it flattens to");
        assert_eq!(Shape::Polygon(vec![[0.0, 0.0], [4.0, 0.0], [0.0, 3.0]]).outline().map(|s| s.len()), Some(4), "three points and a close");
        assert_eq!(Shape::Text { at: [0.0, 0.0], text: String::from("Hi"), size: None }.outline(), None);
    }

    #[test]
    fn an_animation_moves_a_number_pair_or_color_the_kind_has_along_a_named_curve() {
        let item = one(&rect(&[(
            "animate",
            list(&[
                map(&[("property", text("X")), ("to", num(200.0)), ("duration", num(1.5)), ("easing", text("easeinoutcubic")), ("repeat", text("Forever")), ("reverse", Value::Bool(true))]),
                map(&[("property", text("Fill")), ("to", text("Gold")), ("duration", Value::Int(2))]),
                map(&[("property", text("Scale")), ("to", num(3.0)), ("duration", num(0.5)), ("repeat", Value::Int(2))]),
            ]),
        )]));
        assert_eq!(item.animations[0], Animation { property: String::from("x"), to: Target::Number(200.0), duration: 1.5, easing: Easing::EaseInOutCubic, repeat: Repeat::Forever, reverse: true });
        assert_eq!(item.animations[1], Animation { property: String::from("fill"), to: Target::Color(Rgba([255, 215, 0, 255])), duration: 2.0, easing: Easing::Ease, repeat: Repeat::Count(1), reverse: false }, "Ease, one pass and no reversing by default");
        assert_eq!(item.animations[2].to, Target::Pair([3.0, 3.0]));
        assert_eq!(Easing::ALL.len(), 35);
        assert_eq!(Easing::named("EaseOutBounce"), Ok(Easing::EaseOutBounce));
        let animated = |animation: Value| rect(&[("animate", animation)]);
        let refused = |animation: Value, want: &str| {
            let why = refusal(&animated(animation));
            assert!(why.contains(want), "{why}");
        };
        refused(map(&[("property", text("Points")), ("to", num(1.0)), ("duration", num(1.0))]), "a Rect moves no Points; it moves X, Y, Width, Height, Fill, Stroke, StrokeWidth, Opacity, Rotate, Scale, Skew, Origin");
        refused(map(&[("property", text("Opacity")), ("to", num(2.0)), ("duration", num(1.0))]), "Animate: To: Opacity is 2; it is from 0 to 1");
        refused(map(&[("property", text("X")), ("to", num(2.0)), ("duration", num(0.0))]), "Duration is 0; it is more than 0");
        refused(map(&[("property", text("X")), ("to", num(2.0)), ("duration", num(1.0)), ("easing", text("Wobbly"))]), "'Wobbly' is no easing");
        refused(map(&[("property", text("X")), ("to", num(2.0)), ("duration", num(1.0)), ("repeat", Value::Int(0))]), "Repeat is 0; it is a count of passes");
        refused(map(&[("property", text("X")), ("to", num(2.0)), ("duration", num(1.0)), ("speed", num(1.0))]), "an animation takes no speed");
        refused(list(&[map(&[("property", text("X")), ("to", num(2.0)), ("duration", num(1.0))]), map(&[("property", text("x")), ("to", num(3.0)), ("duration", num(1.0))])]), "Animate moves X twice");
        let gradient = rect(&[("fill", map(&[("linear", list(&[text("Red"), text("Blue")]))])), ("animate", map(&[("property", text("Fill")), ("to", text("Red")), ("duration", num(1.0))]))]);
        assert!(refusal(&gradient).contains("Animate moves Fill, which is a gradient"));
    }

    #[test]
    fn a_change_merges_into_an_item_and_is_read_again() {
        let item = one(&rect(&[("name", text("Box")), ("fill", text("Red"))]));
        let changes = BTreeMap::from([(String::from("x"), num(50.0)), (String::from("fill"), Value::Null), (String::from("rotate"), num(30.0))]);
        let changed = match item.changed(&changes) {
            Ok(c) => c,
            Err(why) => panic!("the change reads: {why}"),
        };
        assert_eq!(changed.shape, Shape::Rect(Bounds { x: 50.0, y: 20.0, width: 80.0, height: 40.0 }));
        assert_eq!((changed.style.fill.clone(), changed.style.rotate), (None, 30.0), "a null takes the fill away");
        assert_eq!(changed.name.as_deref(), Some("Box"));
        let width = item.changed(&BTreeMap::from([(String::from("width"), num(-2.0))]));
        assert!(matches!(&width, Err(why) if why == "Rect Box: Width is -2; it is 0 or more"), "{width:?}");
        let kind = item.changed(&BTreeMap::from([(String::from("kind"), text("Ellipse"))]));
        assert!(matches!(&kind, Err(why) if why.contains("a change sets no Kind")), "{kind:?}");
    }

    /// A box at the top and a group with a dot in it, the dot given as one
    /// map rather than a list of one.
    fn board() -> Board {
        let items = list(&[
            rect(&[("name", text("Box"))]),
            map(&[("kind", text("Group")), ("name", text("Layer")), ("children", map(&[("kind", text("Ellipse")), ("name", text("Dot")), ("x", num(0.0)), ("y", num(0.0)), ("width", num(4.0)), ("height", num(4.0))]))]),
        ]);
        kept(Board::of_items(&items))
    }

    fn kept<T>(result: Result<T, String>) -> T {
        match result {
            Ok(v) => v,
            Err(why) => panic!("the board takes it: {why}"),
        }
    }

    fn named<'a>(board: &'a Board, name: &str) -> &'a Entry {
        match board.find(name).and_then(|id| board.entry(id)) {
            Some(e) => e,
            None => panic!("the board has no {name}: {:?}", board.to_value()),
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_change_to_an_item_by_name_merges_at_any_depth_moves_its_revision_and_leaves_the_board_alone_when_it_does_not_read() {
        let mut b = board();
        let changes = BTreeMap::from([(String::from("x"), num(9.0)), (String::from("fill"), text("Gold"))]);
        let id = kept(b.set("DOT", &changes));
        assert_eq!(b.find("dot"), Some(id), "a name is found without regard to case");
        let dot = named(&b, "Dot");
        assert_eq!(dot.item.shape, Shape::Ellipse(Bounds { x: 9.0, y: 0.0, width: 4.0, height: 4.0 }));
        assert_eq!(dot.item.style.fill, Some(Paint::Color(Rgba([255, 215, 0, 255]))));
        assert_eq!(dot.revision, 1);
        kept(b.set("Dot", &BTreeMap::from([(String::from("fill"), Value::Null)])));
        assert_eq!(named(&b, "Dot").item.style.fill, None, "a null takes a key away");
        let before = b.to_value();
        let bad = b.set("Box", &BTreeMap::from([(String::from("width"), num(-1.0))]));
        assert!(matches!(&bad, Err(why) if why.contains("Rect Box: Width is -1")), "{bad:?}");
        assert_eq!(b.to_value(), before, "a change that does not read changes nothing");
        assert_eq!(b.set("Gone", &changes), Err(String::from("no item is named Gone")));
        let renamed = b.set("Box", &BTreeMap::from([(String::from("name"), text("Other"))]));
        assert!(matches!(&renamed, Err(why) if why.starts_with("Box: a change sets no Name")), "{renamed:?}");
    }

    #[test]
    fn items_are_added_above_a_board_s_or_a_group_s_own_and_a_name_given_twice_is_refused() {
        let mut b = board();
        let star = map(&[("kind", text("Polygon")), ("name", text("Star")), ("points", list(&[num(0.0), num(0.0), num(4.0), num(0.0), num(2.0), num(3.0)]))]);
        let ids = kept(b.add(None, std::slice::from_ref(&star)));
        assert_eq!(b.top().last(), ids.first(), "added above the board's own items");
        let spot = map(&[("kind", text("Ellipse")), ("name", text("Spot")), ("x", num(1.0)), ("y", num(1.0)), ("width", num(2.0)), ("height", num(2.0))]);
        kept(b.add(Some("layer"), std::slice::from_ref(&spot)));
        let inside: Vec<Option<String>> = named(&b, "Layer").children.iter().filter_map(|c| b.entry(*c)).map(|e| e.item.name.clone()).collect();
        assert_eq!(inside, vec![Some(String::from("Dot")), Some(String::from("Spot"))], "a group's new items follow its own");
        let spot_x = b.find("SPOT").and_then(|id| b.map(id)).and_then(|m| m.as_map().and_then(|m| m.get("x")).cloned());
        assert_eq!(spot_x, Some(num(1.0)), "an item's map is found at any depth");
        let before = b.to_value();
        let twice = b.add(None, &[spot]);
        assert!(matches!(&twice, Err(why) if why.contains("the name Spot is given twice")), "{twice:?}");
        assert_eq!(b.to_value(), before);
        assert_eq!(b.add(Some("Box"), std::slice::from_ref(&star)), Err(String::from("Box is no Group; items go into a Group")));
        assert_eq!(b.add(Some("Nowhere"), &[star]), Err(String::from("no item is named Nowhere")));
        let mut empty = kept(Board::of_items(&Value::Null));
        kept(empty.add(None, &[rect(&[])]));
        assert_eq!(empty.len(), 1, "a canvas made with no items takes some");
        let pack = map(&[("kind", text("Group")), ("name", text("Pack")), ("children", list(&[rect(&[("name", text("A"))]), rect(&[("name", text("B"))])]))]);
        assert_eq!(kept(empty.add(None, &[pack])).len(), 3, "a group brings its items, each with an id of its own");
    }

    #[test]
    fn items_are_taken_away_by_name_a_group_with_its_own_and_a_name_no_item_has_takes_none() {
        let mut b = board();
        let before = b.to_value();
        assert_eq!(b.remove(&names(&["box", "Missing"])), Err(String::from("no item is named Missing")));
        assert_eq!(b.to_value(), before, "nothing is taken when a name is not found");
        assert_eq!(kept(b.remove(&names(&["Dot"]))).len(), 1);
        assert!(named(&b, "Layer").children.is_empty(), "an item is taken from inside a group");
        assert_eq!(kept(b.remove(&names(&["Layer", "Box"]))).len(), 2);
        assert!(b.is_empty() && b.top().is_empty());
        assert_eq!(b.find("box"), None, "a name taken away is free again");
        let mut nested = board();
        assert_eq!(kept(nested.remove(&names(&["Layer", "Dot"]))).len(), 2, "a group and an item inside it go together");
    }

    #[test]
    fn a_board_writes_its_items_back_as_the_maps_a_canvas_holds() {
        let b = board();
        assert_eq!(b.len(), 3);
        let again = kept(Board::of_items(&b.to_value()));
        assert_eq!(again.to_value(), b.to_value());
    }

    #[test]
    fn every_curve_starts_at_0_ends_at_1_and_matches_its_definition_between() {
        for e in Easing::ALL {
            assert!(e.at(0.0).abs() < 1e-9 && (e.at(1.0) - 1.0).abs() < 1e-9, "{} runs from 0 to 1: {} {}", e.name(), e.at(0.0), e.at(1.0));
        }
        let near = |a: f64, b: f64| (a - b).abs() < 1e-4;
        assert!(near(Easing::Linear.at(0.3), 0.3));
        assert!(near(Easing::EaseInOutCubic.at(0.25), 0.0625));
        assert!(near(Easing::EaseOutQuad.at(0.5), 0.75));
        assert!(near(Easing::Ease.at(0.5), 0.8024033877399112), "CSS's ease at halfway: {}", Easing::Ease.at(0.5));
        assert!(near(Easing::EaseInOut.at(0.5), 0.5), "CSS's ease-in-out is symmetric");
        assert!(Easing::EaseInBack.at(0.2) < 0.0, "Back pulls back before it goes");
        assert!(Easing::EaseOutElastic.at(0.1) > 1.0, "Elastic overshoots");
        assert!(near(Easing::EaseOutBounce.at(1.0 / 2.75), 1.0), "the first bounce lands");
    }

    #[test]
    fn an_animation_passes_forward_back_with_reverse_and_holds_where_its_last_pass_ended() {
        let animation = |repeat, reverse| Animation { property: String::from("x"), to: Target::Number(10.0), duration: 2.0, easing: Easing::Linear, repeat, reverse };
        let once = animation(Repeat::Count(1), false);
        assert_eq!((once.share(1.0), once.share(2.0), once.share(9.0)), (0.5, 1.0, 1.0), "one pass, then stopped at To");
        assert_eq!(once.length(), Some(2.0));
        let back = animation(Repeat::Count(2), true);
        assert_eq!(back.length(), Some(8.0));
        assert_eq!((back.share(1.0), back.share(3.0), back.share(5.0), back.share(8.5)), (0.5, 0.5, 0.5, 0.0), "out and back twice, then stopped where it began");
        assert_eq!(back.share(2.0), 1.0, "the far end of a pass");
        let forever = animation(Repeat::Forever, false);
        assert_eq!((forever.length(), forever.share(101.0)), (None, 0.5));
    }

    #[test]
    fn a_value_moves_straight_or_in_premultiplied_color_and_an_item_reads_and_takes_it() {
        assert_eq!(Target::Number(0.0).toward(Target::Number(10.0), 0.25), Target::Number(2.5));
        assert_eq!(Target::Pair([0.0, 2.0]).toward(Target::Pair([4.0, 0.0]), 0.5), Target::Pair([2.0, 1.0]));
        let fade = Target::Color(Rgba([0, 0, 0, 0])).toward(Target::Color(Rgba([255, 0, 0, 255])), 0.5);
        assert_eq!(fade, Target::Color(Rgba([255, 0, 0, 128])), "from transparent, the color stays and only its alpha moves");
        let mut item = one(&rect(&[("fill", text("Red"))]));
        assert_eq!(item.value_of("x"), Some(Target::Number(10.0)));
        assert_eq!(item.value_of("stroke"), Some(Target::Color(Rgba([0, 0, 0, 0]))), "a stroke left to its group reads as transparent");
        assert_eq!(item.value_of("points"), None);
        item.set_value("x", Target::Number(25.0));
        item.set_value("opacity", Target::Number(1.4));
        item.set_value("fill", Target::Number(3.0));
        assert_eq!(item.shape, Shape::Rect(Bounds { x: 25.0, y: 20.0, width: 80.0, height: 40.0 }));
        assert_eq!(item.style.opacity, 1.0, "an overshoot is cut to the property's bounds");
        assert_eq!(item.style.fill, Some(Paint::Color(Rgba([255, 0, 0, 255]))), "a value of the wrong kind changes nothing");
    }
}
