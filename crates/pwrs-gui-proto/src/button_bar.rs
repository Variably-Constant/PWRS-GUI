//! A ButtonBar's rules, which both renderers follow: each button's role in
//! its window's keys, which focused widgets leave Enter and Escape to those
//! buttons, and the order a bar shows its buttons in on each system.

use crate::{Kind, Node, Shortcut, Value};

/// What a button does when a key presses it for the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The window's default button, which Enter presses.
    Default,
    /// The window's cancel button, which Escape presses.
    Cancel,
    /// Neither.
    Other,
}

impl Role {
    /// A Button's role from its `default` and `cancel`; refused when both
    /// are true, or either is not true or false.
    pub fn of(node: &Node) -> Result<Role, String> {
        let flag = |key: &str| match node.props.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(format!("{key} must be true or false, not {other:?}")),
        };
        match (flag("default")?, flag("cancel")?) {
            (true, true) => Err(String::from("a button is the default or the cancel button, not both")),
            (true, false) => Ok(Role::Default),
            (false, true) => Ok(Role::Cancel),
            (false, false) => Ok(Role::Other),
        }
    }
}

/// The button of `role` in `tree`, its default or its cancel button, the
/// first in tree order; a button whose role cannot be read is none of them.
pub fn role_button(tree: &Node, role: Role) -> Option<&Node> {
    if tree.kind == Kind::Button && Role::of(tree).is_ok_and(|r| r == role) {
        return Some(tree);
    }
    tree.children.iter().find_map(|c| role_button(c, role))
}

/// Whether a focused widget uses `key`, Enter or Escape, itself rather than
/// leave it to the window's default or cancel button: as
/// [`Shortcut::widget_keeps`] says, but a text field of one line, a number
/// field and an editable drop-down keep neither, as a Windows dialog's
/// fields do not, a text field of several lines keeps Enter alone, and a
/// list of check boxes and a table that selects nothing keep neither, since
/// Enter activates nothing there ([`crate::rows::row_key`]).
pub fn keeps(node: &Node, key: &str) -> bool {
    let set = |flag: &str| matches!(node.props.get(flag), Some(Value::Bool(true)));
    match node.kind {
        Kind::TextBox => key == "Enter" && set("multiline"),
        Kind::NumberBox => false,
        Kind::ComboBox if set("editable") => false,
        Kind::ListBox if set("multiple") => false,
        Kind::Table if node.props.get("selection").and_then(Value::as_str).is_none_or(|s| s == "none") => false,
        kind => Shortcut::parse(key).is_ok_and(|s| s.widget_keeps(kind, false)),
    }
}

/// Whether the system this is built for puts a dialog's default button
/// first, as Windows does, and draws a strip under its buttons.
pub const WINDOWS_ORDER: bool = cfg!(windows);

/// The order a bar shows buttons of `roles` in, as their places in
/// `roles`. With `windows_order` the default button comes first, then the
/// others in the order given, then the cancel button (OK, Cancel), as
/// Windows' dialogs order them; otherwise the others in the order given,
/// then the cancel button, then the default (Cancel, OK), as macOS's and
/// GTK's dialogs order them.
pub fn order(roles: &[Role], windows_order: bool) -> Vec<usize> {
    let of = |role: Role| roles.iter().enumerate().filter(move |(_, r)| **r == role).map(|(i, _)| i);
    if windows_order {
        of(Role::Default).chain(of(Role::Other)).chain(of(Role::Cancel)).collect()
    } else {
        of(Role::Other).chain(of(Role::Cancel)).chain(of(Role::Default)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(default: bool, cancel: bool) -> Node {
        Node::new(1, Kind::Button).prop("default", Value::Bool(default)).prop("cancel", Value::Bool(cancel))
    }

    #[test]
    fn a_button_is_the_default_the_cancel_or_neither_and_never_both() {
        assert_eq!(Role::of(&button(true, false)), Ok(Role::Default));
        assert_eq!(Role::of(&button(false, true)), Ok(Role::Cancel));
        assert_eq!(Role::of(&Node::new(1, Kind::Button)), Ok(Role::Other));
        assert!(Role::of(&button(true, true)).is_err());
        let odd = Node::new(1, Kind::Button).prop("default", Value::Str(String::from("yes")));
        assert!(Role::of(&odd).is_err());
    }

    #[test]
    fn windows_puts_the_default_first_and_the_others_put_it_last() {
        use Role::{Cancel, Default, Other};
        assert_eq!(order(&[Default, Cancel], true), [0, 1], "OK, Cancel");
        assert_eq!(order(&[Default, Cancel], false), [1, 0], "Cancel, OK");
        assert_eq!(order(&[Cancel, Default], true), [1, 0], "OK first whichever is written first");
        assert_eq!(order(&[Default, Other, Cancel], true), [0, 1, 2], "Save, Don't Save, Cancel");
        assert_eq!(order(&[Default, Other, Cancel], false), [1, 2, 0], "Don't Save, Cancel, Save");
        assert_eq!(order(&[Other, Other], false), [0, 1], "buttons without roles stay in the order given");
    }

    #[test]
    fn a_window_s_role_buttons_are_found_anywhere_in_it() {
        let ok = Node::new(3, Kind::Button).prop("default", Value::Bool(true));
        let cancel = Node::new(4, Kind::Button).prop("cancel", Value::Bool(true));
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Row).child(ok)).child(cancel);
        assert_eq!(role_button(&tree, Role::Default).map(|b| b.id), Some(3));
        assert_eq!(role_button(&tree, Role::Cancel).map(|b| b.id), Some(4));
        assert_eq!(role_button(&Node::new(1, Kind::Column), Role::Default).map(|b| b.id), None);
    }

    #[test]
    fn fields_leave_enter_and_escape_to_the_role_buttons_and_controls_that_use_them_keep_them() {
        let line = Node::new(1, Kind::TextBox);
        let lines = Node::new(2, Kind::TextBox).prop("multiline", Value::Bool(true));
        assert!(!keeps(&line, "Enter") && !keeps(&line, "Escape"), "a field of one line keeps neither");
        assert!(keeps(&lines, "Enter") && !keeps(&lines, "Escape"), "a field of several lines keeps Enter alone");
        assert!(!keeps(&Node::new(3, Kind::NumberBox), "Enter"), "a number field keeps neither");
        assert!(!keeps(&Node::new(4, Kind::ComboBox).prop("editable", Value::Bool(true)), "Escape"), "an editable drop-down keeps neither");
        assert!(keeps(&Node::new(5, Kind::ComboBox), "Escape"), "a drop-down closes its list with Escape");
        assert!(keeps(&Node::new(6, Kind::Button), "Enter") && !keeps(&Node::new(6, Kind::Button), "Escape"), "a focused button presses itself");
        assert!(keeps(&Node::new(7, Kind::ListBox), "Enter"), "a list activates its row");
        assert!(!keeps(&Node::new(8, Kind::Text), "Enter"), "text uses no key");
        assert!(!keeps(&Node::new(9, Kind::ListBox).prop("multiple", Value::Bool(true)), "Enter"), "a list of check boxes leaves Enter to the default button");
        assert!(!keeps(&Node::new(10, Kind::Table).prop("selection", Value::Str(String::from("none"))), "Enter"), "a table that selects nothing leaves Enter too");
        assert!(keeps(&Node::new(11, Kind::Table).prop("selection", Value::Str(String::from("single"))), "Enter"), "a table that selects activates its row");
    }
}
