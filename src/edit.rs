//! Editing operations on the tree that keep the surrounding formatting consistent.
//!
//! New pairs copy the whitespace style of their siblings (blank lines between
//! dimensions, indentation width), and removed pairs take their leading comments
//! with them, so a modified file reads as if it had been written by hand.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::cst::{Block, Body, Expr, List, ListItem, ListValue, Pair, PairRef, Value};
use crate::parser::{ParseError, is_expr_key, parse};

/// Indentation added per nesting level when a block has no children to copy it from.
pub const DEFAULT_INDENT: &str = "  ";

/// Keys whose values are conventionally quoted even when they would be valid literals.
const STRING_KEYS: &[&str] = &[
    "connection",
    "default_value",
    "description",
    "group_item_label",
    "group_label",
    "icon_url",
    "include",
    "label",
    "max_cache_age",
    "persist_for",
    "url",
    "value",
    "value_format",
    "view_label",
];

/// Wraps a pair in a shared handle.
pub fn new_ref(pair: Pair) -> PairRef {
    Arc::new(RwLock::new(pair))
}

/// Escapes text for use inside a double quoted LookML string.
///
/// Backslashes are only doubled where they would otherwise escape a quote or another
/// backslash, so patterns such as `\d` written by hand keep their original form.
pub fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' if matches!(chars.peek(), None | Some('"') | Some('\\')) => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

/// Decodes the escaped source form of a LookML string into its text.
pub fn unescape_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next @ ('"' | '\\')) = chars.peek()
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Returns true when text can be written as an unquoted literal.
pub fn is_literal(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-' | '*'))
}

/// Picks how a new string value for key is written: expr for `sql` style keys, a literal
/// for identifier-like text such as `number` or `yes`, and a quoted string otherwise.
pub fn infer_scalar(key: &str, text: &str) -> Value {
    if is_expr_key(key) {
        expr_value(text)
    } else if is_literal(text) && !STRING_KEYS.contains(&key) {
        Value::Literal(text.to_string())
    } else {
        Value::String(escape_string(text))
    }
}

/// Builds an expression value, written as `text ;;`.
pub fn expr_value(text: &str) -> Value {
    Value::Expr(Expr {
        text: text.trim().to_string(),
        before_terminator: " ".to_string(),
    })
}

/// Builds a new pair with conventional `key: value` spacing and no leading trivia.
pub fn new_pair(key: &str, value: Value) -> Pair {
    Pair {
        leading: String::new(),
        key: key.to_string(),
        before_colon: String::new(),
        after_colon: " ".to_string(),
        value,
    }
}

/// Builds an empty block value, named as in `dimension: id {}` or anonymous as in `derived_table: {}`.
pub fn block_value(name: Option<&str>) -> Value {
    Value::Block(Block {
        name: name.map(str::to_string),
        before_brace: if name.is_some() { " " } else { "" }.to_string(),
        body: Body::default(),
    })
}

/// The indentation of the line a pair starts on, taken from its leading trivia.
pub fn indent_of(pair: &Pair) -> &str {
    pair.leading.rfind('\n').map_or("", |nl| &pair.leading[nl + 1..])
}

/// The whitespace at the start of trivia, before any comment.
fn initial_whitespace(trivia: &str) -> &str {
    let end = trivia.find('#').unwrap_or(trivia.len());
    &trivia[..end]
}

/// Indentation of the last line of some whitespace.
fn last_line(whitespace: &str) -> &str {
    whitespace
        .rfind('\n')
        .map_or(whitespace, |nl| &whitespace[nl + 1..])
}

impl Body {
    /// Position of the pair behind this handle, compared by identity.
    pub fn position(&self, pair: &PairRef) -> Option<usize> {
        self.items.iter().position(|item| Arc::ptr_eq(item, pair))
    }

    /// The first pair with this key, and name when one is given.
    pub fn find(&self, key: &str, name: Option<&str>) -> Option<PairRef> {
        self.find_all(key, name).into_iter().next()
    }

    /// Every pair with this key, and name when one is given.
    pub fn find_all(&self, key: &str, name: Option<&str>) -> Vec<PairRef> {
        self.items
            .iter()
            .filter(|item| {
                let pair = item.read();
                pair.key == key && (name.is_none() || pair.name() == name)
            })
            .cloned()
            .collect()
    }

    /// The indentation children of this body use, given the indentation of the line that
    /// opens it.
    pub fn child_indent(&self, parent_indent: &str) -> String {
        self.items
            .iter()
            .map(|item| initial_whitespace(&item.read().leading).to_string())
            .find(|ws| ws.contains('\n'))
            .map(|ws| last_line(&ws).to_string())
            .unwrap_or_else(|| format!("{parent_indent}{DEFAULT_INDENT}"))
    }

    /// Inserts a pair at index, giving it leading whitespace that matches its siblings.
    ///
    /// parent_indent is the indentation of the line that opens this body; is_root marks the
    /// top level of a file, which has no closing brace to keep on its own line.
    pub fn insert(&mut self, index: usize, mut pair: Pair, parent_indent: &str, is_root: bool) -> PairRef {
        let index = index.min(self.items.len());
        pair.leading = self.leading_for(index, &pair, parent_indent, is_root);
        if index == 0 && !self.items.is_empty() {
            // The old first pair moves down a slot, so it takes the spacing of a later sibling,
            // and must not be left glued to the new pair when it opened the file.
            let next_spacing = self
                .items
                .get(1)
                .map(|p| initial_whitespace(&p.read().leading).to_string());
            let mut first = self.items[0].write();
            let spacing = initial_whitespace(&first.leading).to_string();
            let comments = first.leading[spacing.len()..].to_string();
            let new_spacing = match next_spacing {
                Some(next) if next.contains('\n') => next,
                _ if spacing.contains('\n') => spacing,
                _ if is_root => "\n".to_string(),
                _ if spacing.is_empty() => " ".to_string(),
                _ => spacing,
            };
            first.leading = new_spacing + &comments;
        }
        if !is_root && self.items.is_empty() && !self.trailing.contains('\n') {
            self.trailing = format!("\n{parent_indent}");
        }
        if is_root && self.items.is_empty() && self.trailing.is_empty() {
            self.trailing = "\n".to_string();
        }
        let pair = new_ref(pair);
        self.items.insert(index, pair.clone());
        pair
    }

    /// Inserts a pair at index keeping the leading trivia it already has.
    pub fn insert_verbatim(&mut self, index: usize, pair: Pair) -> PairRef {
        let pair = new_ref(pair);
        self.items.insert(index.min(self.items.len()), pair.clone());
        pair
    }

    /// Leading whitespace for a pair about to be inserted at index.
    fn leading_for(&self, index: usize, pair: &Pair, parent_indent: &str, is_root: bool) -> String {
        if self.items.is_empty() {
            return if is_root {
                String::new()
            } else {
                format!("\n{parent_indent}{DEFAULT_INDENT}")
            };
        }
        // At the top the new pair takes over the spacing after the opening brace. Elsewhere it
        // copies the nearest sibling with the same key, or else the same shape, so a dimension
        // gets the blank line other dimensions have even when added after a one-line pair. The
        // first pair is skipped as its spacing follows the brace rather than another sibling.
        if index == 0 {
            return initial_whitespace(&self.items[0].read().leading).to_string();
        }
        let is_block = matches!(pair.value, Value::Block(_));
        let nearest_first: Vec<usize> = (1..self.items.len())
            .flat_map(|distance| [index.checked_sub(distance), Some(index + distance - 1)])
            .flatten()
            .filter(|&i| i > 0 && i < self.items.len())
            .collect();
        let find_like = |like: &dyn Fn(&Pair) -> bool| {
            nearest_first
                .iter()
                .copied()
                .find(|&i| like(&self.items[i].read()))
        };
        let like = find_like(&|sibling| sibling.key == pair.key)
            .or_else(|| find_like(&|sibling| matches!(sibling.value, Value::Block(_)) == is_block));
        let spacing = self.spacing_near(like.unwrap_or(index - 1), parent_indent, is_root);
        // With no similar sibling to copy, a named block (a view, explore, dimension or join) is
        // set apart by a blank line, as such blocks conventionally are. Unnamed blocks such as
        // link, when and allowed_value conventionally are not.
        if like.is_none() && pair.name().is_some() && spacing.matches('\n').count() == 1 {
            return format!("\n{spacing}");
        }
        spacing
    }

    /// The spacing before the pair at reference, or before any sibling that starts on its own
    /// line when that pair does not.
    fn spacing_near(&self, reference: usize, parent_indent: &str, is_root: bool) -> String {
        let spacing = initial_whitespace(&self.items[reference].read().leading).to_string();
        if spacing.contains('\n') || (!spacing.is_empty() && !is_root) {
            return spacing;
        }
        self.items
            .iter()
            .rev()
            .map(|item| initial_whitespace(&item.read().leading).to_string())
            .find(|ws| ws.contains('\n'))
            .unwrap_or_else(|| {
                let indent = if is_root {
                    String::new()
                } else {
                    self.child_indent(parent_indent)
                };
                format!("\n{indent}")
            })
    }

    /// Removes the pair at index along with its leading comments.
    pub fn remove(&mut self, index: usize, is_root: bool) -> PairRef {
        let removed = self.items.remove(index);
        let removed_spacing = initial_whitespace(&removed.read().leading).to_string();
        if index == 0
            && let Some(next) = self.items.first()
        {
            // The new first pair takes over the spacing right after the opening brace.
            let mut next = next.write();
            let comments = next.leading[initial_whitespace(&next.leading).len()..].to_string();
            next.leading = removed_spacing + &comments;
        }
        if self.items.is_empty() && !is_root && self.trailing.trim().is_empty() {
            self.trailing.clear();
        }
        removed
    }

    /// Removes the pair behind this handle, returning whether it was found.
    pub fn remove_pair(&mut self, pair: &PairRef, is_root: bool) -> bool {
        match self.position(pair) {
            Some(index) => {
                self.remove(index, is_root);
                true
            }
            None => false,
        }
    }

    /// Parses a LookML snippet and inserts its pairs at index, re-indented to match this body.
    pub fn insert_source(
        &mut self,
        index: usize,
        source: &str,
        parent_indent: &str,
        is_root: bool,
    ) -> Result<Vec<PairRef>, ParseError> {
        let indent = if is_root {
            String::new()
        } else {
            self.child_indent(parent_indent)
        };
        let reindented = reindent(source.trim(), &indent);
        let parsed = parse(&reindented)?;
        let mut inserted = Vec::new();
        let mut at = index.min(self.items.len());
        for (i, item) in parsed.body.items.iter().enumerate() {
            let pair = item.read().clone();
            let handle = if i == 0 {
                // insert() replaces the leading trivia with spacing that matches the siblings,
                // so the snippet's comments above its first pair are added back after it.
                let comments = pair.leading.clone();
                let handle = self.insert(at, pair, parent_indent, is_root);
                handle.write().leading.push_str(&comments);
                handle
            } else {
                self.insert_verbatim(at, pair)
            };
            inserted.push(handle);
            at += 1;
        }
        Ok(inserted)
    }
}

/// Prefixes every non-blank line after the first with indent.
fn reindent(source: &str, indent: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            if !line.trim().is_empty() {
                out.push_str(indent);
            }
        }
        out.push_str(line);
    }
    out
}

impl Pair {
    /// The block name, as in `users` for `view: users { ... }`.
    pub fn name(&self) -> Option<&str> {
        match &self.value {
            Value::Block(block) => block.name.as_deref(),
            _ => None,
        }
    }

    /// The block body when this pair's value is a block.
    pub fn body(&self) -> Option<&Body> {
        match &self.value {
            Value::Block(block) => Some(&block.body),
            _ => None,
        }
    }

    pub fn body_mut(&mut self) -> Option<&mut Body> {
        match &mut self.value {
            Value::Block(block) => Some(&mut block.body),
            _ => None,
        }
    }

    /// The text of a scalar value: a literal as written, a decoded string, or an expression.
    pub fn text(&self) -> Option<String> {
        match &self.value {
            Value::Literal(text) => Some(text.clone()),
            Value::String(raw) => Some(unescape_string(raw)),
            Value::Expr(expr) => Some(expr.text.clone()),
            Value::List(_) | Value::Block(_) => None,
        }
    }

    /// Replaces a scalar value with new text, keeping its kind (literal, string or expr)
    /// and the whitespace around it. Returns false if the value is a list or block.
    pub fn set_text(&mut self, text: &str) -> bool {
        match &mut self.value {
            Value::Literal(literal) => *literal = text.to_string(),
            Value::String(raw) => *raw = escape_string(text),
            Value::Expr(expr) => expr.text = text.trim().to_string(),
            Value::List(_) | Value::Block(_) => return false,
        }
        true
    }

    /// Replaces the value, keeping the spacing after the colon and around a `;;` terminator.
    pub fn set_value(&mut self, value: Value) {
        let value = match (&self.value, value) {
            (Value::Expr(old), Value::Expr(new)) => Value::Expr(Expr {
                text: new.text,
                before_terminator: old.before_terminator.clone(),
            }),
            (Value::List(old), Value::List(new)) => Value::List(restyle_list(old, new)),
            (_, value) => value,
        };
        self.value = value;
    }
}

/// Lays out a new list in the style of the list it replaces: inline or one element per line,
/// leading or trailing commas, and with or without a trailing comma.
///
/// Elements reuse the separators of the element at the same position, and extra elements
/// copy the separators between the last two old elements. Comments in the old list are dropped
/// along with the elements they described, except those before the closing bracket.
pub fn restyle_list(old: &List, new: List) -> List {
    let Some(last) = old.items.last() else {
        return new;
    };
    let leading_at = |i: usize| initial_whitespace(&old.items[i].leading).to_string();
    let comma_at = |i: usize| {
        old.items[i]
            .comma
            .as_deref()
            .map(|c| initial_whitespace(c).to_string())
    };
    let len = old.items.len();
    // Separators for elements past the end of the old list.
    let (extra_leading, middle_comma) = if len >= 2 {
        (leading_at(len - 1), comma_at(len - 2).unwrap_or_default())
    } else if leading_at(0).contains('\n') {
        (leading_at(0), String::new())
    } else {
        (" ".to_string(), String::new())
    };
    let trailing_comma = last.comma.as_deref().map(|c| initial_whitespace(c).to_string());
    let count = new.items.len();
    let items = new
        .items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let is_last = i + 1 == count;
            let leading = if i < len {
                leading_at(i)
            } else {
                extra_leading.clone()
            };
            let comma = if is_last {
                trailing_comma.clone()
            } else {
                Some(
                    comma_at(i)
                        .filter(|_| i + 1 < len)
                        .unwrap_or_else(|| middle_comma.clone()),
                )
            };
            ListItem {
                leading,
                value: item.value,
                comma,
            }
        })
        .collect();
    List {
        items,
        trailing: old.trailing.clone(),
    }
}

/// Builds an inline list from elements, separated by `, `.
pub fn list_of(values: Vec<ListValue>) -> List {
    let count = values.len();
    List {
        items: values
            .into_iter()
            .enumerate()
            .map(|(i, value)| ListItem {
                leading: if i == 0 { String::new() } else { " ".to_string() },
                value,
                comma: (i + 1 < count).then(String::new),
            })
            .collect(),
        trailing: String::new(),
    }
}

impl List {
    /// True when any element is a string, used to keep new elements in the list's quoting style.
    pub fn uses_strings(&self) -> Option<bool> {
        self.items.iter().find_map(|item| match &item.value {
            ListValue::String(_) => Some(true),
            ListValue::Literal(_) => Some(false),
            _ => None,
        })
    }

    /// True when any pair element has a string value, as in `filters: [status: "active"]`.
    pub fn uses_string_pair_values(&self) -> Option<bool> {
        self.items.iter().find_map(|item| match &item.value {
            ListValue::Pair { value, .. } => match value.as_ref() {
                ListValue::String(_) => Some(true),
                ListValue::Literal(_) => Some(false),
                _ => None,
            },
            _ => None,
        })
    }
}
