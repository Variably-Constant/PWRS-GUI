//! What the session and every renderer read a picture's bytes as.

use crate::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Whether `bytes` are an SVG document: after a byte order mark and white
/// space they open with a tag, and an `svg` element starts in them.
pub fn is_svg(bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let opens = text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<');
    opens && text.windows(4).any(|w| w == b"<svg")
}

/// A picture's `bytes` as a renderer takes them: a map of the bytes and
/// their format, which [`format`] tells.
pub fn value(bytes: Vec<u8>) -> Result<Value, String> {
    let format = format(&bytes)?;
    Ok(Value::Map(BTreeMap::from([(String::from("bytes"), Value::Bytes(Arc::from(bytes))), (String::from("format"), Value::Str(format.to_string()))])))
}

/// The format of a picture's `bytes`, told by how they begin, as the
/// renderers name it: `png`, `jpeg`, `gif`, `webp`, `bmp`, `ico` or `svg`.
/// Anything else is an error naming what the bytes look like.
pub fn format(bytes: &[u8]) -> Result<&'static str, String> {
    let at = |range: std::ops::Range<usize>| bytes.get(range);
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok("gif");
    }
    if at(0..4) == Some(b"RIFF") && at(8..12) == Some(b"WEBP") {
        return Ok("webp");
    }
    if bytes.starts_with(b"BM") && matches!(at(14..18), Some([12 | 40 | 52 | 56 | 64 | 108 | 124, 0, 0, 0])) {
        return Ok("bmp");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && matches!(at(4..6), Some([n, m]) if *n != 0 || *m != 0) {
        return Ok("ico");
    }
    if is_svg(bytes) {
        return Ok("svg");
    }
    let what = if bytes.is_empty() {
        "no bytes at all"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "a TIFF image"
    } else if at(4..8) == Some(b"ftyp") && matches!(at(8..12), Some(b"avif" | b"avis")) {
        "an AVIF image"
    } else if at(4..8) == Some(b"ftyp") && matches!(at(8..12), Some(b"heic" | b"heix" | b"mif1" | b"msf1")) {
        "a HEIF image"
    } else if bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(b"\0\0\0\x0cJXL \r\n\x87\n") {
        "a JPEG XL image"
    } else if bytes.starts_with(b"qoif") {
        "a QOI image"
    } else if bytes.starts_with(b"8BPS") {
        "a Photoshop document"
    } else if bytes.starts_with(b"%PDF") {
        "a PDF document"
    } else if bytes.starts_with(&[0, 0, 2, 0]) {
        "a Windows cursor"
    } else if bytes.starts_with(&[0x1F, 0x8B]) {
        "gzip-compressed, as an .svgz file is"
    } else {
        let start: Vec<String> = bytes.iter().take(8).map(|b| format!("{b:02x}")).collect();
        return Err(format!("the bytes begin {}, which starts no PNG, JPEG, GIF, WebP, BMP, ICO or SVG", start.join(" ")));
    };
    Err(format!("the bytes are {what}; an Image takes PNG, JPEG, GIF, WebP, BMP, ICO or SVG"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_document_is_told_by_its_bytes() {
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(is_svg(b"\xEF\xBB\xBF \n<?xml version=\"1.0\"?>\n<svg/>"));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(b"not <svg"));
    }
}
