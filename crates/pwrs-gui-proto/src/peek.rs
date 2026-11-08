//! A password field's peek button, as Windows' PasswordBox offers it in its
//! default Peek mode: [`Peek`] says when the button is offered, and
//! [`crate::tokens::PEEK`] its size. Both renderers draw it inside the
//! field's right end and show the password only while it is held or while
//! Alt+F8 is ([`crate::Shortcut::is_peek`]); the page's script mirrors
//! [`Peek`] in `runtime/layout.js`.

/// Whether a password field offers its peek button: once the field has the
/// focus and a character is typed into it, and, once the focus leaves, not
/// again until the field is cleared and typing starts over, so a password
/// left in a field cannot be shown by whoever comes to it later. Text a
/// script sets withholds the button until the field is cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peek {
    /// Whether typing offers the button: true from the field's last empty
    /// state until the focus leaves it offered.
    armed: bool,
    /// Whether the button is offered now.
    offered: bool,
}

impl Peek {
    /// A field that opens with `text`.
    pub fn new(text: &str) -> Peek {
        Peek { armed: text.is_empty(), offered: false }
    }

    /// `text` set by a script rather than typed.
    pub fn set(&mut self, text: &str) {
        *self = Peek::new(text);
    }

    /// Takes the field's state now, whether it has the focus and its text,
    /// and returns whether the button is offered.
    pub fn update(&mut self, focused: bool, text: &str) -> bool {
        if text.is_empty() {
            self.armed = true;
            self.offered = false;
        } else if focused && self.armed {
            self.offered = true;
        }
        if !focused && self.offered {
            self.offered = false;
            self.armed = false;
        }
        self.offered
    }

    /// Whether the button is offered.
    pub fn offered(&self) -> bool {
        self.offered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_is_offered_once_typed_into_and_withheld_after_the_focus_leaves_until_cleared() {
        let mut peek = Peek::new("");
        assert!(!peek.update(true, ""), "an empty field offers nothing");
        assert!(peek.update(true, "s"), "a character typed with the focus offers it");
        assert!(peek.update(true, "secret"));
        assert!(!peek.update(false, "secret"), "the focus leaving withdraws it");
        assert!(!peek.update(true, "secret"), "nor does the focus coming back offer it again");
        assert!(!peek.update(true, "secret2"), "typing more does not either");
        assert!(!peek.update(true, ""), "clearing the field");
        assert!(peek.update(true, "n"), "and typing again offers it");
    }

    #[test]
    fn text_a_script_sets_withholds_the_button_until_the_field_is_cleared() {
        let mut peek = Peek::new("given");
        assert!(!peek.update(true, "given"), "a field that opens with text offers nothing");
        peek.set("");
        assert!(peek.update(true, "t"));
        peek.set("set");
        assert!(!peek.offered(), "a script's text withdraws it");
        assert!(!peek.update(true, "set"));
        assert!(!peek.update(true, ""));
        assert!(peek.update(true, "x"));
    }

    #[test]
    fn a_field_cleared_without_the_focus_offers_it_when_next_typed_into() {
        let mut peek = Peek::new("");
        assert!(peek.update(true, "a"));
        assert!(!peek.update(false, ""), "cleared as the focus left");
        assert!(peek.update(true, "b"));
    }
}
