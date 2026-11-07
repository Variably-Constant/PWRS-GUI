//! What an ELF file's dynamic section says, read from `readelf -dW`, which
//! Linux's binutils and FreeBSD's elftoolchain both print in this form.

use std::path::Path;
use std::process::Command;

/// The dynamic section's names: the libraries it needs, its own soname, and
/// the folders it searches first.
#[derive(Debug, Default, PartialEq)]
pub struct Dynamic {
    pub needed: Vec<String>,
    pub soname: Option<String>,
    pub runpaths: Vec<String>,
}

/// Whether the file at `path` starts with ELF's magic number.
pub fn is_elf(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    let mut file = std::fs::File::open(path).map_err(|e| format!("{} cannot be opened: {e}", path.display()))?;
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == b"\x7fELF"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(format!("{} cannot be read: {e}", path.display())),
    }
}

/// The dynamic section of the ELF file at `path`.
pub fn read(path: &Path) -> Result<Dynamic, String> {
    let out = Command::new("readelf").arg("-dW").arg(path).output().map_err(|e| format!("readelf cannot run: {e}"))?;
    if !out.status.success() {
        return Err(format!("readelf -dW {} failed: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

/// The names in `readelf -dW`'s output, each in the square brackets after
/// its tag.
pub fn parse(text: &str) -> Dynamic {
    let mut dynamic = Dynamic::default();
    for line in text.lines() {
        let Some(value) = bracketed(line) else {
            continue;
        };
        if line.contains("(NEEDED)") {
            dynamic.needed.push(value.to_string());
        } else if line.contains("(SONAME)") {
            dynamic.soname = Some(value.to_string());
        } else if line.contains("(RUNPATH)") || line.contains("(RPATH)") {
            dynamic.runpaths.extend(value.split(':').filter(|p| !p.is_empty()).map(str::to_string));
        }
    }
    dynamic
}

fn bracketed(line: &str) -> Option<&str> {
    let start = line.find('[')? + 1;
    let end = line[start..].find(']')? + start;
    Some(&line[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needed_soname_and_runpath_come_from_their_brackets() {
        let text = " 0x0000000000000001 (NEEDED)             Shared library: [libgtk-3.so.0]\n \
                    0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]\n \
                    0x000000000000000e (SONAME)             Library soname: [libwebkit2gtk-4.1.so.0]\n \
                    0x000000000000001d (RUNPATH)            Library runpath: [$ORIGIN/../lib:/opt/x]\n";
        let d = parse(text);
        assert_eq!(d.needed, vec!["libgtk-3.so.0", "libc.so.6"]);
        assert_eq!(d.soname.as_deref(), Some("libwebkit2gtk-4.1.so.0"));
        assert_eq!(d.runpaths, vec!["$ORIGIN/../lib", "/opt/x"]);
    }
}
