//! nom based parser that turns LookML source into the lossless tree in cst.

use std::fmt;
use std::sync::Arc;

use nom::branch::alt;
use nom::bytes::complete::{escaped, is_not, take_until, take_while1};
use nom::character::complete::{char, multispace1, none_of, not_line_ending, space0};
use nom::combinator::{cut, opt, recognize};
use nom::error::{ErrorKind, ParseError as NomParseError};
use nom::multi::many0_count;
use nom::sequence::preceded;
use nom::{Err, IResult, Parser};
use parking_lot::RwLock;

use crate::cst::{Block, Body, Document, Expr, List, ListItem, ListValue, Pair, Value};

/// Keys whose value is a raw expression terminated by `;;` rather than a regular value.
/// Any key starting with `sql_` or ending with `_sql` is also an expression key.
const EXPR_KEYS: &[&str] = &["sql", "html", "expression", "expression_custom_filter"];

/// Returns true when values of this key are raw `;;` terminated expressions.
pub fn is_expr_key(key: &str) -> bool {
    EXPR_KEYS.contains(&key) || key.starts_with("sql_") || key.ends_with("_sql")
}

/// A syntax error with the 1-based line and column where parsing stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {}, column {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ParseError {}

/// Parses a LookML file into a document.
pub fn parse(source: &str) -> Result<Document, ParseError> {
    match body(source) {
        Ok(("", body)) => Ok(Document { body }),
        Ok((rest, _)) => Err(error_at(source, rest, "expected a `key:` pair")),
        Err(Err::Error(e) | Err::Failure(e)) => Err(error_at(source, e.input, e.message)),
        Err(Err::Incomplete(_)) => Err(error_at(source, "", "unexpected end of input")),
    }
}

/// Parses a single value as it would appear after `key:`, such as `"text"`, `[a, b]` or `number`.
pub fn parse_value(key: &str, source: &str) -> Result<Value, ParseError> {
    let parsed = if is_expr_key(key) {
        Ok((
            "",
            Value::Expr(Expr {
                text: source.trim().to_string(),
                before_terminator: String::new(),
            }),
        ))
    } else {
        value(source)
    };
    match parsed {
        Ok(("", value)) => Ok(value),
        Ok((rest, _)) => Err(error_at(source, rest, "unexpected text after value")),
        Err(Err::Error(e) | Err::Failure(e)) => Err(error_at(source, e.input, e.message)),
        Err(Err::Incomplete(_)) => Err(error_at(source, "", "unexpected end of input")),
    }
}

fn error_at(source: &str, rest: &str, message: &str) -> ParseError {
    let offset = source.len() - rest.len();
    let consumed = &source[..offset];
    let line = consumed.matches('\n').count() + 1;
    let column = consumed.rfind('\n').map_or(offset, |nl| offset - nl - 1) + 1;
    ParseError {
        message: message.to_string(),
        line,
        column,
        offset,
    }
}

/// nom error carrying a human readable message for the innermost failing construct.
#[derive(Debug)]
struct Error<'a> {
    input: &'a str,
    message: &'static str,
}

impl<'a> NomParseError<&'a str> for Error<'a> {
    fn from_error_kind(input: &'a str, _kind: ErrorKind) -> Self {
        Error {
            input,
            message: "syntax error",
        }
    }

    fn append(_input: &'a str, _kind: ErrorKind, other: Self) -> Self {
        other
    }
}

type PResult<'a, T> = IResult<&'a str, T, Error<'a>>;

fn fail<'a, T>(input: &'a str, message: &'static str) -> PResult<'a, T> {
    Err(Err::Failure(Error { input, message }))
}

/// Characters allowed in keys and unquoted literals, e.g. `sql_on`, `view.field`, `+refined`, `detail*`.
pub fn is_token_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, ',' | '[' | ']' | '{' | '}' | '"' | ':' | '#' | ';')
}

/// Whitespace and `#` comments.
fn trivia(input: &str) -> PResult<'_, &str> {
    recognize(many0_count(alt((
        multispace1,
        recognize((char('#'), not_line_ending)),
    ))))
    .parse(input)
}

fn token(input: &str) -> PResult<'_, &str> {
    take_while1(is_token_char).parse(input)
}

/// Pairs up to the closing brace or end of input.
fn body(mut input: &str) -> PResult<'_, Body> {
    let mut items = Vec::new();
    loop {
        let (rest, leading) = trivia(input)?;
        match rest.chars().next() {
            Some(c) if is_token_char(c) => {
                let (rest, pair) = pair(rest, leading)?;
                items.push(Arc::new(RwLock::new(pair)));
                input = rest;
            }
            _ => {
                return Ok((
                    rest,
                    Body {
                        items,
                        trailing: leading.to_string(),
                    },
                ));
            }
        }
    }
}

fn pair<'a>(input: &'a str, leading: &str) -> PResult<'a, Pair> {
    let (input, key) = token(input)?;
    let (input, before_colon) = space0(input)?;
    let Ok((input, _)) = char::<_, Error>(':').parse(input) else {
        return fail(input, "expected `:` after key");
    };
    let (input, after_colon, value) = if is_expr_key(key) {
        let (input, (after_colon, expr)) = expr(input)?;
        (input, after_colon, Value::Expr(expr))
    } else {
        let (input, after_colon) = trivia(input)?;
        let (input, value) = value(input)?;
        (input, after_colon, value)
    };
    Ok((
        input,
        Pair {
            leading: leading.to_string(),
            key: key.to_string(),
            before_colon: before_colon.to_string(),
            after_colon: after_colon.to_string(),
            value,
        },
    ))
}

/// Raw text up to `;;`, split into leading whitespace, the expression and trailing whitespace.
fn expr(input: &str) -> PResult<'_, (&str, Expr)> {
    let Ok((rest, raw)) = take_until::<_, _, Error>(";;").parse(input) else {
        return fail(input, "expression is missing its `;;` terminator");
    };
    let rest = &rest[2..];
    let text = raw.trim_start();
    let after_colon = &raw[..raw.len() - text.len()];
    let trimmed = text.trim_end();
    Ok((
        rest,
        (
            after_colon,
            Expr {
                text: trimmed.to_string(),
                before_terminator: text[trimmed.len()..].to_string(),
            },
        ),
    ))
}

fn value(input: &str) -> PResult<'_, Value> {
    match input.chars().next() {
        Some('"') => quoted(input).map(|(rest, raw)| (rest, Value::String(raw.to_string()))),
        Some('[') => list(input).map(|(rest, list)| (rest, Value::List(list))),
        Some('{') => block(input, None, ""),
        Some(c) if is_token_char(c) => {
            let (rest, literal) = token(input)?;
            // A literal followed by `{` is the name of a block, as in `view: users {`.
            let (after_trivia, before_brace) = trivia(rest)?;
            if after_trivia.starts_with('{') {
                block(after_trivia, Some(literal), before_brace)
            } else {
                Ok((rest, Value::Literal(literal.to_string())))
            }
        }
        _ => fail(input, "expected a value"),
    }
}

fn block<'a>(input: &'a str, name: Option<&str>, before_brace: &str) -> PResult<'a, Value> {
    let (input, _) = char('{').parse(input)?;
    let (input, body) = body(input)?;
    let Ok((input, _)) = char::<_, Error>('}').parse(input) else {
        return fail(input, "expected `}` or a `key:` pair");
    };
    Ok((
        input,
        Value::Block(Block {
            name: name.map(str::to_string),
            before_brace: before_brace.to_string(),
            body,
        }),
    ))
}

/// A double quoted string, returning its escaped contents without the quotes.
fn quoted(input: &str) -> PResult<'_, &str> {
    let (rest, _) = char('"').parse(input)?;
    let contents = escaped(is_not("\"\\"), '\\', none_of(""));
    let Ok((rest, raw)) = cut((opt(contents), char::<_, Error>('"')))
        .map(|(raw, _)| raw.unwrap_or(""))
        .parse(rest)
    else {
        return fail(input, "unterminated string");
    };
    Ok((rest, raw))
}

fn list(input: &str) -> PResult<'_, List> {
    let (mut input, _) = char('[').parse(input)?;
    let mut items = Vec::new();
    loop {
        let (rest, leading) = trivia(input)?;
        if let Ok((rest, _)) = char::<_, Error>(']').parse(rest) {
            return Ok((
                rest,
                List {
                    items,
                    trailing: leading.to_string(),
                },
            ));
        }
        let (rest, value) = list_value(rest)?;
        let (after_trivia, before_comma) = trivia(rest)?;
        if let Ok((after_comma, _)) = char::<_, Error>(',').parse(after_trivia) {
            items.push(ListItem {
                leading: leading.to_string(),
                value,
                comma: Some(before_comma.to_string()),
            });
            input = after_comma;
        } else if after_trivia.starts_with(']') {
            items.push(ListItem {
                leading: leading.to_string(),
                value,
                comma: None,
            });
            // The trivia before `]` is picked up as the list's trailing trivia.
            input = rest;
        } else {
            return fail(after_trivia, "expected `,` or `]` in list");
        }
    }
}

fn list_value(input: &str) -> PResult<'_, ListValue> {
    match input.chars().next() {
        Some('"') => quoted(input).map(|(rest, raw)| (rest, ListValue::String(raw.to_string()))),
        Some('[') => list(input).map(|(rest, list)| (rest, ListValue::List(list))),
        Some(c) if is_token_char(c) => {
            let (rest, literal) = token(input)?;
            if let Ok((rest, (before_colon, after_colon))) =
                (space0::<_, Error>, preceded(char(':'), trivia)).parse(rest)
            {
                let (rest, value) = list_value(rest)?;
                return Ok((
                    rest,
                    ListValue::Pair {
                        key: literal.to_string(),
                        before_colon: before_colon.to_string(),
                        after_colon: after_colon.to_string(),
                        value: Box::new(value),
                    },
                ));
            }
            Ok((rest, ListValue::Literal(literal.to_string())))
        }
        _ => fail(input, "expected a list element"),
    }
}
