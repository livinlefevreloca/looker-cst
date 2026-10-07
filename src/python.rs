//! Python bindings, exposed as the albert_looker_cst._native extension module.
//!
//! Python objects are handles onto nodes of a shared tree: a Pair returned by find()
//! stays attached to its document, so editing it edits the document.

use std::sync::Arc;

use parking_lot::RwLock;
use pyo3::create_exception;
use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use crate::cst::{Body, Document, List, ListValue, Pair, PairRef, Value};
use crate::edit::{
    block_value, escape_string, expr_value, indent_of, infer_scalar, is_literal, list_of, new_pair,
    unescape_string,
};
use crate::parser::{is_expr_key, is_token_char, parse};

create_exception!(
    albert_looker_cst,
    LookmlSyntaxError,
    PyValueError,
    "Raised when LookML source cannot be parsed. Has line, column and offset attributes."
);

type DocRef = Arc<RwLock<Document>>;

/// Where a pair handle lives, so it can remove itself and report its parent.
#[derive(Clone)]
enum Parent {
    Document(DocRef),
    /// A block pair and where that pair itself lives.
    Pair(PairRef, Option<Arc<Parent>>),
}

fn syntax_error(py: Python<'_>, error: crate::ParseError) -> PyErr {
    let err = LookmlSyntaxError::new_err(error.to_string());
    let value = err.value(py);
    let _ = value.setattr("line", error.line);
    let _ = value.setattr("column", error.column);
    let _ = value.setattr("offset", error.offset);
    err
}

/// Parses LookML source into a Document.
#[pyfunction]
fn parse_lookml(py: Python<'_>, source: &str) -> PyResult<PyDocument> {
    let document = parse(source).map_err(|e| syntax_error(py, e))?;
    Ok(PyDocument {
        inner: Arc::new(RwLock::new(document)),
    })
}

/// A parsed LookML file. str() returns the source, including any edits.
#[pyclass(name = "Document", module = "albert_looker_cst", frozen)]
struct PyDocument {
    inner: DocRef,
}

/// A `key: value` entry in a document, such as `type: number` or `dimension: id { ... }`.
#[pyclass(name = "Pair", module = "albert_looker_cst", frozen)]
struct PyPair {
    inner: PairRef,
    parent: Option<Parent>,
}

/// Operations shared by documents and block pairs, which both contain a body of pairs.
trait Container {
    fn as_parent(&self) -> Parent;
    fn with_body<R>(&self, f: impl FnOnce(&Body) -> R) -> PyResult<R>;
    /// Runs f with the body, the indentation of the line that opens it and whether it is
    /// the top level of a file.
    fn with_body_mut<R>(&self, f: impl FnOnce(&mut Body, &str, bool) -> R) -> PyResult<R>;
}

impl Container for PyDocument {
    fn as_parent(&self) -> Parent {
        Parent::Document(self.inner.clone())
    }

    fn with_body<R>(&self, f: impl FnOnce(&Body) -> R) -> PyResult<R> {
        Ok(f(&self.inner.read().body))
    }

    fn with_body_mut<R>(&self, f: impl FnOnce(&mut Body, &str, bool) -> R) -> PyResult<R> {
        Ok(f(&mut self.inner.write().body, "", true))
    }
}

fn not_a_block(pair: &Pair) -> PyErr {
    PyTypeError::new_err(format!("`{}` is a {}, not a block", pair.key, pair.value.kind()))
}

impl Container for PyPair {
    fn as_parent(&self) -> Parent {
        Parent::Pair(self.inner.clone(), self.parent.clone().map(Arc::new))
    }

    fn with_body<R>(&self, f: impl FnOnce(&Body) -> R) -> PyResult<R> {
        let pair = self.inner.read();
        pair.body().map(f).ok_or_else(|| not_a_block(&pair))
    }

    fn with_body_mut<R>(&self, f: impl FnOnce(&mut Body, &str, bool) -> R) -> PyResult<R> {
        let mut pair = self.inner.write();
        let indent = indent_of(&pair).to_string();
        match pair.body_mut() {
            Some(body) => Ok(f(body, &indent, false)),
            None => Err(not_a_block(&pair)),
        }
    }
}

fn handle(pair: PairRef, parent: &Parent) -> PyPair {
    PyPair {
        inner: pair,
        parent: Some(parent.clone()),
    }
}

fn children(container: &impl Container) -> PyResult<Vec<PyPair>> {
    let parent = container.as_parent();
    container.with_body(|body| body.items.iter().map(|p| handle(p.clone(), &parent)).collect())
}

fn find(container: &impl Container, key: &str, name: Option<&str>) -> PyResult<Option<PyPair>> {
    let parent = container.as_parent();
    container.with_body(|body| body.find(key, name).map(|p| handle(p, &parent)))
}

fn find_all(container: &impl Container, key: &str, name: Option<&str>) -> PyResult<Vec<PyPair>> {
    let parent = container.as_parent();
    container.with_body(|body| {
        body.find_all(key, name)
            .into_iter()
            .map(|p| handle(p, &parent))
            .collect()
    })
}

fn get_item(py: Python<'_>, container: &impl Container, key: &str) -> PyResult<Py<PyAny>> {
    let found = container.with_body(|body| body.find(key, None))?;
    match found {
        Some(pair) => value_to_py(py, &pair.read().value),
        None => Err(PyKeyError::new_err(key.to_string())),
    }
}

fn get(
    py: Python<'_>,
    container: &impl Container,
    key: &str,
    default: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let found = container.with_body(|body| body.find(key, None))?;
    match found {
        Some(pair) => value_to_py(py, &pair.read().value),
        None => Ok(default.unwrap_or_else(|| py.None())),
    }
}

/// Sets the value of the first pair with key, adding the pair at the end when missing.
fn set_item(container: &impl Container, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    match container.with_body(|body| body.find(key, None))? {
        Some(pair) => assign_value(&pair, value),
        None => add(container, key, Some(value), None, None, None).map(|_| ()),
    }
}

fn add(
    container: &impl Container,
    key: &str,
    value: Option<&Bound<'_, PyAny>>,
    name: Option<&str>,
    kind: Option<&str>,
    index: Option<isize>,
) -> PyResult<PyPair> {
    validate_token(key, "key")?;
    let value = match value {
        None => {
            if let Some(name) = name {
                validate_token(name, "block name")?;
            }
            match kind {
                None | Some("block") => block_value(name),
                Some(other) => return Err(PyValueError::new_err(format!("a {other} value needs a value"))),
            }
        }
        Some(value) => {
            if name.is_some() {
                return Err(PyValueError::new_err("name can only be given for a block value"));
            }
            value_from_py(key, value, kind, None)?
        }
    };
    let parent = container.as_parent();
    container.with_body_mut(|body, indent, is_root| {
        let index = resolve_index(index, body.items.len(), true)?;
        Ok(handle(
            body.insert(index, new_pair(key, value), indent, is_root),
            &parent,
        ))
    })?
}

fn add_source(
    container: &impl Container,
    py: Python<'_>,
    source: &str,
    index: Option<isize>,
) -> PyResult<Vec<PyPair>> {
    let parent = container.as_parent();
    container.with_body_mut(|body, indent, is_root| {
        let index = resolve_index(index, body.items.len(), true)?;
        body.insert_source(index, source, indent, is_root)
            .map(|pairs| pairs.into_iter().map(|p| handle(p, &parent)).collect())
            .map_err(|e| syntax_error(py, e))
    })?
}

/// Removes a pair given its handle, or the first pair with a key (and name), returning
/// whether anything was removed.
fn remove(container: &impl Container, target: &Bound<'_, PyAny>, name: Option<&str>) -> PyResult<bool> {
    if let Ok(pair) = target.extract::<PyRef<'_, PyPair>>() {
        let pair = pair.inner.clone();
        return container.with_body_mut(|body, _, is_root| body.remove_pair(&pair, is_root));
    }
    let key: String = target
        .extract()
        .map_err(|_| PyTypeError::new_err("remove() takes a Pair or a key"))?;
    container.with_body_mut(|body, _, is_root| match body.find(&key, name) {
        Some(pair) => body.remove_pair(&pair, is_root),
        None => false,
    })
}

/// Every pair under the container, depth first in source order.
fn walk(container: &impl Container) -> PyResult<Vec<PyPair>> {
    fn visit(pair: &PairRef, parent: &Parent, out: &mut Vec<PyPair>) {
        out.push(handle(pair.clone(), parent));
        let as_parent = Parent::Pair(pair.clone(), Some(Arc::new(parent.clone())));
        if let Some(body) = pair.read().body() {
            for child in &body.items {
                visit(child, &as_parent, out);
            }
        }
    }
    let parent = container.as_parent();
    let mut out = Vec::new();
    container.with_body(|body| {
        for item in &body.items {
            visit(item, &parent, &mut out);
        }
    })?;
    Ok(out)
}

fn resolve_index(index: Option<isize>, len: usize, inserting: bool) -> PyResult<usize> {
    let limit = if inserting { len } else { len.saturating_sub(1) };
    match index {
        None => Ok(len),
        Some(i) if i >= 0 => Ok((i as usize).min(limit)),
        Some(i) => {
            let from_end = i.unsigned_abs();
            if from_end > len {
                Err(PyIndexError::new_err("index out of range"))
            } else {
                Ok(len - from_end)
            }
        }
    }
}

fn validate_token(text: &str, what: &str) -> PyResult<()> {
    if text.is_empty() || !text.chars().all(is_token_char) {
        return Err(PyValueError::new_err(format!(
            "{text:?} is not a valid LookML {what}"
        )));
    }
    Ok(())
}

fn value_to_py(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    Ok(match value {
        Value::Literal(text) => PyString::new(py, text).into_any().unbind(),
        Value::String(raw) => PyString::new(py, &unescape_string(raw)).into_any().unbind(),
        Value::Expr(expr) => PyString::new(py, &expr.text).into_any().unbind(),
        Value::List(list) => list_to_py(py, list)?,
        Value::Block(_) => py.None(),
    })
}

fn list_to_py(py: Python<'_>, list: &List) -> PyResult<Py<PyAny>> {
    let items = list
        .items
        .iter()
        .map(|item| list_value_to_py(py, &item.value))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, items)?.into_any().unbind())
}

fn list_value_to_py(py: Python<'_>, value: &ListValue) -> PyResult<Py<PyAny>> {
    Ok(match value {
        ListValue::Literal(text) => PyString::new(py, text).into_any().unbind(),
        ListValue::String(raw) => PyString::new(py, &unescape_string(raw)).into_any().unbind(),
        ListValue::Pair { key, value, .. } => {
            let value = list_value_to_py(py, value)?;
            PyTuple::new(py, [PyString::new(py, key).into_any().unbind(), value])?
                .into_any()
                .unbind()
        }
        ListValue::List(list) => list_to_py(py, list)?,
    })
}

/// Text for Python scalars that LookML writes as literals: yes/no for booleans and numbers as is.
fn literal_scalar(value: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    if value.is_instance_of::<PyBool>() {
        return Ok(Some(
            if value.extract::<bool>()? { "yes" } else { "no" }.to_string(),
        ));
    }
    if value.is_instance_of::<PyInt>() || value.is_instance_of::<PyFloat>() {
        return Ok(Some(value.str()?.to_string()));
    }
    Ok(None)
}

/// Converts a Python value into a LookML value for key.
///
/// kind forces the value's form; without it strings follow infer_scalar, lists and dicts
/// become lists, and booleans and numbers become literals. old is the list being replaced,
/// whose quoting style new list elements follow.
fn value_from_py(
    key: &str,
    value: &Bound<'_, PyAny>,
    kind: Option<&str>,
    old: Option<&List>,
) -> PyResult<Value> {
    let is_list = value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>()
        || value.is_instance_of::<PyDict>();
    match kind {
        None if is_list => Ok(Value::List(list_from_py(value, old)?)),
        Some("list") => {
            if !is_list {
                return Err(PyTypeError::new_err("a list value must be a list, tuple or dict"));
            }
            Ok(Value::List(list_from_py(value, old)?))
        }
        Some("block") => Err(PyValueError::new_err(
            "block values are created with add(key, name=...)",
        )),
        _ => {
            let text = match literal_scalar(value)? {
                Some(text) => text,
                None => value
                    .extract::<String>()
                    .map_err(|_| PyTypeError::new_err("value must be a str, bool, number, list or dict"))?,
            };
            scalar(key, &text, kind, literal_scalar(value)?.is_some())
        }
    }
}

fn scalar(key: &str, text: &str, kind: Option<&str>, from_literal_type: bool) -> PyResult<Value> {
    match kind {
        None if from_literal_type => Ok(Value::Literal(text.to_string())),
        None => Ok(infer_scalar(key, text)),
        Some("literal") => {
            validate_token(text, "literal")?;
            Ok(Value::Literal(text.to_string()))
        }
        Some("string") => Ok(Value::String(escape_string(text))),
        Some("expr") => {
            if text.contains(";;") {
                return Err(PyValueError::new_err("an expression cannot contain `;;`"));
            }
            Ok(expr_value(text))
        }
        Some(other) => Err(PyValueError::new_err(format!(
            "unknown kind {other:?}; expected literal, string, expr, list or block"
        ))),
    }
}

fn list_from_py(value: &Bound<'_, PyAny>, old: Option<&List>) -> PyResult<List> {
    let strings = old.and_then(List::uses_strings);
    let pair_strings = old.and_then(List::uses_string_pair_values).unwrap_or(true);
    let elements: Vec<ListValue> = if let Ok(dict) = value.cast::<PyDict>() {
        dict.iter()
            .map(|(k, v)| list_pair(&k, &v, pair_strings))
            .collect::<PyResult<_>>()?
    } else {
        value
            .try_iter()?
            .map(|item| list_element(&item?, strings, pair_strings))
            .collect::<PyResult<_>>()?
    };
    Ok(list_of(elements))
}

fn list_element(item: &Bound<'_, PyAny>, strings: Option<bool>, pair_strings: bool) -> PyResult<ListValue> {
    if let Ok(tuple) = item.cast::<PyTuple>() {
        if tuple.len() != 2 {
            return Err(PyValueError::new_err("list pairs must be (key, value) tuples"));
        }
        return list_pair(&tuple.get_item(0)?, &tuple.get_item(1)?, pair_strings);
    }
    if item.is_instance_of::<PyList>() {
        return Ok(ListValue::List(list_from_py(item, None)?));
    }
    if let Some(text) = literal_scalar(item)? {
        return Ok(ListValue::Literal(text));
    }
    let text: String = item
        .extract()
        .map_err(|_| PyTypeError::new_err("list elements must be str, bool, number, (key, value) or list"))?;
    Ok(match strings {
        Some(false) | None if is_literal(&text) => ListValue::Literal(text),
        _ => ListValue::String(escape_string(&text)),
    })
}

fn list_pair(key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>, strings: bool) -> PyResult<ListValue> {
    let key: String = key.extract()?;
    validate_token(&key, "key")?;
    let value = if value.is_instance_of::<PyList>() {
        ListValue::List(list_from_py(value, None)?)
    } else if let Some(text) = literal_scalar(value)? {
        ListValue::Literal(text)
    } else {
        let text: String = value.extract()?;
        if strings || !is_literal(&text) {
            ListValue::String(escape_string(&text))
        } else {
            ListValue::Literal(text)
        }
    };
    Ok(ListValue::Pair {
        key,
        before_colon: String::new(),
        after_colon: " ".to_string(),
        value: Box::new(value),
    })
}

/// Assigns a Python value to a pair, keeping the kind of its current value.
fn assign_value(pair: &PairRef, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let mut guard = pair.write();
    let key = guard.key.clone();
    let new_value = match &guard.value {
        Value::Block(_) => {
            return Err(PyTypeError::new_err(format!(
                "`{key}` is a block; edit its children or use set_value()"
            )));
        }
        Value::List(old) => {
            let old = old.clone();
            value_from_py(&key, value, Some("list"), Some(&old))?
        }
        current => {
            let kind = current.kind();
            if value.is_instance_of::<PyList>() || value.is_instance_of::<PyDict>() {
                return Err(PyTypeError::new_err(format!(
                    "`{key}` holds a {kind}; use set_value(value, kind=\"list\") to change it"
                )));
            }
            // A literal stays a literal unless the new text needs quotes.
            let kind = match (kind, literal_scalar(value)?, value.extract::<String>()) {
                ("literal", None, Ok(text)) if !text.chars().all(is_token_char) || text.is_empty() => {
                    "string"
                }
                _ => kind,
            };
            value_from_py(&key, value, Some(kind), None)?
        }
    };
    guard.set_value(new_value);
    Ok(())
}

/// The trivia lines that are comments, without surrounding whitespace.
fn comment_lines(leading: &str) -> Vec<String> {
    leading
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

macro_rules! container_methods {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// The pairs directly inside this container.
            #[getter]
            fn children(&self) -> PyResult<Vec<PyPair>> {
                children(self)
            }

            /// The first child with key, and block name when given, or None.
            #[pyo3(signature = (key, name=None))]
            fn find(&self, key: &str, name: Option<&str>) -> PyResult<Option<PyPair>> {
                find(self, key, name)
            }

            /// Every child with key, and block name when given.
            #[pyo3(signature = (key, name=None))]
            fn find_all(&self, key: &str, name: Option<&str>) -> PyResult<Vec<PyPair>> {
                find_all(self, key, name)
            }

            /// The value of the first child with key, or default.
            #[pyo3(signature = (key, default=None))]
            fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
                get(py, self, key, default)
            }

            /// Adds a child pair and returns it. Without a value the child is a block, named
            /// when name is given. kind forces the value's form: literal, string, expr, list
            /// or block. index defaults to the end.
            #[pyo3(signature = (key, value=None, *, name=None, kind=None, index=None))]
            fn add(
                &self,
                key: &str,
                value: Option<&Bound<'_, PyAny>>,
                name: Option<&str>,
                kind: Option<&str>,
                index: Option<isize>,
            ) -> PyResult<PyPair> {
                add(self, key, value, name, kind, index)
            }

            /// Parses a LookML snippet and inserts its pairs, re-indented to fit, returning them.
            #[pyo3(signature = (source, index=None))]
            fn add_source(
                &self,
                py: Python<'_>,
                source: &str,
                index: Option<isize>,
            ) -> PyResult<Vec<PyPair>> {
                add_source(self, py, source, index)
            }

            /// Removes a child given its Pair, or the first child with a key (and name).
            /// Returns whether a child was removed.
            #[pyo3(signature = (target, name=None))]
            fn remove(&self, target: &Bound<'_, PyAny>, name: Option<&str>) -> PyResult<bool> {
                remove(self, target, name)
            }

            /// Every pair under this container, depth first in source order.
            fn walk(&self) -> PyResult<Vec<PyPair>> {
                walk(self)
            }

            fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
                get_item(py, self, key)
            }

            fn __setitem__(&self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
                set_item(self, key, value)
            }

            fn __delitem__(&self, key: &str) -> PyResult<()> {
                let removed = self.with_body_mut(|body, _, is_root| match body.find(key, None) {
                    Some(pair) => body.remove_pair(&pair, is_root),
                    None => false,
                })?;
                if removed {
                    Ok(())
                } else {
                    Err(PyKeyError::new_err(key.to_string()))
                }
            }

            fn __contains__(&self, key: &str) -> PyResult<bool> {
                self.with_body(|body| body.find(key, None).is_some())
            }

            fn __len__(&self) -> PyResult<usize> {
                self.with_body(|body| body.items.len())
            }

            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                let items = PyList::new(py, children(self)?)?;
                Ok(items.try_iter()?.into_any().unbind())
            }
        }
    };
}

container_methods!(PyDocument);
container_methods!(PyPair);

#[pymethods]
impl PyDocument {
    /// Parses LookML source into a Document.
    #[staticmethod]
    fn parse(py: Python<'_>, source: &str) -> PyResult<PyDocument> {
        parse_lookml(py, source)
    }

    /// The document as LookML source.
    #[allow(clippy::inherent_to_string)]
    fn to_string(&self) -> String {
        self.inner.read().to_string()
    }

    fn __str__(&self) -> String {
        self.to_string()
    }

    fn __repr__(&self) -> String {
        format!("<Document with {} pairs>", self.inner.read().body.items.len())
    }
}

#[pymethods]
impl PyPair {
    #[getter]
    fn key(&self) -> String {
        self.inner.read().key.clone()
    }

    #[setter]
    fn set_key(&self, key: &str) -> PyResult<()> {
        validate_token(key, "key")?;
        let mut pair = self.inner.write();
        if is_expr_key(key) != matches!(pair.value, Value::Expr(_)) {
            return Err(PyValueError::new_err(format!(
                "cannot rename `{}` to `{key}`: expression keys and other keys hold different kinds of value",
                pair.key
            )));
        }
        pair.key = key.to_string();
        Ok(())
    }

    /// The block name, as in users for `view: users { ... }`, or None.
    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.read().name().map(str::to_string)
    }

    #[setter]
    fn set_name(&self, name: Option<&str>) -> PyResult<()> {
        if let Some(name) = name {
            validate_token(name, "block name")?;
        }
        let mut pair = self.inner.write();
        match &mut pair.value {
            Value::Block(block) => {
                if block.name.is_none() && name.is_some() && block.before_brace.is_empty() {
                    block.before_brace = " ".to_string();
                }
                block.name = name.map(str::to_string);
                Ok(())
            }
            _ => Err(not_a_block(&pair)),
        }
    }

    /// One of literal, string, expr, list or block.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.read().value.kind()
    }

    /// The value: str for literals, strings and expressions, a list for lists (with
    /// (key, value) tuples for pairs such as filters) and None for blocks. Assigning keeps
    /// the current kind.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        value_to_py(py, &self.inner.read().value)
    }

    #[setter(value)]
    fn set_value_attr(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        assign_value(&self.inner, value)
    }

    /// Replaces the value, optionally changing its kind: literal, string, expr or list.
    #[pyo3(signature = (value, kind=None))]
    fn set_value(&self, value: &Bound<'_, PyAny>, kind: Option<&str>) -> PyResult<()> {
        let key = self.inner.read().key.clone();
        let old = match &self.inner.read().value {
            Value::List(list) => Some(list.clone()),
            _ => None,
        };
        let new_value = value_from_py(&key, value, kind, old.as_ref())?;
        self.inner.write().set_value(new_value);
        Ok(())
    }

    /// The comment lines directly above this pair, including their `#`.
    #[getter]
    fn comments(&self) -> Vec<String> {
        comment_lines(&self.inner.read().leading)
    }

    #[setter]
    fn set_comments(&self, comments: Vec<String>) {
        let mut pair = self.inner.write();
        let indent = indent_of(&pair).to_string();
        let first_comment = pair.leading.find('#').unwrap_or(pair.leading.len());
        let before = &pair.leading[..first_comment];
        let prefix = before.rfind('\n').map_or("", |nl| &before[..=nl]).to_string();
        let mut leading = prefix + &indent;
        for comment in comments {
            let comment = comment.trim();
            if comment.starts_with('#') {
                leading.push_str(comment);
            } else {
                leading.push_str("# ");
                leading.push_str(comment);
            }
            leading.push('\n');
            leading.push_str(&indent);
        }
        pair.leading = leading;
    }

    /// The block or document containing this pair, or None for a pair that was never attached.
    #[getter]
    fn parent(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match &self.parent {
            None => py.None(),
            Some(Parent::Document(doc)) => Py::new(py, PyDocument { inner: doc.clone() })?.into_any(),
            Some(Parent::Pair(pair, parent)) => Py::new(
                py,
                PyPair {
                    inner: pair.clone(),
                    parent: parent.as_deref().cloned(),
                },
            )?
            .into_any(),
        })
    }

    /// Removes this pair from its parent, returning whether it was still attached.
    fn detach(&self) -> PyResult<bool> {
        let removed = match &self.parent {
            None => false,
            Some(Parent::Document(doc)) => doc.write().body.remove_pair(&self.inner, true),
            Some(Parent::Pair(parent, _)) => parent
                .write()
                .body_mut()
                .is_some_and(|body| body.remove_pair(&self.inner, false)),
        };
        Ok(removed)
    }

    /// The pair as LookML source, without the whitespace and comments before it unless
    /// include_leading is set.
    #[pyo3(signature = (include_leading=false))]
    fn to_string(&self, include_leading: bool) -> String {
        let pair = self.inner.read();
        let text = pair.to_string();
        if include_leading {
            text
        } else {
            text[pair.leading.len()..].to_string()
        }
    }

    fn __str__(&self) -> String {
        self.to_string(false)
    }

    fn __repr__(&self) -> String {
        let pair = self.inner.read();
        match &pair.value {
            Value::Block(block) => match &block.name {
                Some(name) => format!("<Pair {}: {name} {{...}}>", pair.key),
                None => format!("<Pair {}: {{...}}>", pair.key),
            },
            value => {
                let text = value.to_string();
                let short: String = text.chars().take(60).collect();
                let ellipsis = if short.len() < text.len() { "..." } else { "" };
                format!("<Pair {}: {short}{ellipsis}>", pair.key)
            }
        }
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<PyRef<'_, PyPair>>()
            .is_ok_and(|other| Arc::ptr_eq(&self.inner, &other.inner))
    }

    fn __hash__(&self) -> isize {
        Arc::as_ptr(&self.inner) as isize
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDocument>()?;
    m.add_class::<PyPair>()?;
    m.add_function(wrap_pyfunction!(parse_lookml, m)?)?;
    m.add("LookmlSyntaxError", m.py().get_type::<LookmlSyntaxError>())?;
    Ok(())
}
