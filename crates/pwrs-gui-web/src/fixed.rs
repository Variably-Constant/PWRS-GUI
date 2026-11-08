//! WebView2's Fixed Version runtime, which the `PWRS-GUI.WebView2.win-x64`
//! add-on includes: the version a runtime folder contains, and the environment
//! every webview of a renderer runs in when it runs on one, made once with
//! the runtime's folder and the options wry gives the evergreen runtime.

use std::path::Path;
use webview2_com::Microsoft::Web::WebView2::Win32::{CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2Environment, ICoreWebView2EnvironmentOptions};
use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler};
use windows::core::{HSTRING, PWSTR};

/// The features wry turns off in the evergreen runtime's browser, which a
/// fixed runtime's turns off too: Edge's mini menu, its PDF menu and
/// SmartScreen.
const BROWSER_ARGUMENTS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// The version of the runtime in `folder`, as WebView2 reads it there.
pub(crate) fn version(folder: &Path) -> Result<String, String> {
    let mut info = PWSTR::null();
    let at = HSTRING::from(folder.as_os_str());
    unsafe { GetAvailableCoreWebView2BrowserVersionString(&at, &mut info) }.map_err(|e| format!("the WebView2 runtime in {} gives no version: {}", folder.display(), e.message()))?;
    if info.is_null() {
        return Err(format!("{} has no WebView2 runtime", folder.display()));
    }
    Ok(webview2_com::take_pwstr(info))
}

/// The language the user's Windows shows itself in, as WebView2 takes it.
fn user_language() -> Result<String, String> {
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME};
    let mut name = [0u16; MAX_LOCALE_NAME as usize];
    let lcid = unsafe { GetUserDefaultUILanguage() };
    let written = unsafe { LCIDToLocaleName(u32::from(lcid), Some(&mut name), LOCALE_ALLOW_NEUTRAL_NAMES) };
    if written <= 0 {
        return Err(format!("Windows gave no name for the user's language {lcid:#06x}: {}", windows::core::Error::from_thread().message()));
    }
    let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    Ok(String::from_utf16_lossy(&name[..end]))
}

/// The environment of the runtime in `folder`, storing its data in `data`,
/// or WebView2's own folder beside the executable when `None`. It is made on
/// the event loop's thread, in the single-threaded apartment WebView2 takes,
/// as wry enters it before making an environment of its own.
pub(crate) fn environment(folder: &Path, data: Option<&Path>) -> Result<ICoreWebView2Environment, String> {
    use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if entered.is_err() {
        return Err(format!("this thread cannot enter the apartment WebView2 runs in: {}", entered.message()));
    }
    let options = CoreWebView2EnvironmentOptions::default();
    let language = user_language()?;
    unsafe {
        options.set_additional_browser_arguments(String::from(BROWSER_ARGUMENTS));
        options.set_language(language);
    }
    let browser = HSTRING::from(folder.as_os_str());
    let data = match data {
        Some(d) => HSTRING::from(d.as_os_str()),
        None => HSTRING::new(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |code, environment| {
        let made = code.and_then(|()| environment.ok_or_else(|| windows::core::Error::new(E_POINTER, "WebView2 returned no environment")));
        tx.send(made).map_err(|unsent| windows::core::Error::new(E_UNEXPECTED, format!("the environment came after its waiter left: {unsent}")))
    }));
    unsafe { CreateCoreWebView2EnvironmentWithOptions(&browser, &data, &ICoreWebView2EnvironmentOptions::from(options), &handler) }
        .map_err(|e| format!("the WebView2 runtime in {} did not start an environment: {}", folder.display(), e.message()))?;
    webview2_com::wait_with_pump(rx)
        .map_err(|e| format!("the WebView2 runtime in {} never finished making its environment: {e}", folder.display()))?
        .map_err(|e| format!("the WebView2 runtime in {} made no environment: {}", folder.display(), e.message()))
}
