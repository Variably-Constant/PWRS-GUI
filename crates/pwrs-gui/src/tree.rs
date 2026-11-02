//! Builds a window's tree: nodes numbered from 1 in the order made.

use pwrs_gui_client::proto::{Kind, Node, NodeId, Value};

pub struct Builder {
    next: NodeId,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { next: 1 }
    }

    /// A node with the next id.
    pub fn node(&mut self, kind: Kind) -> Node {
        let id = self.next;
        self.next += 1;
        Node::new(id, kind)
    }

    /// A node with the next id and the name test commands and snapshots
    /// address it by.
    pub fn named(&mut self, kind: Kind, name: &str) -> Node {
        self.node(kind).named(name)
    }

    /// A text node.
    pub fn text(&mut self, text: &str) -> Node {
        self.node(Kind::Text).prop("text", Value::Str(text.to_string()))
    }

    /// A named button.
    pub fn button(&mut self, name: &str, text: &str) -> Node {
        self.named(Kind::Button, name).prop("text", Value::Str(text.to_string()))
    }
}
