//! The keys a ListBox, Table and TreeView use themselves, which both
//! renderers follow, as [`crate::Shortcut::widget_keeps`] lists them. With
//! the widget focused, the arrows, Home, End and the page keys move the row
//! the keyboard is on and select it as a click there would. Space selects
//! it, Ctrl+A selects every row shown, and Enter activates it when it is
//! selected. In a table, Ctrl+C copies the rows selected ([`copied`]). Rows
//! are counted from 0 in the order the widget shows them: a table's after
//! its sort and filter, a tree's open items.

use std::collections::BTreeSet;

/// How a widget's rows are selected, which sets what its keys select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// No row can be: the keys move the row the keyboard is on alone, and
    /// Enter activates nothing.
    None,
    /// One row at a time.
    Single,
    /// Any number of rows, Shift taking a range and Ctrl moving without
    /// selecting.
    Multiple,
    /// Any number of rows, each with a check box of its own, as a ListBox
    /// with -Multiple shows them: the keys move without checking, Space
    /// checks or clears the row, and Enter activates nothing.
    Checks,
}

/// A key a list, table or tree uses itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    Enter,
    /// Ctrl+A.
    All,
}

impl RowKey {
    /// The key a press of `key`, named as a shortcut names it, stands for,
    /// with Ctrl, Shift and Alt pressed as `ctrl`, `shift` and `alt` say; none
    /// for a press the widgets leave to the window, as
    /// [`crate::Shortcut::widget_keeps`] does: any with Alt, Space and Enter
    /// with Ctrl, and Ctrl+A with Shift.
    pub fn named(key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<RowKey> {
        if alt {
            return None;
        }
        Some(match key {
            "ArrowUp" => RowKey::Up,
            "ArrowDown" => RowKey::Down,
            "Home" => RowKey::Home,
            "End" => RowKey::End,
            "PageUp" => RowKey::PageUp,
            "PageDown" => RowKey::PageDown,
            "Space" if !ctrl => RowKey::Space,
            "Enter" if !ctrl => RowKey::Enter,
            "A" if ctrl && !shift => RowKey::All,
            _ => return None,
        })
    }
}

/// What a key does to a widget's selection, in rows as shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSelect {
    /// Nothing.
    Same,
    /// The row alone.
    Only(usize),
    /// The rows from the first to the second, both included, in either
    /// order.
    Range(usize, usize),
    /// The row added, or taken away when it was selected.
    Toggle(usize),
    /// Every row shown.
    All,
}

impl RowSelect {
    /// Applies the selection to `selected`, whose rows are named by the
    /// keys `shown` gives the rows as shown, and returns whether it changed.
    /// A row past `shown`'s end selects nothing.
    pub fn apply<K: Ord + Copy>(self, selected: &mut BTreeSet<K>, shown: &[K]) -> bool {
        let before = selected.clone();
        match self {
            RowSelect::Same => {}
            RowSelect::Only(at) => *selected = shown.get(at).copied().into_iter().collect(),
            RowSelect::Range(from, to) => {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                *selected = shown.iter().skip(lo).take(hi - lo + 1).copied().collect();
            }
            RowSelect::Toggle(at) => {
                if let Some(key) = shown.get(at)
                    && !selected.remove(key)
                {
                    selected.insert(*key);
                }
            }
            RowSelect::All => *selected = shown.iter().copied().collect(),
        }
        *selected != before
    }
}

/// What a key does: the row the keyboard is on after it, the row a Shift
/// range runs from, the selection it makes, and whether it activates the
/// row the keyboard is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowMove {
    pub current: usize,
    pub anchor: Option<usize>,
    pub select: RowSelect,
    pub activate: bool,
}

/// Where a list, table or tree stands as a key comes: how many rows it
/// shows, how they are selected, the row its keyboard is on, none before
/// the first key or click, whether that row is selected, the row its Shift
/// range runs from, and how many rows it shows at once ([`page_rows`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowState {
    pub count: usize,
    pub selection: Selection,
    pub current: Option<usize>,
    pub current_selected: bool,
    pub anchor: Option<usize>,
    pub page: usize,
}

/// What `key` does, with Shift and Ctrl pressed as `shift` and `ctrl` say, in
/// a widget standing as `state` says; `None` when the key does nothing. A
/// move with no row before goes to the first row, and Enter on a row that
/// is not selected does nothing, as Enter on a list with nothing selected
/// does in a Windows dialog.
pub fn row_key(key: RowKey, shift: bool, ctrl: bool, state: RowState) -> Option<RowMove> {
    let RowState { count, selection, current, current_selected, anchor, page } = state;
    let last = count.checked_sub(1)?;
    let current = current.map(|c| c.min(last));
    let step = page.max(1);
    let to = match key {
        RowKey::Up => current.map_or(0, |c| c.saturating_sub(1)),
        RowKey::Down => current.map_or(0, |c| (c + 1).min(last)),
        RowKey::Home => 0,
        RowKey::End => last,
        RowKey::PageUp => current.map_or(0, |c| c.saturating_sub(step)),
        RowKey::PageDown => current.map_or(0, |c| c.saturating_add(step).min(last)),
        RowKey::Space => {
            let at = current?;
            let (anchor, select) = match selection {
                Selection::None => return None,
                Selection::Single => (Some(at), RowSelect::Only(at)),
                Selection::Multiple if shift => {
                    let from = anchor.unwrap_or(at);
                    (Some(from), RowSelect::Range(from, at))
                }
                Selection::Multiple | Selection::Checks => (Some(at), RowSelect::Toggle(at)),
            };
            return Some(RowMove { current: at, anchor, select, activate: false });
        }
        RowKey::Enter => {
            let at = current?;
            let activates = current_selected && matches!(selection, Selection::Single | Selection::Multiple);
            return activates.then_some(RowMove { current: at, anchor, select: RowSelect::Same, activate: true });
        }
        RowKey::All => {
            let all = matches!(selection, Selection::Multiple | Selection::Checks);
            return all.then_some(RowMove { current: current.unwrap_or(0), anchor, select: RowSelect::All, activate: false });
        }
    };
    let (anchor, select) = match selection {
        Selection::None | Selection::Checks => (anchor, RowSelect::Same),
        Selection::Multiple if ctrl => (anchor, RowSelect::Same),
        Selection::Multiple if shift => {
            let from = anchor.or(current).unwrap_or(to);
            (Some(from), RowSelect::Range(from, to))
        }
        Selection::Single | Selection::Multiple => (Some(to), RowSelect::Only(to)),
    };
    Some(RowMove { current: to, anchor, select, activate: false })
}

/// How many rows a page key moves by in a widget showing `shown` points of
/// rows `row` points apart: the rows that fit whole, less one so the row
/// left stays in sight, and one at least.
pub fn page_rows(shown: f32, row: f32) -> usize {
    if row.is_nan() || row <= 0.0 || !shown.is_finite() {
        return 1;
    }
    ((shown / row).floor() as usize).saturating_sub(1).max(1)
}

/// The text Ctrl+C copies from a table's rows, as Out-GridView copies its
/// own. `rows` are the rows selected, in the order the table shows them,
/// each its cells' text in the columns' order. Every cell is followed by a
/// tab, the last too, and every row by `newline`, the system's line ending,
/// the last too. No row copies nothing.
pub fn copied<R: AsRef<[S]>, S: AsRef<str>>(rows: &[R], newline: &str) -> String {
    let mut text = String::new();
    for row in rows {
        for cell in row.as_ref() {
            text.push_str(cell.as_ref());
            text.push('\t');
        }
        text.push_str(newline);
    }
    text
}

/// The system's line ending, which [`copied`] ends each row with: CR LF on
/// Windows, LF elsewhere, as .NET's `Environment.NewLine` is.
pub const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_copy_as_out_grid_view_copies_them() {
        let rows = [vec!["web01", "10"], vec!["web 02", ""]];
        assert_eq!(copied(&rows, "\r\n"), "web01\t10\t\r\nweb 02\t\t\r\n");
        assert_eq!(copied(&rows[..1], "\n"), "web01\t10\t\n");
        assert_eq!(copied::<Vec<&str>, &str>(&[], "\n"), "", "no row copies nothing");
    }

    fn keyed(key: RowKey, shift: bool, ctrl: bool, current: Option<usize>, anchor: Option<usize>, selection: Selection) -> Option<RowMove> {
        row_key(key, shift, ctrl, RowState { count: 10, selection, current, current_selected: true, anchor, page: 4 })
    }

    #[test]
    fn the_moves_go_a_row_a_page_or_to_an_end_and_select_as_a_click() {
        let only = |at: usize| Some(RowMove { current: at, anchor: Some(at), select: RowSelect::Only(at), activate: false });
        assert_eq!(keyed(RowKey::Down, false, false, Some(3), None, Selection::Single), only(4));
        assert_eq!(keyed(RowKey::Up, false, false, Some(3), None, Selection::Single), only(2));
        assert_eq!(keyed(RowKey::Down, false, false, Some(9), None, Selection::Single), only(9), "the last row stays the last");
        assert_eq!(keyed(RowKey::Up, false, false, Some(0), None, Selection::Single), only(0));
        assert_eq!(keyed(RowKey::Down, false, false, None, None, Selection::Single), only(0), "the first key goes to the first row");
        assert_eq!(keyed(RowKey::Up, false, false, None, None, Selection::Single), only(0));
        assert_eq!(keyed(RowKey::Home, false, false, Some(5), None, Selection::Single), only(0));
        assert_eq!(keyed(RowKey::End, false, false, Some(5), None, Selection::Single), only(9));
        assert_eq!(keyed(RowKey::PageDown, false, false, Some(3), None, Selection::Single), only(7));
        assert_eq!(keyed(RowKey::PageDown, false, false, Some(8), None, Selection::Single), only(9));
        assert_eq!(keyed(RowKey::PageUp, false, false, Some(6), None, Selection::Single), only(2));
        assert_eq!(keyed(RowKey::PageUp, false, false, Some(2), None, Selection::Single), only(0));
        assert_eq!(keyed(RowKey::Down, false, false, Some(40), None, Selection::Single), only(9), "a row past the end, as after rows went, counts as the last");
        let empty = RowState { count: 0, selection: Selection::Single, current: None, current_selected: false, anchor: None, page: 4 };
        assert_eq!(row_key(RowKey::Down, false, false, empty), None, "a widget with no rows does nothing");
    }

    #[test]
    fn shift_takes_a_range_and_ctrl_moves_without_selecting_where_several_rows_can_be() {
        let moved = keyed(RowKey::Down, true, false, Some(3), Some(1), Selection::Multiple);
        assert_eq!(moved, Some(RowMove { current: 4, anchor: Some(1), select: RowSelect::Range(1, 4), activate: false }));
        let first = keyed(RowKey::Down, true, false, Some(3), None, Selection::Multiple);
        assert_eq!(first, Some(RowMove { current: 4, anchor: Some(3), select: RowSelect::Range(3, 4), activate: false }), "the row the keyboard was on starts a range with no anchor");
        let ctrl = keyed(RowKey::End, false, true, Some(3), Some(1), Selection::Multiple);
        assert_eq!(ctrl, Some(RowMove { current: 9, anchor: Some(1), select: RowSelect::Same, activate: false }));
        assert_eq!(keyed(RowKey::Down, true, false, Some(3), Some(1), Selection::Single).map(|m| m.select), Some(RowSelect::Only(4)), "one row at a time takes no range");
        assert_eq!(keyed(RowKey::Down, false, false, Some(3), None, Selection::None).map(|m| (m.current, m.select)), Some((4, RowSelect::Same)), "a table that selects nothing moves alone");
        assert_eq!(keyed(RowKey::Down, false, false, Some(3), None, Selection::Checks).map(|m| (m.current, m.select)), Some((4, RowSelect::Same)), "check boxes move without checking");
    }

    #[test]
    fn space_selects_enter_activates_and_ctrl_a_selects_all() {
        assert_eq!(keyed(RowKey::Space, false, false, Some(2), None, Selection::Single).map(|m| m.select), Some(RowSelect::Only(2)));
        assert_eq!(keyed(RowKey::Space, false, false, Some(2), None, Selection::Multiple).map(|m| m.select), Some(RowSelect::Toggle(2)));
        assert_eq!(keyed(RowKey::Space, false, false, Some(2), None, Selection::Checks).map(|m| m.select), Some(RowSelect::Toggle(2)));
        assert_eq!(keyed(RowKey::Space, true, false, Some(5), Some(2), Selection::Multiple).map(|m| m.select), Some(RowSelect::Range(2, 5)));
        assert_eq!(keyed(RowKey::Space, false, false, None, None, Selection::Single), None, "Space with no row does nothing");
        assert_eq!(keyed(RowKey::Space, false, false, Some(2), None, Selection::None), None);
        let enter = keyed(RowKey::Enter, false, false, Some(2), Some(1), Selection::Single);
        assert_eq!(enter, Some(RowMove { current: 2, anchor: Some(1), select: RowSelect::Same, activate: true }));
        assert_eq!(keyed(RowKey::Enter, false, false, Some(2), None, Selection::None), None, "a table that selects nothing activates nothing");
        assert_eq!(keyed(RowKey::Enter, false, false, Some(2), None, Selection::Checks), None, "check boxes activate nothing");
        for selection in [Selection::Single, Selection::Multiple] {
            let unselected = RowState { count: 10, selection, current: Some(2), current_selected: false, anchor: None, page: 4 };
            assert_eq!(row_key(RowKey::Enter, false, false, unselected), None, "Enter on a row not selected activates nothing");
        }
        assert_eq!(keyed(RowKey::All, false, true, Some(2), None, Selection::Multiple).map(|m| m.select), Some(RowSelect::All));
        assert_eq!(keyed(RowKey::All, false, true, Some(2), None, Selection::Single), None);
    }

    #[test]
    fn a_press_is_a_row_key_only_where_the_widgets_keep_it() {
        assert_eq!(RowKey::named("ArrowDown", false, true, false), Some(RowKey::Down));
        assert_eq!(RowKey::named("PageUp", true, false, false), Some(RowKey::PageUp));
        assert_eq!(RowKey::named("A", true, false, false), Some(RowKey::All));
        assert_eq!(RowKey::named("A", true, true, false), None, "Ctrl+Shift+A goes to the window");
        assert_eq!(RowKey::named("A", false, false, false), None);
        assert_eq!(RowKey::named("Space", true, false, false), None, "Ctrl+Space goes to the window");
        assert_eq!(RowKey::named("Enter", false, true, false), Some(RowKey::Enter));
        assert_eq!(RowKey::named("ArrowDown", false, false, true), None, "Alt goes to the window");
        assert_eq!(RowKey::named("ArrowLeft", false, false, false), None, "a tree opens and closes with Left and Right itself");
        for written in ["ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown", "Space", "Enter", "Ctrl+A", "Shift+ArrowUp", "Ctrl+End", "Shift+Space"] {
            let s = crate::Shortcut::parse(written).expect("a shortcut");
            assert!(s.widget_keeps(crate::Kind::Table, false), "a table keeps {written}, which moves or selects");
            assert!(RowKey::named(&s.key, s.ctrl, s.shift, s.alt).is_some(), "{written} is a row key");
        }
    }

    #[test]
    fn a_selection_is_applied_in_the_keys_the_rows_are_known_by() {
        let shown = [10, 20, 30, 40];
        let mut selected = BTreeSet::from([20]);
        assert!(RowSelect::Only(2).apply(&mut selected, &shown));
        assert_eq!(selected, BTreeSet::from([30]));
        assert!(!RowSelect::Only(2).apply(&mut selected, &shown), "the same row again changes nothing");
        assert!(RowSelect::Range(3, 1).apply(&mut selected, &shown));
        assert_eq!(selected, BTreeSet::from([20, 30, 40]));
        assert!(RowSelect::Toggle(2).apply(&mut selected, &shown));
        assert_eq!(selected, BTreeSet::from([20, 40]));
        assert!(RowSelect::Toggle(0).apply(&mut selected, &shown));
        assert_eq!(selected, BTreeSet::from([10, 20, 40]));
        assert!(RowSelect::All.apply(&mut selected, &shown));
        assert_eq!(selected, BTreeSet::from([10, 20, 30, 40]));
        assert!(!RowSelect::Same.apply(&mut selected, &shown));
        let mut hidden = BTreeSet::from([99]);
        assert!(RowSelect::All.apply(&mut hidden, &shown), "every row shown replaces one not shown");
        assert!(!RowSelect::Toggle(9).apply(&mut selected, &shown), "a row past the end selects nothing");
    }

    #[test]
    fn a_page_is_the_rows_that_fit_less_one() {
        assert_eq!(page_rows(200.0, 20.0), 9);
        assert_eq!(page_rows(209.0, 20.0), 9, "a row cut short does not count");
        assert_eq!(page_rows(30.0, 20.0), 1, "one row at least");
        assert_eq!(page_rows(f32::INFINITY, 20.0), 1, "a widget measured with no end moves a row");
        assert_eq!(page_rows(200.0, 0.0), 1);
    }
}
