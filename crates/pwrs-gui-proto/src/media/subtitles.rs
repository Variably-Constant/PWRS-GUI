//! A clip's subtitles: the cues of a sidecar SubRip (`.srt`) or WebVTT
//! (`.vtt`) file, or of a track in the clip its player reports, each a
//! stretch of time and its text. Both renderers draw the cues showing at a
//! player's position alike, from the shared tokens.

use crate::Value;
use std::collections::BTreeMap;

/// A cue: its text, shown from `start` to `end` seconds of the clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// The cues of a SubRip or WebVTT file's text, told apart by WebVTT's
/// `WEBVTT` header; or why the text holds none.
pub fn parse(text: &str) -> Result<Vec<Cue>, String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let vtt = text.starts_with("WEBVTT");
    let mut cues = Vec::new();
    for (n, block) in text.split("\n\n").enumerate() {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(timing) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let (from, to) = lines[timing].split_once("-->").ok_or_else(|| format!("block {} has no -->", n + 1))?;
        // WebVTT's settings follow the end time on its line.
        let to = to.split_whitespace().next().ok_or_else(|| format!("block {}'s timing {:?} has no end", n + 1, lines[timing]))?;
        let start = time(from.trim()).map_err(|e| format!("block {}: {e}", n + 1))?;
        let end = time(to).map_err(|e| format!("block {}: {e}", n + 1))?;
        let words: Vec<String> = lines[timing + 1..].iter().map(|l| plain(l)).collect();
        if !words.is_empty() {
            cues.push(Cue { start, end, text: words.join("\n") });
        }
    }
    if cues.is_empty() {
        let kind = if vtt { "WebVTT" } else { "SubRip" };
        return Err(format!("the {kind} text holds no cue: no block has a timing line with -->"));
    }
    Ok(cues)
}

/// The seconds a cue's timing gives, as `01:02:03,456` in SubRip or
/// `01:02:03.456` or `02:03.456` in WebVTT.
fn time(given: &str) -> Result<f64, String> {
    let parts: Vec<&str> = given.split(':').collect();
    let number = |s: &str| s.replace(',', ".").parse::<f64>().map_err(|e| format!("{given:?} is no time: {e}"));
    match parts.as_slice() {
        [h, m, s] => Ok(number(h)? * 3600.0 + number(m)? * 60.0 + number(s)?),
        [m, s] => Ok(number(m)? * 60.0 + number(s)?),
        _ => Err(format!("{given:?} is no time: it is hours, minutes and seconds")),
    }
}

/// A cue's line as shown: its markup, as `<i>` and WebVTT's voice and
/// class spans, or the Pango markup a player's text track gives, taken
/// away.
pub fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in line.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ")
}

/// The texts of the cues showing at `seconds`, in their order.
pub fn showing(cues: &[Cue], seconds: f64) -> Vec<&str> {
    cues.iter().filter(|c| c.start <= seconds && seconds < c.end).map(|c| c.text.as_str()).collect()
}

/// The cues as a MediaPlayer's `subtitles` holds them: a list of maps of
/// each one's `start`, `end` and `text`.
pub fn value(cues: &[Cue]) -> Value {
    Value::List(
        cues.iter()
            .map(|c| Value::Map(BTreeMap::from([(String::from("start"), Value::Float(c.start)), (String::from("end"), Value::Float(c.end)), (String::from("text"), Value::Str(c.text.clone()))])))
            .collect(),
    )
}

/// The cues a MediaPlayer's `subtitles` holds, as [`value`] writes them.
pub fn of(value: &Value) -> Result<Vec<Cue>, String> {
    let Value::List(items) = value else {
        return Err(format!("subtitles is a list of cues, not {value:?}"));
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let m = item.as_map().ok_or_else(|| format!("cue {} is not a map of its start, end and text", i + 1))?;
            let number = |key: &str| m.get(key).and_then(Value::as_number).ok_or_else(|| format!("cue {} has no {key} in seconds", i + 1));
            let text = m.get("text").and_then(Value::as_str).ok_or_else(|| format!("cue {} has no text", i + 1))?;
            Ok(Cue { start: number("start")?, end: number("end")?, text: text.to_string() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subrip_and_webvtt_give_their_cues_without_markup() {
        let srt = "1\r\n00:00:00,500 --> 00:00:01,250\r\n<i>Hello</i>\r\nthere\r\n\r\n2\r\n00:00:01,500 --> 00:00:02,000\r\nBye &amp; thanks\r\n";
        let cues = parse(srt).expect("cues");
        assert_eq!(cues, vec![Cue { start: 0.5, end: 1.25, text: String::from("Hello\nthere") }, Cue { start: 1.5, end: 2.0, text: String::from("Bye & thanks") }]);
        let vtt = "WEBVTT\n\nintro\n00:00.250 --> 00:01.000 align:start\n<v Ann>Hi</v>\n";
        assert_eq!(parse(vtt).expect("cues"), vec![Cue { start: 0.25, end: 1.0, text: String::from("Hi") }]);
        assert_eq!(showing(&cues, 0.6), vec!["Hello\nthere"]);
        assert!(showing(&cues, 1.3).is_empty());
        assert_eq!(of(&value(&cues)).expect("cues"), cues);
        assert!(parse("no cues here").is_err());
        assert!(parse("1\n00:00:xx,000 --> 00:00:01,000\nHi\n").is_err());
    }
}
