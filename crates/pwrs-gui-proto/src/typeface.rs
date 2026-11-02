//! The fonts both renderers draw text in, by family: for proportional text
//! the system's UI font, and for monospace text the font an HTML window
//! draws with. Each kind is a list in the order the system prefers them,
//! and a renderer draws in the first of it the system has: egui by looking
//! each up, a page by naming the list in CSS, whose engine takes the first
//! it finds. Both renderers find that font with [`first_found`], and a page
//! is given the height of a row of text in it ([`row_height`]), which both
//! renderers round to a whole physical pixel, so a line of text takes the
//! same room in both.

/// Where a renderer looks for the font of one kind of text.
#[derive(Clone, Debug, PartialEq)]
pub enum Wanted {
    /// A family by its name.
    Family(String),
    /// A font file by its path: the system's own UI font on macOS, whose
    /// family a page names as `system-ui`.
    #[cfg(target_vendor = "apple")]
    File(&'static str),
}

/// The fonts to look for, each kind in the order the system prefers them,
/// and each problem met finding out which.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Families {
    pub ui: Vec<Wanted>,
    pub mono: Vec<Wanted>,
    pub problems: Vec<String>,
}

/// A font the system has for one kind of text: the face's bytes and its
/// index in its file, its family, and the file it came from.
#[derive(Clone, Debug)]
pub struct Found {
    pub data: Vec<u8>,
    pub index: u32,
    pub family: String,
    pub path: String,
}

/// The system's fonts as both renderers search them: the faces in the
/// system's font folders, and on macOS in Terminal's own, where SF Mono is.
pub fn system_database() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    if cfg!(target_vendor = "apple") {
        db.load_fonts_dir("/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts");
    }
    db
}

/// The path of a face's file; none for a face read into memory.
pub fn source_path(source: &fontdb::Source) -> Option<&std::path::Path> {
    match source {
        fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path),
        fontdb::Source::Binary(_) => None,
    }
}

/// The first font of `wanted` in the database, or none; one it lists
/// whose file cannot be read is an error naming the file.
pub fn first_found(db: &fontdb::Database, wanted: &[Wanted]) -> Result<Option<Found>, String> {
    for w in wanted {
        let id = match w {
            Wanted::Family(name) => db.query(&fontdb::Query { families: &[fontdb::Family::Name(name)], ..fontdb::Query::default() }),
            #[cfg(target_vendor = "apple")]
            Wanted::File(path) => db.faces().find(|f| source_path(&f.source) == Some(std::path::Path::new(path))).map(|f| f.id),
        };
        let Some(face) = id.and_then(|id| db.face(id)) else {
            continue;
        };
        let path = match source_path(&face.source) {
            Some(p) => p.display().to_string(),
            None => String::from("memory"),
        };
        let family = match face.families.first() {
            Some((name, _)) => name.clone(),
            None => face.post_script_name.clone(),
        };
        return match db.with_face_data(face.id, |data, index| (data.to_vec(), index)) {
            Some((data, index)) => Ok(Some(Found { data, index, family, path })),
            None => Err(format!("the system lists the font {family} in {path}, and the file cannot be read")),
        };
    }
    Ok(None)
}

/// What egui rounds a text's metrics to: 1/32 of a point.
const GUI_ROUNDING: f32 = 1.0 / 32.0;

/// The OS/2 table's flag saying its typographic metrics are the ones to use.
const USE_TYPO_METRICS: u16 = 1 << 7;

/// A face's vertical metrics as egui's fonts measure them: its units per em,
/// and its ascent, descent (below the baseline, so negative) and line gap
/// in those units, as FreeType chooses them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMetrics {
    pub units: f32,
    pub ascent: f32,
    pub descent: f32,
    pub gap: f32,
}

impl FaceMetrics {
    /// The ascent, descent and line gap in points of text `size` points
    /// tall, each scaled and rounded to 1/32 of a point as egui rounds them.
    pub fn at(&self, size: f32) -> [f32; 3] {
        let scale = size / self.units;
        let round = |v: f32| (v / GUI_ROUNDING).round() * GUI_ROUNDING;
        [round(self.ascent * scale), round(self.descent * scale), round(self.gap * scale)]
    }
}

/// The height in points of one row of text `size` points tall in the face
/// at `index` in `data`, as egui's fonts measure it: the face's ascent,
/// descent and line gap ([`face_metrics`]), each scaled to `size` and
/// rounded to 1/32 of a point before they are added. egui's layout then
/// rounds the row to a whole physical pixel.
pub fn row_height(data: &[u8], index: u32, size: f32) -> Result<f32, String> {
    let [ascent, descent, gap] = face_metrics(data, index)?.at(size);
    Ok(ascent - descent + gap)
}

/// The width in points of a space in text `size` points tall in the face
/// at `index` in `data`, at the face's default instance, as egui measures
/// a glyph's width: its advance scaled to `size`, unrounded.
pub fn space_width(data: &[u8], index: u32, size: f32) -> Result<f32, String> {
    let face = ttf_parser::Face::parse(data, index).map_err(|e| format!("the font cannot be read: {e}"))?;
    let space = face.glyph_index(' ').ok_or_else(|| String::from("the font has no space"))?;
    let advance = face.glyph_hor_advance(space).ok_or_else(|| String::from("the font gives the space no advance"))?;
    Ok(f32::from(advance) * size / f32::from(face.units_per_em()))
}

/// The vertical metrics of the face at `index` in `data` as FreeType
/// chooses them: the OS/2 table's typographic metrics when its
/// USE_TYPO_METRICS flag is set, else the hhea table's; where those give no
/// ascent and no descent, the typographic ones, or the Windows ones when
/// those are zero as well. A table shorter than its version's fields counts
/// as absent.
pub fn face_metrics(data: &[u8], index: u32) -> Result<FaceMetrics, String> {
    let face = ttf_parser::RawFace::parse(data, index).map_err(|e| format!("the font cannot be read: {e}"))?;
    let table = |tag: &[u8; 4]| face.table(ttf_parser::Tag::from_bytes(tag));
    let unsigned = |t: &[u8], at: usize| u16::from_be_bytes([t[at], t[at + 1]]);
    let signed = |t: &[u8], at: usize| f32::from(i16::from_be_bytes([t[at], t[at + 1]]));
    let units = table(b"head").filter(|t| t.len() >= 54).map(|t| unsigned(t, 18)).filter(|u| *u > 0).ok_or_else(|| String::from("the font says no units per em"))?;
    let os2 = table(b"OS/2").filter(|t| {
        let least = match t.get(..2).map(|v| u16::from_be_bytes([v[0], v[1]])) {
            Some(0) => 78,
            Some(1) => 86,
            Some(2..=4) => 96,
            Some(_) => 100,
            None => usize::MAX,
        };
        t.len() >= least
    });
    let typo = os2.map(|t| (signed(t, 68), signed(t, 70), signed(t, 72)));
    let (mut ascent, mut descent, mut gap) = (0.0, 0.0, 0.0);
    match (os2, typo) {
        (Some(t), Some(chosen)) if unsigned(t, 62) & USE_TYPO_METRICS != 0 => (ascent, descent, gap) = chosen,
        _ => {
            if let Some(h) = table(b"hhea").filter(|t| t.len() >= 36) {
                (ascent, descent, gap) = (signed(h, 4), signed(h, 6), signed(h, 8));
            }
            if ascent == 0.0
                && descent == 0.0
                && let (Some(t), Some((a, d, g))) = (os2, typo)
            {
                if a != 0.0 || d != 0.0 {
                    (ascent, descent, gap) = (a, d, g);
                } else {
                    (ascent, descent) = (f32::from(unsigned(t, 74)), -f32::from(unsigned(t, 76)));
                }
            }
        }
    }
    Ok(FaceMetrics { units: f32::from(units), ascent, descent, gap })
}

/// The size harfrust tracks a face at when it is given none, as egui gives
/// it none.
const HARFRUST_POINTS: f32 = 12.0;

/// The spacing, in em, that a page gives text in the face at `index` in
/// `data` so that WebKit on macOS lays it out as egui does; `None` where
/// the two space it alike. egui's shaper, harfrust, tracks a face that has
/// a tracking table by that table at 12 points; CoreText tracks a face with
/// an optical size axis by the table at the axis's default, the instance a
/// page draws with optical sizing off. The spacing is the first tracking
/// less the second, each read from the table's track 0 as harfrust reads
/// it. A table the face has and that cannot be read is an error.
pub fn page_tracking(data: &[u8], index: u32) -> Result<Option<f32>, String> {
    let face = ttf_parser::Face::parse(data, index).map_err(|e| format!("the font cannot be read: {e}"))?;
    let has = |tag: &[u8; 4]| face.raw_face().table(ttf_parser::Tag::from_bytes(tag)).is_some();
    let trak = match face.tables().trak {
        Some(t) => t,
        None if has(b"trak") => return Err(String::from("the font's tracking table cannot be read")),
        None => return Ok(None),
    };
    let axes = match face.tables().fvar {
        Some(f) => f.axes,
        None if has(b"fvar") => return Err(String::from("the font's variation axes cannot be read")),
        None => return Ok(None),
    };
    let opsz = ttf_parser::Tag::from_bytes(b"opsz");
    let Some(optical) = axes.into_iter().find(|a| a.tag == opsz) else { return Ok(None) };
    let sizes: Vec<f32> = trak.horizontal.sizes.into_iter().map(|s| s.0).collect();
    let tracks: Vec<(f32, Vec<f32>)> = trak.horizontal.tracks.into_iter().map(|t| (t.value, t.values.into_iter().map(f32::from).collect())).collect();
    let units = f32::from(face.units_per_em());
    let spacing = (normal_tracking(&tracks, &sizes, HARFRUST_POINTS) - normal_tracking(&tracks, &sizes, optical.def_value)) / units;
    Ok((spacing != 0.0).then_some(spacing))
}

/// A tracking table's tracking in font units at `points`, in its track 0
/// as harfrust reads it: the track whose value is 0, else the two around 0
/// interpolated; each track's per-size values interpolated between the
/// sizes around `points`, the first size's below them and the last's above.
/// `tracks` has each track's value and per-size values in the table's
/// order.
fn normal_tracking(tracks: &[(f32, Vec<f32>)], sizes: &[f32], points: f32) -> f32 {
    let at = |values: &[f32]| -> f32 {
        let n = sizes.len().min(values.len());
        if n == 0 {
            return 0.0;
        }
        match (0..n).find(|&i| sizes[i] >= points) {
            Some(0) => values[0],
            Some(i) if (sizes[i] - sizes[i - 1]).abs() < f32::EPSILON => (values[i - 1] + values[i]) / 2.0,
            Some(i) => values[i - 1] + (points - sizes[i - 1]) / (sizes[i] - sizes[i - 1]) * (values[i] - values[i - 1]),
            None => values[n - 1],
        }
    };
    match tracks {
        [] => 0.0,
        [(_, only)] => at(only),
        _ => {
            let mut below = 0;
            while below + 1 < tracks.len() && tracks[below + 1].0 <= 0.0 {
                below += 1;
            }
            let mut above = tracks.len() - 1;
            while above > 0 && tracks[above - 1].0 >= 0.0 {
                above -= 1;
            }
            let (low, high) = (&tracks[below], &tracks[above]);
            if below == above || (high.0 - low.0).abs() < f32::EPSILON {
                return at(&low.1);
            }
            let (a, b) = (at(&low.1), at(&high.1));
            a + (0.0 - low.0) / (high.0 - low.0) * (b - a)
        }
    }
}

/// The fonts to look for: the font Windows draws its messages in and
/// Consolas, with the problem met reading the first.
#[cfg(windows)]
pub fn wanted() -> Families {
    let mono = vec![Wanted::Family(String::from("Consolas"))];
    match message_font() {
        Ok(family) => Families { ui: vec![Wanted::Family(family)], mono, problems: Vec::new() },
        Err(why) => Families { ui: vec![Wanted::Family(String::from("Segoe UI"))], mono, problems: vec![why] },
    }
}

/// The fonts to look for: the system font's file and SF Mono.
#[cfg(target_vendor = "apple")]
pub fn wanted() -> Families {
    Families { ui: vec![Wanted::File("/System/Library/Fonts/SFNS.ttf")], mono: vec![Wanted::Family(String::from("SF Mono"))], problems: Vec::new() }
}

/// The fonts to look for: the families fontconfig prefers for sans-serif
/// and for monospace text, with the problems met reading its configuration.
#[cfg(all(unix, not(target_vendor = "apple")))]
pub fn wanted() -> Families {
    let path = fontconfig_path();
    match fontconfig_file(&path).and_then(|file| fontconfig_families(&file, &path)) {
        Ok(found) => Families { ui: found.sans.into_iter().map(Wanted::Family).collect(), mono: found.mono.into_iter().map(Wanted::Family).collect(), problems: found.problems },
        Err(why) => Families { ui: Vec::new(), mono: Vec::new(), problems: vec![why] },
    }
}

/// No system fonts to look for where no font store is known.
#[cfg(not(any(windows, unix)))]
pub fn wanted() -> Families {
    Families::default()
}

/// The family Windows draws its message boxes' text in: Segoe UI on an
/// English system.
#[cfg(windows)]
fn message_font() -> Result<String, String> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::WindowsAndMessaging::{NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SystemParametersInfoW};
    let mut metrics: NONCLIENTMETRICSW = unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    if unsafe { SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, (&mut metrics as *mut NONCLIENTMETRICSW).cast(), 0) } == 0 {
        let code = unsafe { GetLastError() };
        return Err(format!("Windows did not say the font it draws its messages in (error {code}), so Segoe UI is looked for"));
    }
    let face = &metrics.lfMessageFont.lfFaceName;
    let end = match face.iter().position(|c| *c == 0) {
        Some(at) => at,
        None => face.len(),
    };
    Ok(String::from_utf16_lossy(&face[..end]))
}

/// The folders fontconfig looks in for a file named without one: those
/// `FONTCONFIG_PATH` lists, then its own, `/etc/fonts` or FreeBSD's
/// `/usr/local/etc/fonts`, whichever has `fonts.conf`.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn fontconfig_path() -> Vec<std::path::PathBuf> {
    let mut path = Vec::new();
    if let Some(listed) = std::env::var_os("FONTCONFIG_PATH") {
        path.extend(std::env::split_paths(&listed).filter(|p| !p.as_os_str().is_empty()));
    }
    if let Some(own) = ["/etc/fonts", "/usr/local/etc/fonts"].into_iter().map(std::path::PathBuf::from).find(|p| p.join("fonts.conf").is_file()) {
        path.push(own);
    }
    path
}

/// A file or folder of fontconfig's, found as fontconfig finds one by
/// `name`: a name from the root as it is, one under `~` in the home folder,
/// and any other in the first folder of `path` that has it; none when it
/// is not there.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn fontconfig_find(name: &std::path::Path, path: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    if name.is_absolute() {
        return name.exists().then(|| name.to_path_buf());
    }
    let mut parts = name.components();
    if parts.next() == Some(std::path::Component::Normal(std::ffi::OsStr::new("~"))) {
        return std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(parts.as_path())).filter(|p| p.exists());
    }
    path.iter().map(|folder| folder.join(name)).find(|p| p.exists())
}

/// fontconfig's configuration file: the one `FONTCONFIG_FILE` names, else
/// `fonts.conf`, found as fontconfig finds a file by name.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn fontconfig_file(path: &[std::path::PathBuf]) -> Result<std::path::PathBuf, String> {
    let name = match std::env::var_os("FONTCONFIG_FILE") {
        Some(file) => std::path::PathBuf::from(file),
        None => std::path::PathBuf::from("fonts.conf"),
    };
    fontconfig_find(&name, path).ok_or_else(|| {
        if name.is_absolute() {
            format!("fontconfig's configuration {} does not exist, so text is drawn in egui's own fonts", name.display())
        } else {
            let folders: Vec<String> = path.iter().map(|p| p.display().to_string()).collect();
            format!("fontconfig's configuration {} is in none of its folders ({}), so text is drawn in egui's own fonts", name.display(), folders.join(", "))
        }
    })
}

/// What fontconfig's configuration says of families as it is read: its
/// aliases in the order fontconfig reads them, the files read, and each
/// file or include that could not be read, with why.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[derive(Default)]
struct FontconfigRead {
    aliases: Vec<fontconfig_parser::Alias>,
    read: std::collections::HashSet<std::path::PathBuf>,
    problems: Vec<String>,
}

/// Reads fontconfig's configuration at `at` as fontconfig includes it: a
/// file's aliases in order and each of its includes where it stands, and a
/// folder's files whose names start with a digit and end in `.conf`, in the
/// order of their names. An include with no prefix is found as fontconfig
/// finds a file by name, in `path`; with the prefix `xdg` in
/// `XDG_CONFIG_HOME` or `~/.config`, `relative` beside the file including
/// it, and `cwd` in the working folder. A file read once is not read again.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn read_fontconfig(at: &std::path::Path, path: &[std::path::PathBuf], out: &mut FontconfigRead) -> Result<(), String> {
    use fontconfig_parser::{ConfigPart, DirPrefix};
    let unread = |e: std::io::Error| format!("fontconfig's configuration {} cannot be read: {e}", at.display());
    if std::fs::metadata(at).map_err(unread)?.is_dir() {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(at).map_err(unread)? {
            let entry = entry.map_err(unread)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(|c: char| c.is_ascii_digit()) && name.ends_with(".conf") {
                files.push(entry.path());
            }
        }
        files.sort();
        for file in files {
            if let Err(why) = read_fontconfig(&file, path, out) {
                out.problems.push(why);
            }
        }
        return Ok(());
    }
    if !out.read.insert(std::fs::canonicalize(at).map_err(unread)?) {
        return Ok(());
    }
    let text = std::fs::read_to_string(at).map_err(unread)?;
    let parts = fontconfig_parser::parse_config_parts(&text).map_err(|e| format!("fontconfig's configuration {} cannot be read: {e}", at.display()))?;
    for part in parts {
        match part {
            ConfigPart::Alias(alias) => out.aliases.push(alias),
            ConfigPart::Include(include) => {
                let name = std::path::Path::new(&include.path);
                let target = match include.prefix {
                    DirPrefix::Default => fontconfig_find(name, path),
                    DirPrefix::Xdg => {
                        let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
                            Some(folder) => Some(std::path::PathBuf::from(folder)),
                            None => std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config")),
                        };
                        config_home.map(|folder| folder.join(name)).filter(|p| p.exists())
                    }
                    DirPrefix::Relative => at.parent().map(|folder| folder.join(name)).filter(|p| p.exists()),
                    DirPrefix::Cwd => Some(name.to_path_buf()).filter(|p| p.exists()),
                };
                match target {
                    Some(found) => {
                        if let Err(why) = read_fontconfig(&found, path, out) {
                            out.problems.push(why);
                        }
                    }
                    None if include.ignore_missing => {}
                    None => out.problems.push(format!("fontconfig's configuration {} includes {}, which is not there", at.display(), include.path)),
                }
            }
            ConfigPart::Description(_)
            | ConfigPart::SelectFont(_)
            | ConfigPart::Dir(_)
            | ConfigPart::CacheDir(_)
            | ConfigPart::Match(_)
            | ConfigPart::Config(_)
            | ConfigPart::RemapDir(_)
            | ConfigPart::ResetDirs => {}
        }
    }
    Ok(())
}

/// The families fontconfig's configuration prefers for each kind of text,
/// in the order it tries them, and each file or include of it that could
/// not be read, with why.
#[cfg(all(unix, not(target_vendor = "apple")))]
struct FontconfigFamilies {
    sans: Vec<String>,
    mono: Vec<String>,
    problems: Vec<String>,
}

/// The families fontconfig's configuration `file` prefers for sans-serif
/// and for monospace text, in the order it tries them, which is where
/// fontconfig's edits put them in the pattern: every alias's `<prefer>`
/// families in the order the aliases are read, each set inserted just
/// before the generic name; then the `<accept>` families of the last alias
/// first, each set inserted just after it; then every `<default>` family
/// in order, each set appended. `path` is where fontconfig finds a file
/// named without a folder.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn fontconfig_families(file: &std::path::Path, path: &[std::path::PathBuf]) -> Result<FontconfigFamilies, String> {
    let mut read = FontconfigRead::default();
    read_fontconfig(file, path, &mut read)?;
    let problems = std::mem::take(&mut read.problems);
    let families = |names: &[&str]| -> Vec<String> {
        let aliases: Vec<&fontconfig_parser::Alias> = read.aliases.iter().filter(|a| names.contains(&a.alias.to_lowercase().as_str())).collect();
        let preferred = aliases.iter().flat_map(|a| a.prefer.iter());
        let accepted = aliases.iter().rev().flat_map(|a| a.accept.iter());
        preferred.chain(accepted).chain(aliases.iter().flat_map(|a| a.default.iter())).cloned().collect()
    };
    Ok(FontconfigFamilies { sans: families(&["sans-serif", "sans serif"]), mono: families(&["monospace"]), problems })
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    use super::*;

    /// SFNS.ttf's tracking table, as macOS 14 and 26 ship it: its sizes, and
    /// its tracks' values and per-size values.
    const SF_SIZES: [f32; 21] = [6.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 20.0, 22.0, 24.0, 28.0, 32.0, 36.0, 50.0, 64.0, 80.0, 100.0, 138.0];
    fn sf_tracks() -> Vec<(f32, Vec<f32>)> {
        let track = |value: f32, values: [i16; 21]| (value, values.iter().map(|v| f32::from(*v)).collect());
        vec![
            track(-1.0, [54, 10, -4, -16, -28, -40, -50, -60, -68, -80, -76, -58, -34, 0, -2, -7, -14, -21, -28, -28, -28]),
            track(0.0, [82, 38, 24, 12, 0, -12, -22, -32, -40, -52, -46, -24, 6, 28, 26, 21, 14, 7, 0, 0, 0]),
            track(1.0, [110, 66, 52, 40, 28, 16, 6, -4, -12, -24, -18, 4, 34, 56, 54, 49, 42, 35, 28, 28, 28]),
        ]
    }

    #[test]
    fn a_tracking_table_is_read_in_track_0_between_its_sizes_as_harfrust_reads_it() {
        let close = |got: f32, want: f32| (got - want).abs() < 1e-4;
        let tracks = sf_tracks();
        let at = |points: f32| super::normal_tracking(&tracks, &SF_SIZES, points);
        assert!(close(at(12.0), 0.0) && close(at(28.0), 28.0), "a listed size reads its own value: {} {}", at(12.0), at(28.0));
        assert!(close(at(12.5), -6.0) && close(at(18.0), -50.0), "a size between two reads between their values: {} {}", at(12.5), at(18.0));
        assert!(close(at(4.0), 82.0) && close(at(200.0), 0.0), "beyond the sizes, the nearest end's: {} {}", at(4.0), at(200.0));
        let around = [(-1.0, vec![-10.0; 21]), (1.0, vec![30.0; 21])];
        assert!(close(super::normal_tracking(&around, &SF_SIZES, 12.0), 10.0), "with no track at 0, the two around it interpolated");
        assert!(close(super::normal_tracking(&[(2.0, vec![5.0; 21])], &SF_SIZES, 12.0), 5.0), "a single track is the track");
        assert!(close(super::normal_tracking(&[], &SF_SIZES, 12.0), 0.0), "no track tracks nothing");
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn the_system_ui_font_is_spaced_on_a_page_by_its_tracking_at_12_points_less_its_optical_sizes() {
        let data = std::fs::read("/System/Library/Fonts/SFNS.ttf").expect("macOS has SFNS.ttf");
        let spacing = super::page_tracking(&data, 0).expect("SFNS.ttf reads").expect("SFNS.ttf has a tracking table and an optical size axis");
        assert!(spacing < 0.0 && spacing > -0.05, "{spacing}");
    }

    #[cfg(any(windows, target_vendor = "apple"))]
    #[test]
    fn windows_and_macos_name_a_font_for_each_kind_of_text() {
        let found = super::wanted();
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        assert!(!found.ui.is_empty() && !found.mono.is_empty(), "{found:?}");
    }

    /// The families `fontconfig_families` reads from a configuration of
    /// only `aliases`.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn families_of(aliases: &str) -> FontconfigFamilies {
        let file = std::env::temp_dir().join(format!("pwrs-gui-proto-fonts-{}-{}.conf", std::process::id(), aliases.len()));
        std::fs::write(&file, format!("<?xml version=\"1.0\"?><fontconfig>{aliases}</fontconfig>")).expect("a configuration to read");
        let read = fontconfig_families(&file, &[]);
        std::fs::remove_file(&file).expect("the configuration removed");
        read.expect("the families")
    }

    /// The orders asserted are fontconfig 2.15's own for these aliases, as
    /// `fc-pattern --config 'sans\-serif'` prints them on Ubuntu 24.04:
    /// `"A" "B" "sans-serif" "C" "D"`, and `"P1" "P2" "sans-serif" "C2" "C1"
    /// "D1" "D2"`.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    #[test]
    fn fontconfig_tries_preferences_in_alias_order_then_the_last_alias_accepted_first_then_defaults_in_order() {
        let found = families_of(
            "<alias><family>sans-serif</family><prefer><family>A</family></prefer><default><family>D</family></default></alias>\
             <alias><family>monospace</family><prefer><family>M</family></prefer></alias>\
             <alias><family>sans-serif</family><prefer><family>B</family></prefer><accept><family>C</family></accept></alias>",
        );
        assert_eq!(found.sans, ["A", "B", "C", "D"]);
        assert_eq!(found.mono, ["M"]);
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let both = families_of(
            "<alias><family>sans-serif</family><prefer><family>P1</family></prefer><accept><family>C1</family></accept><default><family>D1</family></default></alias>\
             <alias><family>sans-serif</family><prefer><family>P2</family></prefer><accept><family>C2</family></accept><default><family>D2</family></default></alias>",
        );
        assert_eq!(both.sans, ["P1", "P2", "C2", "C1", "D1", "D2"]);
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    #[test]
    fn fontconfig_finds_a_relative_include_in_its_folders_and_reads_only_numbered_conf_files() {
        let root = std::env::temp_dir().join(format!("pwrs-gui-proto-fontconfig-{}", std::process::id()));
        std::fs::create_dir_all(root.join("conf.d")).expect("a configuration folder");
        let alias = |family: &str, prefer: &str| format!("<?xml version=\"1.0\"?><fontconfig><alias><family>{family}</family><prefer><family>{prefer}</family></prefer></alias></fontconfig>");
        let files = [
            ("fonts.conf", String::from("<?xml version=\"1.0\"?><fontconfig><include ignore_missing=\"yes\">conf.d</include><include ignore_missing=\"yes\">gone.d</include><include>gone.conf</include></fontconfig>")),
            ("conf.d/60-latin.conf", alias("sans-serif", "A")),
            ("conf.d/61-mono.conf", alias("monospace", "M")),
            ("conf.d/62-broken.conf", String::from("<fontconfig><alias>")),
            ("conf.d/63-again.conf", String::from("<?xml version=\"1.0\"?><fontconfig><include>conf.d/60-latin.conf</include></fontconfig>")),
            ("conf.d/local.conf", alias("sans-serif", "Unread")),
            ("conf.d/README", String::from("not a configuration")),
        ];
        for (name, text) in &files {
            std::fs::write(root.join(name), text).expect("a configuration file");
        }
        let read = fontconfig_families(&root.join("fonts.conf"), std::slice::from_ref(&root));
        for (name, _) in &files {
            std::fs::remove_file(root.join(name)).expect("a configuration file removed");
        }
        std::fs::remove_dir(root.join("conf.d")).expect("the conf.d folder removed");
        std::fs::remove_dir(&root).expect("the configuration folder removed");
        let found = read.expect("the families");
        assert_eq!(found.sans, ["A"]);
        assert_eq!(found.mono, ["M"]);
        let problems = &found.problems;
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("62-broken.conf")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("gone.conf")), "{problems:?}");
    }
}
