//! The engine's own chrome, which the web helper sets on each webview as it
//! is built, so a page acts alike in every engine and as a window does in
//! egui: WebView2 with no browser accelerator keys, zooming by keys or by
//! pinching, swipe navigation, status bar or default context menu;
//! WebKitGTK with no navigation gestures or context menu, and the families
//! egui draws in as its default fonts; WKWebView with no magnification or
//! navigation gestures. A page whose developer tools are allowed has the
//! engine's developer keys and default context menu. [`settings`] reads
//! each back from the webview, [`folder`] says where the engine's code was
//! loaded from, and [`floor_note`] compares the engine's version with the
//! oldest the test matrix runs.

use pwrs_gui_proto::Value;
use std::collections::BTreeMap;
use wry::{WebView, WebViewBuilder};

/// The oldest WebView2 the test matrix runs, by its major version: the one
/// the release's gates ran.
pub(crate) const WEBVIEW2_FLOOR: u32 = 154;
/// The oldest WebKit the test matrix runs, by its major version without the
/// system's prefix: macOS 15.8's, as WKWebView reports it there. An app's
/// WebKit on macOS 15 does not follow the installed Safari.
pub(crate) const WEBKIT_FLOOR: u32 = 621;
/// The oldest WebKitGTK the test matrix runs, by its major and minor
/// version: FreeBSD 15.0's.
pub(crate) const WEBKITGTK_FLOOR: (u32, u32) = (2, 46);

/// The families WebKitGTK takes as its default fonts: the first of the
/// system's preferred families for each kind of text that it has, as egui
/// finds them; `None` where the system has none of them, which leaves
/// WebKitGTK's own.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DefaultFonts {
    pub ui: Option<String>,
    pub mono: Option<String>,
}

/// What wry sets as the webview is built: no zooming by keys or pinching
/// and no navigation gestures, and on Windows the browser's accelerator
/// keys and default context menu only where the developer tools are
/// allowed.
#[cfg(windows)]
pub(crate) fn builder(builder: WebViewBuilder<'_>, dev_tools: bool) -> WebViewBuilder<'_> {
    use wry::WebViewBuilderExtWindows;
    builder.with_hotkeys_zoom(false).with_back_forward_navigation_gestures(false).with_browser_accelerator_keys(dev_tools).with_default_context_menus(dev_tools)
}

/// What wry sets as the webview is built: no navigation gestures; the
/// context menu and the rest are set once it is built ([`after_build`]).
#[cfg(not(windows))]
pub(crate) fn builder(builder: WebViewBuilder<'_>, _dev_tools: bool) -> WebViewBuilder<'_> {
    builder.with_hotkeys_zoom(false).with_back_forward_navigation_gestures(false)
}

/// WebView2 takes all of its settings as the webview is built.
#[cfg(windows)]
pub(crate) fn after_build(_webview: &WebView, _dev_tools: bool, _fonts: &DefaultFonts) -> Result<(), String> {
    Ok(())
}

/// Stops WebKitGTK's context menu from showing unless the developer tools
/// are allowed, and gives it `fonts` as its default families.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn after_build(webview: &WebView, dev_tools: bool, fonts: &DefaultFonts) -> Result<(), String> {
    use webkit2gtk::{SettingsExt, WebViewExt};
    use wry::WebViewExtUnix;
    let view = webview.webview();
    if !dev_tools {
        view.connect_context_menu(|_, _, _, _| true);
    }
    let settings = WebViewExt::settings(&view).ok_or("WebKitGTK reports no settings for the web view, so its default fonts are its own")?;
    if let Some(ui) = &fonts.ui {
        settings.set_default_font_family(ui);
        settings.set_sans_serif_font_family(ui);
    }
    if let Some(mono) = &fonts.mono {
        settings.set_monospace_font_family(mono);
    }
    Ok(())
}

/// Turns WKWebView's magnification off; wry has turned its navigation
/// gestures off.
#[cfg(target_os = "macos")]
pub(crate) fn after_build(webview: &WebView, _dev_tools: bool, _fonts: &DefaultFonts) -> Result<(), String> {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    // SAFETY: the webview is used on the event loop's thread, the main one.
    unsafe { view.setAllowsMagnification(false) };
    Ok(())
}

/// Each chrome setting the helper sets, as WebView2 reports it now, by the
/// name of its property in WebView2's settings.
#[cfg(windows)]
pub(crate) fn settings(webview: &WebView) -> Result<BTreeMap<String, Value>, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings5, ICoreWebView2Settings6};
    use windows::core::{BOOL, Interface};
    use wry::WebViewExtWindows;
    let unread = |name: &'static str| move |e: windows::core::Error| format!("WebView2 did not report {name}: {}", e.message());
    let core = webview.webview();
    let s = unsafe { core.Settings() }.map_err(unread("its settings"))?;
    let s3: ICoreWebView2Settings3 = s.cast().map_err(unread("AreBrowserAcceleratorKeysEnabled"))?;
    let s5: ICoreWebView2Settings5 = s.cast().map_err(unread("IsPinchZoomEnabled"))?;
    let s6: ICoreWebView2Settings6 = s.cast().map_err(unread("IsSwipeNavigationEnabled"))?;
    let (mut keys, mut zoom, mut pinch, mut swipe, mut status, mut menus, mut tools) =
        (BOOL::default(), BOOL::default(), BOOL::default(), BOOL::default(), BOOL::default(), BOOL::default(), BOOL::default());
    unsafe {
        s3.AreBrowserAcceleratorKeysEnabled(&mut keys).map_err(unread("AreBrowserAcceleratorKeysEnabled"))?;
        s.IsZoomControlEnabled(&mut zoom).map_err(unread("IsZoomControlEnabled"))?;
        s5.IsPinchZoomEnabled(&mut pinch).map_err(unread("IsPinchZoomEnabled"))?;
        s6.IsSwipeNavigationEnabled(&mut swipe).map_err(unread("IsSwipeNavigationEnabled"))?;
        s.IsStatusBarEnabled(&mut status).map_err(unread("IsStatusBarEnabled"))?;
        s.AreDefaultContextMenusEnabled(&mut menus).map_err(unread("AreDefaultContextMenusEnabled"))?;
        s.AreDevToolsEnabled(&mut tools).map_err(unread("AreDevToolsEnabled"))?;
    }
    Ok(BTreeMap::from([
        (String::from("AreBrowserAcceleratorKeysEnabled"), Value::Bool(keys.as_bool())),
        (String::from("IsZoomControlEnabled"), Value::Bool(zoom.as_bool())),
        (String::from("IsPinchZoomEnabled"), Value::Bool(pinch.as_bool())),
        (String::from("IsSwipeNavigationEnabled"), Value::Bool(swipe.as_bool())),
        (String::from("IsStatusBarEnabled"), Value::Bool(status.as_bool())),
        (String::from("AreDefaultContextMenusEnabled"), Value::Bool(menus.as_bool())),
        (String::from("AreDevToolsEnabled"), Value::Bool(tools.as_bool())),
    ]))
}

/// Each chrome setting the helper sets, as WebKitGTK reports it now, by the
/// name of its property in WebKitGTK's settings; `default-context-menu` is
/// whether nothing stops the web view's context menu, read from its
/// `context-menu` signal. A font family WebKitGTK reports none for is null.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn settings(webview: &WebView) -> Result<BTreeMap<String, Value>, String> {
    use webkit2gtk::glib::object::ObjectType;
    use webkit2gtk::glib::translate::IntoGlib;
    use webkit2gtk::glib::{StaticType, gobject_ffi};
    use webkit2gtk::{SettingsExt, WebViewExt};
    use wry::WebViewExtUnix;
    let view = webview.webview();
    let settings = WebViewExt::settings(&view).ok_or("WebKitGTK reports no settings for the web view")?;
    let font = |f: Option<webkit2gtk::glib::GString>| match f {
        Some(family) => Value::Str(family.to_string()),
        None => Value::Null,
    };
    let name = std::ffi::CString::new("context-menu").map_err(|e| format!("the signal's name cannot be passed to GObject: {e}"))?;
    // SAFETY: the signal is looked up on WebKitWebView's own type, and the
    // instance queried is a live web view of that type.
    let (id, held) = unsafe {
        let id = gobject_ffi::g_signal_lookup(name.as_ptr(), webkit2gtk::WebView::static_type().into_glib());
        (id, id != 0 && gobject_ffi::g_signal_has_handler_pending(view.as_ptr().cast(), id, 0, 0) != 0)
    };
    if id == 0 {
        return Err(String::from("WebKitGTK's web view has no context-menu signal"));
    }
    Ok(BTreeMap::from([
        (String::from("enable-back-forward-navigation-gestures"), Value::Bool(settings.enables_back_forward_navigation_gestures())),
        (String::from("enable-developer-extras"), Value::Bool(settings.enables_developer_extras())),
        (String::from("default-context-menu"), Value::Bool(!held)),
        (String::from("default-font-family"), font(settings.default_font_family())),
        (String::from("sans-serif-font-family"), font(settings.sans_serif_font_family())),
        (String::from("monospace-font-family"), font(settings.monospace_font_family())),
    ]))
}

/// Each chrome setting the helper sets, as WKWebView reports it now, by the
/// name of its property.
#[cfg(target_os = "macos")]
pub(crate) fn settings(webview: &WebView) -> Result<BTreeMap<String, Value>, String> {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    // SAFETY: the webview is used on the event loop's thread, the main one.
    let (magnification, gestures) = unsafe { (view.allowsMagnification(), view.allowsBackForwardNavigationGestures()) };
    Ok(BTreeMap::from([(String::from("allowsMagnification"), Value::Bool(magnification)), (String::from("allowsBackForwardNavigationGestures"), Value::Bool(gestures))]))
}

/// The folder the engine drawing `webview` was started from: the folder of
/// WebView2's browser process.
#[cfg(windows)]
pub(crate) fn folder(webview: &WebView) -> Result<String, String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::core::PWSTR;
    use wry::WebViewExtWindows;
    let mut pid = 0u32;
    unsafe { webview.webview().BrowserProcessId(&mut pid) }.map_err(|e| format!("WebView2 did not name its browser process: {}", e.message()))?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(|e| format!("WebView2's browser process {pid} cannot be opened: {}", e.message()))?;
    let mut path = vec![0u16; 32768];
    let mut len = path.len() as u32;
    let named = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len) };
    let closed = unsafe { CloseHandle(process) };
    named.map_err(|e| format!("WebView2's browser process {pid} did not name its file: {}", e.message()))?;
    closed.map_err(|e| format!("WebView2's browser process {pid}'s handle did not close: {}", e.message()))?;
    let file = std::path::PathBuf::from(String::from_utf16_lossy(&path[..len as usize]));
    file.parent().map(|f| f.display().to_string()).ok_or_else(|| format!("WebView2's browser process runs {}, which is in no folder", file.display()))
}

/// The folder WebKitGTK's library was loaded from.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn folder(_webview: &WebView) -> Result<String, String> {
    let mut info = std::mem::MaybeUninit::<libc::Dl_info>::zeroed();
    let address = webkit2gtk::ffi::webkit_get_major_version as *const libc::c_void;
    // SAFETY: the address is a function of the loaded library, and the
    // information block is written in full when dladdr succeeds.
    let found = unsafe { libc::dladdr(address, info.as_mut_ptr()) };
    if found == 0 {
        return Err(String::from("the loader did not say which file WebKitGTK was loaded from"));
    }
    let info = unsafe { info.assume_init() };
    if info.dli_fname.is_null() {
        return Err(String::from("the loader named no file for WebKitGTK"));
    }
    // SAFETY: dladdr returned a file name it owns, terminated by a nul.
    let file = std::path::PathBuf::from(unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned());
    file.parent().map(|f| f.display().to_string()).ok_or_else(|| format!("WebKitGTK was loaded from {}, which is in no folder", file.display()))
}

/// The folder of the WebKit framework WKWebView comes from.
#[cfg(target_os = "macos")]
pub(crate) fn folder(_webview: &WebView) -> Result<String, String> {
    use objc2::ClassType;
    use objc2_foundation::NSBundle;
    use objc2_web_kit::WKWebView;
    // SAFETY: WKWebView's class is a class object the WebKit framework
    // registers, which is what bundleForClass takes.
    let bundle = unsafe { NSBundle::bundleForClass(WKWebView::class()) };
    Ok(bundle.bundlePath().to_string())
}

/// Part `at` of a dotted version, counted from 0, as a number.
fn part(version: &str, at: usize) -> Result<u32, String> {
    let text = version.split('.').nth(at).ok_or_else(|| format!("it has no part {}", at + 1))?;
    text.trim().parse().map_err(|e| format!("its part '{text}' is not a number ({e})"))
}

/// A warning naming `version`, the engine's, and the floor, when the engine
/// is older than the oldest the test matrix runs, or when its version
/// cannot be read; none when it is not older.
pub(crate) fn floor_note(engine: &str, version: &str) -> Option<String> {
    let held = match engine {
        "WebView2" => part(version, 0).map(|major| (major < WEBVIEW2_FLOOR, format!("{WEBVIEW2_FLOOR}"))),
        "WebKit" => part(version, 0).map(|build| (build % 1000 < WEBKIT_FLOOR, format!("{WEBKIT_FLOOR}, macOS 15.8's"))),
        _ => part(version, 0).and_then(|major| part(version, 1).map(|minor| ((major, minor) < WEBKITGTK_FLOOR, format!("{}.{}", WEBKITGTK_FLOOR.0, WEBKITGTK_FLOOR.1)))),
    };
    match held {
        Ok((false, _)) => None,
        Ok((true, floor)) => Some(format!("this {engine}, {version}, is older than {floor}, the oldest PWRS-GUI's tests run, so pages may not draw or act as they should; windows still open")),
        Err(why) => Some(format!("this {engine}'s version, '{version}', cannot be read ({why}), so whether it is older than the oldest PWRS-GUI's tests run is not known; windows still open")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_older_than_the_floor_is_named_with_the_floor_and_others_are_not() {
        assert_eq!(floor_note("WebView2", "154.0.4258.62"), None);
        assert_eq!(floor_note("WebView2", "160.0.1.2"), None);
        let old = floor_note("WebView2", "141.0.3537.71").expect("141 is older than the floor");
        assert!(old.contains("141.0.3537.71") && old.contains("154") && old.contains("still open"), "{old}");
        assert_eq!(floor_note("WebKit", "20621.3.11.11.3"), None, "macOS 15.8's WebKit is the floor");
        assert_eq!(floor_note("WebKit", "21624.5.1.11.3"), None, "macOS 26.6's is newer");
        let older = floor_note("WebKit", "19617.1.17.11.12").expect("WebKit 617 is older");
        assert!(older.contains("621, macOS 15.8's") && older.contains("still open"), "{older}");
        assert!(floor_note("WebKit", "20620.9.1").is_some(), "one below the floor is warned of");
        assert_eq!(floor_note("WebKitGTK", "2.46.6"), None);
        assert_eq!(floor_note("WebKitGTK", "2.52.6"), None);
        assert!(floor_note("WebKitGTK", "2.44.3").expect("2.44 is older").contains("2.46"));
        let unread = floor_note("WebKitGTK", "unknown").expect("an unreadable version is said");
        assert!(unread.contains("cannot be read") && unread.contains("'unknown' is not a number"), "{unread}");
    }
}
