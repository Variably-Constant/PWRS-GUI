//! The look both renderers draw from: a palette for light and one for dark,
//! the spacing and sizes, and the text sizes, all in points, with values
//! that are egui 0.37's own but where a constant's doc names another, and a
//! dialog's measures ([`DIALOG`]), which egui has none of. The egui
//! renderer builds its style from these, and the web helper gives them to
//! the widget runtime's page as CSS variables, so the two renderers differ
//! by rasterizer alone; the typeface both draw text in is
//! [`crate::typeface`]'s.

/// A color as egui stores one: sRGB with its alpha premultiplied. A color
/// whose alpha is 0 and whose other channels are not adds its light to what
/// is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);
    pub const WHITE: Color = Color([255, 255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color([r, g, b, 255])
    }

    pub const fn gray(l: u8) -> Color {
        Color([l, l, l, 255])
    }

    /// Black at alpha `a`.
    pub const fn black_alpha(a: u8) -> Color {
        Color([0, 0, 0, a])
    }

    /// Light of `l` added to what is under it.
    pub const fn additive(l: u8) -> Color {
        Color([l, l, l, 0])
    }
}

/// A line's width in points and its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
}

impl Stroke {
    pub const NONE: Stroke = Stroke { width: 0.0, color: Color::TRANSPARENT };

    pub const fn new(width: f32, color: Color) -> Stroke {
        Stroke { width, color }
    }
}

/// A shadow: how far it falls across and down, how far it blurs and
/// spreads, in points, and its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset: [i8; 2],
    pub blur: u8,
    pub spread: u8,
    pub color: Color,
}

/// How an interactive widget is drawn in one of its states: the fill of a
/// button, the fill of a check box or a slider's rail, the outline, the
/// text and marks, the corners' radius, and how far the widget grows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub weak_bg_fill: Color,
    pub bg_fill: Color,
    pub bg_stroke: Stroke,
    pub fg_stroke: Stroke,
    pub corner_radius: u8,
    pub expansion: f32,
}

/// The colors of one theme, and the corners and shadows of windows and
/// menus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Labels, frames and separators: what takes no input.
    pub noninteractive: Look,
    /// A widget at rest.
    pub inactive: Look,
    /// A widget under the pointer.
    pub hovered: Look,
    /// A widget pressed or dragged.
    pub active: Look,
    /// A drop-down or menu whose popup is open.
    pub open: Look,
    /// Selected text, items and rows.
    pub selection_fill: Color,
    pub selection_stroke: Stroke,
    pub hyperlink: Color,
    /// Barely different from the panel, as a striped row is.
    pub faint_bg: Color,
    /// A text field's background.
    pub extreme_bg: Color,
    /// Code's background.
    pub code_bg: Color,
    pub warn: Color,
    pub error: Color,
    pub window_corner_radius: u8,
    pub window_shadow: Shadow,
    /// A popup's and a menu's background.
    pub window_fill: Color,
    pub window_stroke: Stroke,
    pub menu_corner_radius: u8,
    /// A window's background.
    pub panel_fill: Color,
    pub popup_shadow: Shadow,
    /// A text field's cursor.
    pub text_cursor: Stroke,
    /// Weak text's alpha, as a share of the text color's.
    pub weak_text_alpha: f32,
    /// A disabled widget's alpha, as a share of its color's.
    pub disabled_alpha: f32,
    /// A button bar's strip on Windows, a shade darker than the window, as
    /// Windows' own dialogs draw the strip under their buttons.
    pub strip: Color,
}

/// A dialog's measures in points, which egui has none of: from Windows'
/// dialog layout guidelines and WinUI's ContentDialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dialog {
    /// Around the widgets of a window with a button bar, and inside the bar.
    pub margin: f32,
    /// The least size of a button bar's button, across and down.
    pub button: [f32; 2],
    /// Between a button bar's buttons.
    pub button_gap: f32,
    /// A Text of style `instruction`, a dialog's main line.
    pub instruction: f32,
    /// The least and most width of a dialog fitted to its content.
    pub width: [f32; 2],
    /// The most height of a dialog fitted to its content.
    pub height_max: f32,
}

/// The spacing and sizes, in points, of both themes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// Between widgets side by side, and one above another.
    pub item_spacing: [f32; 2],
    /// Inside a floating window.
    pub window_margin: i8,
    /// Inside a menu or a popup.
    pub menu_margin: i8,
    /// Between a button's text and its edges, across and down.
    pub button_padding: [f32; 2],
    /// How far a tree's or a nested section's contents are indented.
    pub indent: f32,
    /// The least size of an interactive widget, across and down.
    pub interact_size: [f32; 2],
    pub slider_width: f32,
    pub slider_rail_height: f32,
    /// A drop-down's width.
    pub combo_width: f32,
    /// A text field's width.
    pub text_edit_width: f32,
    /// The check box's and option button's box, its inside mark, and the
    /// space between the box and its text.
    pub icon_width: f32,
    pub icon_width_inner: f32,
    pub icon_spacing: f32,
    /// The size a widget that takes the room it is given, such as a chart,
    /// a GPU surface or a canvas, takes where it is given none, as a window
    /// fitting its content is measured.
    pub default_area_size: [f32; 2],
    pub tooltip_width: f32,
    pub menu_width: f32,
    /// Between a menu's entries.
    pub menu_spacing: f32,
    /// The most a drop-down's list shows before it scrolls.
    pub combo_height: f32,
    /// Around a window's widgets.
    pub panel_margin: i8,
    /// Around a menu bar's and a status bar's widgets, across and down.
    pub bar_margin: [i8; 2],
    /// Inside a group's frame.
    pub group_margin: i8,
    /// Between a text field's text and its edges, across and down.
    pub text_edit_margin: [i8; 2],
    /// The rows a text field of several lines shows.
    pub text_edit_rows: u32,
    /// A scroll bar at its widest, and while nothing points at it.
    pub scroll_bar_width: f32,
    pub scroll_bar_floating_width: f32,
    /// The room a scroll bar takes beside the content it scrolls.
    pub scroll_bar_allocated_width: f32,
    /// The shortest a scroll bar's handle gets.
    pub scroll_handle_min_length: f32,
    /// Between a scroll bar and the content beside it.
    pub scroll_bar_inner_margin: f32,
    /// How near a table column's edge, either side, the pointer takes it to
    /// resize the column: egui's `Interaction::resize_grab_radius_side`.
    pub resize_grab: f32,
}

/// Text sizes in points: the height of the font's em, as both an egui
/// `FontId` and a CSS `font-size` take it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSizes {
    pub small: f32,
    pub body: f32,
    pub button: f32,
    pub heading: f32,
    pub monospace: f32,
}

/// The light theme: egui's `Visuals::light()`.
pub const LIGHT: Palette = Palette {
    dark: false,
    noninteractive: Look {
        weak_bg_fill: Color::gray(248),
        bg_fill: Color::gray(248),
        bg_stroke: Stroke::new(1.0, Color::gray(190)),
        fg_stroke: Stroke::new(1.0, Color::gray(80)),
        corner_radius: 4,
        expansion: 0.0,
    },
    inactive: Look { weak_bg_fill: Color::gray(230), bg_fill: Color::gray(230), bg_stroke: Stroke::NONE, fg_stroke: Stroke::new(1.0, Color::gray(60)), corner_radius: 6, expansion: 0.0 },
    hovered: Look {
        weak_bg_fill: Color::gray(220),
        bg_fill: Color::gray(220),
        bg_stroke: Stroke::new(1.0, Color::gray(105)),
        fg_stroke: Stroke::new(1.5, Color::BLACK),
        corner_radius: 6,
        expansion: 0.0,
    },
    active: Look { weak_bg_fill: Color::gray(165), bg_fill: Color::gray(165), bg_stroke: Stroke::new(1.0, Color::BLACK), fg_stroke: Stroke::new(2.0, Color::BLACK), corner_radius: 6, expansion: 0.0 },
    open: Look { weak_bg_fill: Color::gray(220), bg_fill: Color::gray(220), bg_stroke: Stroke::new(1.0, Color::gray(160)), fg_stroke: Stroke::new(1.0, Color::BLACK), corner_radius: 6, expansion: 0.0 },
    selection_fill: Color::rgb(144, 209, 255),
    selection_stroke: Stroke::new(1.0, Color::rgb(0, 83, 125)),
    hyperlink: Color::rgb(0, 155, 255),
    faint_bg: Color::additive(5),
    extreme_bg: Color::gray(255),
    code_bg: Color::gray(230),
    warn: Color::rgb(255, 100, 0),
    error: Color::rgb(255, 0, 0),
    window_corner_radius: 6,
    window_shadow: Shadow { offset: [10, 20], blur: 15, spread: 0, color: Color::black_alpha(25) },
    window_fill: Color::gray(248),
    window_stroke: Stroke::new(1.0, Color::gray(190)),
    menu_corner_radius: 6,
    panel_fill: Color::gray(248),
    popup_shadow: Shadow { offset: [6, 10], blur: 8, spread: 0, color: Color::black_alpha(25) },
    text_cursor: Stroke::new(2.0, Color::rgb(0, 83, 125)),
    weak_text_alpha: 0.6,
    disabled_alpha: 0.5,
    strip: Color::gray(236),
};

/// The dark theme: egui's `Visuals::dark()`, with labels in gray 200 where
/// egui's are gray 140, so they read as the systems' own dark labels do.
pub const DARK: Palette = Palette {
    dark: true,
    noninteractive: Look {
        weak_bg_fill: Color::gray(27),
        bg_fill: Color::gray(27),
        bg_stroke: Stroke::new(1.0, Color::gray(60)),
        fg_stroke: Stroke::new(1.0, Color::gray(200)),
        corner_radius: 4,
        expansion: 0.0,
    },
    inactive: Look { weak_bg_fill: Color::gray(50), bg_fill: Color::gray(60), bg_stroke: Stroke::NONE, fg_stroke: Stroke::new(1.0, Color::gray(215)), corner_radius: 6, expansion: 0.0 },
    hovered: Look {
        weak_bg_fill: Color::gray(64),
        bg_fill: Color::gray(70),
        bg_stroke: Stroke::new(1.0, Color::gray(150)),
        fg_stroke: Stroke::new(1.5, Color::gray(245)),
        corner_radius: 6,
        expansion: 0.0,
    },
    active: Look { weak_bg_fill: Color::gray(40), bg_fill: Color::gray(55), bg_stroke: Stroke::new(1.0, Color::WHITE), fg_stroke: Stroke::new(2.0, Color::WHITE), corner_radius: 6, expansion: 0.0 },
    open: Look { weak_bg_fill: Color::gray(45), bg_fill: Color::gray(27), bg_stroke: Stroke::new(1.0, Color::gray(60)), fg_stroke: Stroke::new(1.0, Color::gray(210)), corner_radius: 6, expansion: 0.0 },
    selection_fill: Color::rgb(0, 92, 128),
    selection_stroke: Stroke::new(1.0, Color::rgb(192, 222, 255)),
    hyperlink: Color::rgb(90, 170, 255),
    faint_bg: Color::additive(5),
    extreme_bg: Color::gray(10),
    code_bg: Color::gray(64),
    warn: Color::rgb(255, 143, 0),
    error: Color::rgb(255, 0, 0),
    window_corner_radius: 6,
    window_shadow: Shadow { offset: [10, 20], blur: 15, spread: 0, color: Color::black_alpha(96) },
    window_fill: Color::gray(27),
    window_stroke: Stroke::new(1.0, Color::gray(60)),
    menu_corner_radius: 6,
    panel_fill: Color::gray(27),
    popup_shadow: Shadow { offset: [6, 10], blur: 8, spread: 0, color: Color::black_alpha(96) },
    text_cursor: Stroke::new(2.0, Color::rgb(192, 222, 255)),
    weak_text_alpha: 0.6,
    disabled_alpha: 0.5,
    strip: Color::gray(19),
};

/// egui's `Spacing::default()`, its frames' margins and its text field's,
/// with its thin scroll bars (`ScrollStyle::thin`), which show while the
/// content is longer than its view.
pub const SPACING: Spacing = Spacing {
    item_spacing: [8.0, 3.0],
    window_margin: 6,
    menu_margin: 6,
    button_padding: [8.0, 2.0],
    indent: 18.0,
    interact_size: [40.0, 20.0],
    slider_width: 100.0,
    slider_rail_height: 8.0,
    combo_width: 100.0,
    text_edit_width: 280.0,
    icon_width: 14.0,
    icon_width_inner: 8.0,
    icon_spacing: 4.0,
    default_area_size: [600.0, 400.0],
    tooltip_width: 500.0,
    menu_width: 400.0,
    menu_spacing: 2.0,
    combo_height: 200.0,
    panel_margin: 8,
    bar_margin: [8, 2],
    group_margin: 6,
    text_edit_margin: [4, 2],
    text_edit_rows: 4,
    scroll_bar_width: 10.0,
    scroll_bar_floating_width: 2.0,
    scroll_bar_allocated_width: 6.0,
    scroll_handle_min_length: 12.0,
    scroll_bar_inner_margin: 4.0,
    resize_grab: 3.0,
};

/// egui's default text styles.
pub const TEXT: TextSizes = TextSizes { small: 9.0, body: 13.0, button: 13.0, heading: 18.0, monospace: 13.0 };

/// A dialog's measures: Windows' 11-point margin rounded to 12 as Visual
/// Studio's dialogs take it, buttons of at least Windows' 75 by 23 at
/// egui's 13-point text, WinUI's 8 points between buttons, and WinUI's
/// ContentDialog's widths from 320 to 548 and height to 756.
pub const DIALOG: Dialog = Dialog { margin: 12.0, button: [80.0, 24.0], button_gap: 8.0, instruction: 15.0, width: [320.0, 548.0], height_max: 756.0 };

/// A progress bar's thickness in points, thin as Fluent's large determinate
/// bar and macOS's are, under the line with its text.
pub const PROGRESS_BAR: f32 = 4.0;

/// The thickness in points of the line under a tab's title, the chosen
/// tab's and a hovered one's, as Fluent's TabList draws it.
pub const TAB_MARK: f32 = 3.0;

/// A chevron whose arms meet at a right angle: `span` points from one arm's
/// end to the other's, its point half that from the line between them, and
/// its arms `stroke` points thick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chevron {
    pub span: f32,
    pub stroke: f32,
}

/// A tree item's chevron, about Fluent's 12-pixel chevron at egui's
/// 13-point text.
pub const CHEVRON: Chevron = Chevron { span: 8.0, stroke: 1.5 };

/// A password field's peek button: a box `button` points wide at the
/// field's right end, as tall as the field's inside, holding an eye
/// `eye` points across and down, drawn in lines `stroke` points thick,
/// with a pupil `pupil` points across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeekButton {
    pub button: f32,
    pub eye: [f32; 2],
    pub stroke: f32,
    pub pupil: f32,
}

/// The peek button, about Fluent's reveal glyph at egui's 13-point text.
pub const PEEK: PeekButton = PeekButton { button: 22.0, eye: [13.0, 8.0], stroke: 1.25, pupil: 3.5 };

/// The palette of `dark` or light.
pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}
