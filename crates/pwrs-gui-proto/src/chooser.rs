//! A FilePicker's dialog: the system dialog a file chooser's node opens,
//! read alike by both renderers.

use crate::{DialogKind, Node, Value};
use std::path::Path;

/// The title and kind of the dialog the file chooser `node` opens: a folder,
/// or with `multiple` folders, when `folder` is set; a file to save when
/// `save` is; else a file, or with `multiple` files, to open; with its
/// `filter`, starting where the first of its chosen paths, `chosen`, is.
/// An empty title leaves the dialog the system's own.
pub fn dialog(node: &Node, chosen: &[String]) -> Result<(String, DialogKind), String> {
    let flag = |key: &str| match node.props.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(format!("{key} must be true or false, not {other:?}")),
    };
    let (folder, save, multiple) = (flag("folder")?, flag("save")?, flag("multiple")?);
    let title = match node.props.get("title") {
        None | Some(Value::Null) => String::new(),
        Some(Value::Str(t)) => t.clone(),
        Some(other) => return Err(format!("title must be text, not {other:?}")),
    };
    let first = chosen.first().map(Path::new);
    let directory = match first {
        Some(first) if folder => Some(first),
        Some(first) => first.parent().filter(|p| !p.as_os_str().is_empty()),
        None => None,
    }
    .map(|d| d.display().to_string());
    let kind = if folder {
        DialogKind::Folder { multiple, directory }
    } else if save {
        let file_name = first.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        DialogKind::Save { filters: filters(node)?, directory, file_name }
    } else {
        DialogKind::Open { multiple, filters: filters(node)?, directory }
    };
    Ok((title, kind))
}

/// A file chooser's `filter`: each kind of file's name and its extensions,
/// without dots, `*` for any file.
fn filters(node: &Node) -> Result<Vec<(String, Vec<String>)>, String> {
    let items = match node.props.get("filter") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::List(items)) => items,
        Some(other) => return Err(format!("filter must be a list, not {other:?}")),
    };
    items
        .iter()
        .map(|f| {
            let map = f.as_map().ok_or_else(|| format!("each filter must be a map of a name and its extensions, not {f:?}"))?;
            let name = map.get("name").and_then(Value::as_str).ok_or_else(|| format!("a filter names its files as text: {f:?}"))?;
            let extensions = match map.get("extensions") {
                Some(Value::List(e)) => e.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("a filter's extensions are text: {f:?}"))).collect::<Result<Vec<String>, String>>()?,
                other => return Err(format!("a filter's extensions are a list, not {other:?}")),
            };
            Ok((name.to_string(), extensions))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;
    use std::collections::BTreeMap;

    fn chooser(props: &[(&str, Value)]) -> Node {
        props.iter().fold(Node::new(1, Kind::FilePicker), |node, (key, value)| node.prop(*key, value.clone()))
    }

    fn logs() -> Value {
        let map = BTreeMap::from([(String::from("name"), Value::Str(String::from("Logs"))), (String::from("extensions"), Value::List(vec![Value::Str(String::from("log"))]))]);
        Value::List(vec![Value::Map(map)])
    }

    #[test]
    fn a_chooser_opens_files_from_where_its_first_path_is() {
        let node = chooser(&[("multiple", Value::Bool(true)), ("filter", logs()), ("title", Value::Str(String::from("Pick logs")))]);
        let (title, kind) = dialog(&node, &[String::from("/var/log/a.log")]).expect("the chooser reads");
        assert_eq!(title, "Pick logs");
        assert_eq!(kind, DialogKind::Open { multiple: true, filters: vec![(String::from("Logs"), vec![String::from("log")])], directory: Some(String::from("/var/log")) });
    }

    #[test]
    fn a_save_chooser_fills_in_its_file_name_and_a_folder_chooser_starts_in_its_folder() {
        let (_, save) = dialog(&chooser(&[("save", Value::Bool(true))]), &[String::from("/tmp/report.csv")]).expect("the save chooser reads");
        assert_eq!(save, DialogKind::Save { filters: Vec::new(), directory: Some(String::from("/tmp")), file_name: Some(String::from("report.csv")) });
        let (title, folder) = dialog(&chooser(&[("folder", Value::Bool(true)), ("save", Value::Bool(true))]), &[String::from("/tmp")]).expect("the folder chooser reads");
        assert_eq!(title, "", "no title leaves the system's own");
        assert_eq!(folder, DialogKind::Folder { multiple: false, directory: Some(String::from("/tmp")) });
    }

    #[test]
    fn a_bare_file_name_starts_nowhere_in_particular() {
        let (_, kind) = dialog(&chooser(&[]), &[String::from("notes.txt")]).expect("the chooser reads");
        assert_eq!(kind, DialogKind::Open { multiple: false, filters: Vec::new(), directory: None });
    }

    #[test]
    fn a_chooser_refuses_a_flag_that_is_not_true_or_false() {
        let refused = dialog(&chooser(&[("folder", Value::Str(String::from("yes")))]), &[]).expect_err("folder is text");
        assert!(refused.contains("folder"), "{refused}");
    }
}
