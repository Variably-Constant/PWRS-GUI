//! Files dropped on a page of the caller's own, which keeps every DOM drop
//! event, its own drags included. Once the page's own handlers have seen a
//! drop of files, its bridge tells the helper of it ([`Dropped`]). On
//! Windows that message carries the files themselves, as WebView2's
//! additional objects, whose full paths [`watch`] reads. Elsewhere wry's
//! drop handler, which hands the drop on to the engine, gives the paths
//! first, and the page's message follows them.

use serde_json::Value as Json;

/// A drop of files on a page, as its bridge reports it: whether the page
/// took it with preventDefault, and where, in points from the top left of
/// its window's inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Dropped {
    pub(super) taken: bool,
    pub(super) at: (f64, f64),
}

impl Dropped {
    /// The drop a page's `drop` message describes.
    pub(super) fn of(msg: &Json) -> Result<Dropped, String> {
        let taken = match msg.get("taken").and_then(Json::as_bool) {
            Some(taken) => taken,
            None => return Err(format!("a page's drop says not whether the page took it: {msg}")),
        };
        let at = |key: &str| match msg.get(key).and_then(Json::as_f64) {
            Some(v) if v.is_finite() => Ok(v),
            Some(_) | None => Err(format!("a page's drop has no {key} that is a number: {msg}")),
        };
        Ok(Dropped { taken, at: (at("x")?, at("y")?) })
    }

    /// What the page's `window.pwrs.ondrop` handlers are given: the full
    /// paths dropped, where the drop landed, and whether the page took it.
    pub(super) fn for_page(&self, paths: &[String]) -> Json {
        serde_json::json!({ "paths": paths, "x": self.at.0, "y": self.at.1, "taken": self.taken })
    }
}

/// Watches a page's WebView2 for its bridge's drop messages, a JSON object
/// whose additional objects are the files dropped, which wry's own message
/// handler passes over since they are no strings. Each drop from an address
/// `admitted` takes is given to `dropped` with the address, the message and
/// the files' full paths; a file that has no path, as one a test made in
/// the page, is left out.
#[cfg(windows)]
pub(super) fn watch(webview: &wry::WebView, admitted: impl Fn(&str) -> bool + 'static, dropped: impl Fn(&str, Json, Vec<String>) + 'static) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2};
    use webview2_com::{WebMessageReceivedEventHandler, take_pwstr};
    use windows::core::{Interface, PWSTR};
    use wry::WebViewExtWindows;
    let core = webview.webview();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut source = PWSTR::null();
        unsafe { args.Source(&mut source)? };
        let source = take_pwstr(source);
        let mut written = PWSTR::null();
        unsafe { args.WebMessageAsJson(&mut written)? };
        let written = take_pwstr(written);
        let msg: Json = match serde_json::from_str(&written) {
            Ok(m) => m,
            Err(e) => return Err(windows::core::Error::new(windows::core::HRESULT(-1), format!("a page's message is not JSON ({e}): {written}"))),
        };
        if msg.get("t").and_then(Json::as_str) != Some("drop") || !admitted(&source) {
            return Ok(());
        }
        let objects = unsafe { args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>()?.AdditionalObjects()? };
        let mut count = 0u32;
        unsafe { objects.Count(&mut count)? };
        let mut paths = Vec::new();
        for i in 0..count {
            let item = unsafe { objects.GetValueAtIndex(i)? };
            let Ok(file) = item.cast::<ICoreWebView2File>() else {
                continue;
            };
            let mut path = PWSTR::null();
            if unsafe { file.Path(&mut path) }.is_ok() {
                let path = take_pwstr(path);
                if !path.is_empty() {
                    paths.push(path);
                }
            }
        }
        dropped(&source, msg, paths);
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { core.add_WebMessageReceived(&handler, &mut token) }.map_err(|e| format!("WebView2 did not take the handler for the page's dropped files: {}", e.message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_drop_says_whether_the_page_took_it_and_where_it_landed() {
        let dropped = Dropped::of(&json!({ "t": "drop", "taken": false, "x": 12.5, "y": 40 })).expect("a drop");
        assert_eq!(dropped, Dropped { taken: false, at: (12.5, 40.0) });
        let given = dropped.for_page(&[String::from("C:\\a.txt")]);
        assert_eq!(given, json!({ "paths": ["C:\\a.txt"], "x": 12.5, "y": 40.0, "taken": false }));
        assert!(Dropped::of(&json!({ "t": "drop", "x": 1, "y": 2 })).is_err(), "a drop that says not whether it was taken");
        assert!(Dropped::of(&json!({ "t": "drop", "taken": true, "x": "left", "y": 2 })).is_err(), "a place that is no number");
    }
}
