//! The pages of the SubEtha Guide as its tests read them: every fenced
//! block with its language, its attributes, the line it opens on and the
//! tab it sits in, and the programs those blocks make up.
//!
//! The build script includes this file by path to extract the programs;
//! the library compiles it as a module for the test.

use std::path::{Path, PathBuf};

/// One fenced block of a page.
#[derive(Clone, Debug)]
pub struct Block {
    /// The line the opening fence is on, from 1.
    pub line: usize,
    /// The info string's first word: `rust`, `python`, `text`. Empty for a
    /// fence that names no language.
    pub lang: String,
    /// The `key="value"` attributes between the braces after the language.
    pub attrs: Vec<(String, String)>,
    /// The lines between the fences, each ending in a newline, with the
    /// fence's own indentation taken off.
    pub body: String,
    /// The tab group the block sits in, numbered in page order.
    pub group: Option<usize>,
    /// The name of the tab it sits in.
    pub tab: Option<String>,
    /// The tab group that closed before this block with no block between
    /// them. An Output block there belongs to every tab of that group.
    pub after_group: Option<usize>,
}

impl Block {
    /// The value of attribute `key`.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Whether this is a block showing what a program prints.
    pub fn is_output(&self) -> bool {
        self.lang == "text" && self.attr("filename") == Some("Output")
    }
}

/// The four surfaces a program is written for, in the order their tabs
/// appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Surface {
    Rust,
    Python,
    PowerShell,
    C,
}

impl Surface {
    /// The surface a fence's language names, if it names one.
    pub fn of_lang(lang: &str) -> Option<Surface> {
        match lang {
            "rust" => Some(Surface::Rust),
            "python" => Some(Surface::Python),
            "powershell" => Some(Surface::PowerShell),
            "c" => Some(Surface::C),
            _ => None,
        }
    }

    /// The name its tab carries.
    pub fn tab_name(self) -> &'static str {
        match self {
            Surface::Rust => "Rust",
            Surface::Python => "Python",
            Surface::PowerShell => "PowerShell",
            Surface::C => "C",
        }
    }
}

/// A program a page shows: a block in one of the four languages that
/// names the file it is saved as.
#[derive(Clone, Debug)]
pub struct Program {
    /// The page, relative to the Guide's content directory, with `/`
    /// between its parts.
    pub page: String,
    pub line: usize,
    pub surface: Surface,
    /// The file name the block's header shows.
    pub filename: String,
    /// The one operating system the program runs on, as `target_os`
    /// names it, when it runs on one only.
    pub os: Option<String>,
    /// Whether the next program starts without waiting for this one.
    pub alongside: bool,
    pub source: String,
    /// What the page shows it printing, and the line that block opens on.
    pub output: Option<(usize, String)>,
}

/// Every fenced block of `text`, in page order. An error names the line
/// of a fence that never closes or of an attribute list that does not
/// parse.
pub fn blocks(text: &str) -> Result<Vec<Block>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut group: Option<usize> = None;
    let mut groups = 0usize;
    let mut tab: Option<String> = None;
    let mut closed: Option<usize> = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            let indent = line.len() - trimmed.len();
            let (lang, attrs) = parse_info(info).map_err(|e| format!("line {}: {e}", i + 1))?;
            let mut body = String::new();
            let mut j = i + 1;
            loop {
                let Some(inner) = lines.get(j) else {
                    return Err(format!("line {}: a fence that never closes", i + 1));
                };
                if inner.trim() == "```" {
                    break;
                }
                let cut = inner.len() - inner.trim_start().len();
                body.push_str(&inner[cut.min(indent)..]);
                body.push('\n');
                j += 1;
            }
            out.push(Block {
                line: i + 1,
                lang,
                attrs,
                body,
                group,
                tab: tab.clone(),
                after_group: if group.is_none() { closed } else { None },
            });
            closed = None;
            i = j + 1;
            continue;
        }
        let shortcode = trimmed.trim_end();
        if shortcode.starts_with("{{< tabs") {
            group = Some(groups);
            groups += 1;
        } else if shortcode == "{{< /tabs >}}" {
            closed = group.take();
            tab = None;
        } else if let Some(rest) = shortcode.strip_prefix("{{< tab ") {
            tab = Some(shortcode_name(rest).ok_or_else(|| format!("line {}: a tab with no name", i + 1))?);
        } else if shortcode == "{{< /tab >}}" {
            tab = None;
        }
        i += 1;
    }
    Ok(out)
}

/// The `name="..."` of a tab shortcode's arguments.
fn shortcode_name(args: &str) -> Option<String> {
    let start = args.find("name=\"")? + "name=\"".len();
    let len = args[start..].find('"')?;
    Some(args[start..start + len].to_string())
}

/// A fence's info string split into its language and its attributes:
/// `rust,no_run {filename="main.rs" os="linux"}` is `rust` and the two
/// pairs.
fn parse_info(info: &str) -> Result<(String, Vec<(String, String)>), String> {
    let (head, braces) = match info.find('{') {
        Some(at) => (&info[..at], Some(&info[at..])),
        None => (info, None),
    };
    let lang = head.split([',', ' ', '\t']).next().unwrap_or("").trim().to_string();
    let mut attrs = Vec::new();
    if let Some(braces) = braces {
        let inner = braces
            .trim_end()
            .strip_prefix('{')
            .and_then(|b| b.strip_suffix('}'))
            .ok_or_else(|| format!("attributes {braces:?} are not one {{...}} list"))?;
        let mut rest = inner.trim_start_matches([' ', ',']);
        while !rest.is_empty() {
            let eq = rest.find('=').ok_or_else(|| format!("attribute {rest:?} has no value"))?;
            let key = rest[..eq].trim().to_string();
            let after = &rest[eq + 1..];
            let (value, tail) = if let Some(quoted) = after.strip_prefix('"') {
                let end = quoted.find('"').ok_or_else(|| format!("attribute {key} has an unclosed quote"))?;
                (quoted[..end].to_string(), &quoted[end + 1..])
            } else {
                let end = after.find([' ', ',']).unwrap_or(after.len());
                (after[..end].to_string(), &after[end..])
            };
            attrs.push((key, value));
            rest = tail.trim_start_matches([' ', ',']);
        }
    }
    Ok((lang, attrs))
}

/// The programs of one page and the Output blocks that belong to them.
///
/// An Output block inside a tab belongs to the program before it in that
/// tab. One placed after a tab group belongs to the last program of every
/// tab in the group. One outside any group belongs to the program before
/// it outside any group. Each takes its programs, so a second Output
/// block has none to belong to. An Output block with no program to belong
/// to, a program in a tab named for another surface, and a program block
/// with an empty file name are errors.
pub fn programs(page: &str, blocks: &[Block]) -> Result<Vec<Program>, String> {
    let mut programs: Vec<Program> = Vec::new();
    let mut last_in_tab: Vec<((usize, String), usize)> = Vec::new();
    let mut last_outside: Option<usize> = None;
    for block in blocks {
        if block.is_output() {
            let mut owners = Vec::new();
            match (block.group, &block.tab, block.after_group) {
                (Some(g), Some(t), _) => {
                    if let Some(at) = last_in_tab.iter().position(|((lg, lt), _)| *lg == g && lt == t) {
                        owners.push(last_in_tab.remove(at).1);
                    }
                }
                (None, _, Some(g)) => {
                    last_in_tab.retain(|((lg, _), p)| {
                        if *lg == g {
                            owners.push(*p);
                            false
                        } else {
                            true
                        }
                    });
                }
                _ => owners.extend(last_outside.take()),
            }
            if owners.is_empty() {
                return Err(format!("{page}:{}: an Output block with no program before it", block.line));
            }
            for p in owners {
                programs[p].output = Some((block.line, block.body.clone()));
            }
            continue;
        }
        let Some(surface) = Surface::of_lang(&block.lang) else { continue };
        let Some(filename) = block.attr("filename") else { continue };
        if filename.trim().is_empty() {
            return Err(format!("{page}:{}: a program with an empty file name", block.line));
        }
        if let Some(tab) = &block.tab
            && tab != surface.tab_name()
        {
            return Err(format!(
                "{page}:{}: a {} program in the {tab} tab",
                block.line,
                surface.tab_name()
            ));
        }
        let index = programs.len();
        programs.push(Program {
            page: page.to_string(),
            line: block.line,
            surface,
            filename: filename.to_string(),
            os: block.attr("os").map(str::to_string),
            alongside: block.attr("alongside") == Some("true"),
            source: block.body.clone(),
            output: None,
        });
        match (block.group, &block.tab) {
            (Some(g), Some(t)) => {
                last_in_tab.retain(|((lg, lt), _)| !(*lg == g && lt == t));
                last_in_tab.push(((g, t.clone()), index));
            }
            _ => last_outside = Some(index),
        }
    }
    Ok(programs)
}

/// Every markdown page under `root`, as (path relative to `root` with `/`
/// between its parts, full path), sorted by the relative path.
pub fn pages(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut found = Vec::new();
    walk(root, root, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("an entry under {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| format!("{} is not under {}: {e}", path.display(), root.display()))?;
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            found.push((rel, path));
        }
    }
    Ok(())
}

/// Text as a page shows it and a program prints it compared: line ends
/// made `\n`, each line's trailing whitespace dropped, and the blank lines
/// at the end dropped.
pub fn normalized(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABBED: &str = "\
{{< tabs >}}
{{< tab name=\"Rust\" >}}
```rust {filename=\"src/main.rs\"}
fn main() {}
```
{{< /tab >}}
{{< tab name=\"Python\" >}}
```python {filename=\"main.py\"}
print(1)
```
```text {filename=\"Output\"}
python only
```
{{< /tab >}}
{{< /tabs >}}

```text {filename=\"Output\"}
every tab
```
";

    #[test]
    fn an_info_string_gives_the_language_and_every_attribute() {
        let (lang, attrs) = parse_info("rust,no_run {filename=\"a b.rs\", os=linux alongside=\"true\"}").unwrap();
        assert_eq!(lang, "rust");
        let pairs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(pairs, [("filename", "a b.rs"), ("os", "linux"), ("alongside", "true")]);
        assert_eq!(parse_info("").unwrap().0, "");
        assert!(parse_info("rust {filename=\"a.rs\"").is_err());
    }

    #[test]
    fn an_output_block_belongs_to_its_tab_or_to_every_tab_of_the_group_before_it() {
        let found = blocks(TABBED).unwrap();
        let programs = programs("p.md", &found).unwrap();
        assert_eq!(programs.len(), 2);
        assert_eq!(programs[0].surface, Surface::Rust);
        assert_eq!(programs[0].output.as_ref().map(|(_, t)| t.as_str()), Some("every tab\n"));
        assert_eq!(programs[1].output.as_ref().map(|(_, t)| t.as_str()), Some("python only\n"));
    }

    #[test]
    fn an_output_block_outside_tabs_belongs_to_the_program_before_it() {
        let page = "```c {filename=\"a.c\"}\nint main(void) { return 0; }\n```\n\n```text {filename=\"Output\"}\nhi\n```\n";
        let programs = programs("p.md", &blocks(page).unwrap()).unwrap();
        assert_eq!(programs[0].output.as_ref().map(|(line, t)| (*line, t.as_str())), Some((5, "hi\n")));
    }

    #[test]
    fn an_output_block_with_no_program_a_second_one_and_a_program_in_the_wrong_tab_are_refused() {
        let orphan = "```text {filename=\"Output\"}\nhi\n```\n";
        assert!(programs("p.md", &blocks(orphan).unwrap()).unwrap_err().contains("no program before it"));
        let twice = "```c {filename=\"a.c\"}\nx\n```\n```text {filename=\"Output\"}\na\n```\n```text {filename=\"Output\"}\nb\n```\n";
        assert!(programs("p.md", &blocks(twice).unwrap()).unwrap_err().contains("no program before it"));
        let misplaced = "{{< tabs >}}\n{{< tab name=\"Python\" >}}\n```rust {filename=\"a.rs\"}\nfn main() {}\n```\n{{< /tab >}}\n{{< /tabs >}}\n";
        assert!(programs("p.md", &blocks(misplaced).unwrap()).unwrap_err().contains("in the Python tab"));
    }

    #[test]
    fn a_fence_that_never_closes_is_refused_with_its_line() {
        assert_eq!(blocks("text\n```rust\nfn main() {}\n").unwrap_err(), "line 2: a fence that never closes");
    }

    #[test]
    fn normalizing_drops_carriage_returns_trailing_whitespace_and_trailing_blank_lines() {
        assert_eq!(normalized("a  \r\nb\t\r\n\r\n\n"), "a\nb");
        assert_eq!(normalized(""), "");
    }
}
