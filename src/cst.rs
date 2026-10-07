//! Lossless concrete syntax tree for LookML.
//!
//! Every byte of the source is owned by exactly one node: whitespace and comments
//! are kept as "trivia" strings on the node that follows them, so printing the
//! tree reproduces the input exactly. Pairs are shared handles so that callers
//! (including the Python bindings) can hold a reference to a node and mutate it
//! in place while it stays attached to the document.

use std::fmt::{self, Write};
use std::sync::Arc;

use parking_lot::RwLock;

/// A shared, mutable handle to a pair in the tree.
pub type PairRef = Arc<RwLock<Pair>>;

/// A parsed LookML file.
#[derive(Debug, Default)]
pub struct Document {
    pub body: Body,
}

/// The contents of a block, or of the whole file: a sequence of pairs followed
/// by the trivia before the closing brace (or end of file).
#[derive(Debug, Default)]
pub struct Body {
    pub items: Vec<PairRef>,
    pub trailing: String,
}

/// A `key: value` entry.
#[derive(Debug, Clone)]
pub struct Pair {
    /// Whitespace and comments between the previous token and the key.
    pub leading: String,
    pub key: String,
    /// Whitespace between the key and the colon.
    pub before_colon: String,
    /// Whitespace (and, outside expressions, comments) between the colon and the value.
    pub after_colon: String,
    pub value: Value,
}

/// The value of a pair.
#[derive(Debug, Clone)]
pub enum Value {
    /// An unquoted token such as `number`, `yes`, `left_outer` or `detail*`.
    Literal(String),
    /// A double quoted string, stored in its escaped source form without the quotes.
    String(String),
    /// A raw expression terminated by `;;`, used by `sql`, `html` and similar keys.
    Expr(Expr),
    List(List),
    Block(Block),
}

/// A raw expression such as the body of `sql: ${TABLE}.id ;;`.
#[derive(Debug, Clone, Default)]
pub struct Expr {
    /// The expression text with surrounding whitespace removed.
    pub text: String,
    /// Whitespace between the expression text and the `;;` terminator.
    pub before_terminator: String,
}

/// A `{ ... }` block, optionally named as in `dimension: id { ... }`.
#[derive(Debug, Default)]
pub struct Block {
    pub name: Option<String>,
    /// Trivia between the name and the opening brace.
    pub before_brace: String,
    pub body: Body,
}

/// A `[ ... ]` list.
#[derive(Debug, Clone, Default)]
pub struct List {
    pub items: Vec<ListItem>,
    /// Trivia before the closing bracket.
    pub trailing: String,
}

/// One element of a list along with its surrounding trivia and separator.
#[derive(Debug, Clone)]
pub struct ListItem {
    /// Trivia before the element.
    pub leading: String,
    pub value: ListValue,
    /// Trivia before the following comma, when the element is followed by one.
    pub comma: Option<String>,
}

/// An element of a list.
#[derive(Debug, Clone)]
pub enum ListValue {
    Literal(String),
    /// A double quoted string in its escaped source form, without the quotes.
    String(String),
    /// A `key: value` element as in `filters: [status: "active"]`.
    Pair {
        key: String,
        before_colon: String,
        after_colon: String,
        value: Box<ListValue>,
    },
    List(List),
}

impl Clone for Block {
    /// Deep copies the block, giving the copy its own pair handles.
    fn clone(&self) -> Self {
        Block {
            name: self.name.clone(),
            before_brace: self.before_brace.clone(),
            body: self.body.deep_clone(),
        }
    }
}

impl Body {
    /// Copies the body, giving every pair in the copy its own handle.
    pub fn deep_clone(&self) -> Body {
        Body {
            items: self
                .items
                .iter()
                .map(|item| Arc::new(RwLock::new(item.read().clone())))
                .collect(),
            trailing: self.trailing.clone(),
        }
    }
}

impl Value {
    /// The name of this value's kind as exposed to callers: literal, string, expr, list or block.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Literal(_) => "literal",
            Value::String(_) => "string",
            Value::Expr(_) => "expr",
            Value::List(_) => "list",
            Value::Block(_) => "block",
        }
    }
}

/// Prints the document back out, byte for byte equal to the parsed source when unmodified.
impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.body.fmt(f)
    }
}

impl fmt::Display for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for item in &self.items {
            item.read().fmt(f)?;
        }
        f.write_str(&self.trailing)
    }
}

impl fmt::Display for Pair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}:{}",
            self.leading, self.key, self.before_colon, self.after_colon
        )?;
        self.value.fmt(f)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Literal(text) => f.write_str(text),
            Value::String(raw) => write!(f, "\"{raw}\""),
            Value::Expr(expr) => write!(f, "{}{};;", expr.text, expr.before_terminator),
            Value::List(list) => list.fmt(f),
            Value::Block(block) => {
                if let Some(name) = &block.name {
                    f.write_str(name)?;
                }
                write!(f, "{}{{{}}}", block.before_brace, block.body)
            }
        }
    }
}

impl fmt::Display for List {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('[')?;
        for item in &self.items {
            write!(f, "{}{}", item.leading, item.value)?;
            if let Some(before_comma) = &item.comma {
                write!(f, "{before_comma},")?;
            }
        }
        write!(f, "{}]", self.trailing)
    }
}

impl fmt::Display for ListValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListValue::Literal(text) => f.write_str(text),
            ListValue::String(raw) => write!(f, "\"{raw}\""),
            ListValue::Pair {
                key,
                before_colon,
                after_colon,
                value,
            } => write!(f, "{key}{before_colon}:{after_colon}{value}"),
            ListValue::List(list) => list.fmt(f),
        }
    }
}
