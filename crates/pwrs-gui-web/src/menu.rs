//! The menus the HTML helper gives macOS. A webview takes a Command
//! shortcut only through its app's menu: WebKit offers the key to the page,
//! and a key the page leaves goes back to the app, where the menu item with
//! that shortcut sends its action, such as copy: or paste:, to the key
//! window. So the helper has the menus every Mac app has: the app menu,
//! with the items winit gives the egui helper, and the Edit menu
//! ([`install`]). A test's Command shortcut reaches the same items
//! ([`act`]). The egui helper has no Edit menu, since egui takes these
//! shortcuts itself.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSProcessInfo, NSString};

/// The modifier keys a menu item's shortcut is told apart by.
fn modifiers() -> NSEventModifierFlags {
    NSEventModifierFlags::Shift | NSEventModifierFlags::Control | NSEventModifierFlags::Option | NSEventModifierFlags::Command
}

/// Gives the app its menus: the app menu, named after the process, with
/// About, Services, Hide, Hide Others, Show All and Quit, and the Edit menu
/// with Undo, Redo, Cut, Copy, Paste and Select All, each with the shortcut
/// Mac apps give it. Refused off the main thread, where AppKit makes no
/// menus.
pub(crate) fn install() -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(String::from("AppKit makes menus on the main thread alone, and this is another"));
    };
    let app = NSApplication::sharedApplication(mtm);
    let name = NSProcessInfo::processInfo().processName().to_string();
    let command = NSEventModifierFlags::Command;
    let services = NSMenu::new(mtm);
    let services_item = item(mtm, "Services", None, "", command);
    services_item.setSubmenu(Some(&services));
    let app_menu = menu(
        mtm,
        &name,
        [
            item(mtm, &format!("About {name}"), Some(sel!(orderFrontStandardAboutPanel:)), "", command),
            NSMenuItem::separatorItem(mtm),
            services_item,
            item(mtm, &format!("Hide {name}"), Some(sel!(hide:)), "h", command),
            item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h", command | NSEventModifierFlags::Option),
            item(mtm, "Show All", Some(sel!(unhideAllApplications:)), "", command),
            NSMenuItem::separatorItem(mtm),
            item(mtm, &format!("Quit {name}"), Some(sel!(terminate:)), "q", command),
        ],
    );
    let edit_menu = menu(
        mtm,
        "Edit",
        [
            item(mtm, "Undo", Some(sel!(undo:)), "z", command),
            item(mtm, "Redo", Some(sel!(redo:)), "z", command | NSEventModifierFlags::Shift),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Cut", Some(sel!(cut:)), "x", command),
            item(mtm, "Copy", Some(sel!(copy:)), "c", command),
            item(mtm, "Paste", Some(sel!(paste:)), "v", command),
            item(mtm, "Select All", Some(sel!(selectAll:)), "a", command),
        ],
    );
    let bar = NSMenu::new(mtm);
    for (title, under) in [(name.as_str(), &app_menu), ("Edit", &edit_menu)] {
        let top = item(mtm, title, None, "", command);
        top.setSubmenu(Some(under));
        bar.addItem(&top);
    }
    app.setServicesMenu(Some(&services));
    app.setMainMenu(Some(&bar));
    Ok(())
}

/// A menu titled `title` holding `items` in order.
fn menu<const N: usize>(mtm: MainThreadMarker, title: &str, items: [Retained<NSMenuItem>; N]) -> Retained<NSMenu> {
    let made = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
    for each in &items {
        made.addItem(each);
    }
    made
}

/// A menu item titled `title` that sends `action`, with the shortcut `key`
/// and `held`; none where `key` is empty.
fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str, held: NSEventModifierFlags) -> Retained<NSMenuItem> {
    // SAFETY: each action given is one AppKit's own menus send, a selector
    // its responders and the app define.
    let made = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(title), action, &NSString::from_str(key)) };
    made.setKeyEquivalentModifierMask(held);
    made
}

/// Sends the action of the main menu's item whose shortcut is `characters`
/// with `flags` held, AppKit's modifier flags, as AppKit sends it when the
/// key reaches the menu: to `view` when it takes the action, else along the
/// key window's responder chain to the app, which takes Hide and Quit.
/// Whether an item had that shortcut and something took its action.
pub(crate) fn act(view: &NSView, characters: &str, flags: usize) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(bar) = app.mainMenu() else {
        return false;
    };
    let held = NSEventModifierFlags::from_bits_retain(flags) & modifiers();
    let Some(chosen) = find(&bar, &characters.to_lowercase(), held) else {
        return false;
    };
    let Some(action) = chosen.action() else {
        return false;
    };
    // SAFETY: respondsToSelector: is NSObject's, which every view is.
    let takes: Bool = unsafe { msg_send![view, respondsToSelector: action] };
    let target: &AnyObject = view;
    let sender: &AnyObject = &chosen;
    // SAFETY: the action is the item's own, sent with the item as its
    // sender, as its menu sends it.
    unsafe { app.sendAction_to_from(action, takes.as_bool().then_some(target), Some(sender)) }
}

/// The item of `menu`, or of a menu under it, whose shortcut is `key` with
/// `held`: the first, in the order the menus show them.
fn find(menu: &NSMenu, key: &str, held: NSEventModifierFlags) -> Option<Retained<NSMenuItem>> {
    menu.itemArray().iter().find_map(|each| match each.submenu() {
        Some(under) => find(&under, key, held),
        None => (!key.is_empty() && each.keyEquivalent().to_string() == key && (each.keyEquivalentModifierMask() & modifiers()) == held).then_some(each),
    })
}
