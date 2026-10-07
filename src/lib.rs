//! Lossless LookML parser.
//!
//! Parses LookML into a concrete syntax tree that keeps every byte of the source,
//! so a file can be read, modified and written back out with its original
//! formatting and comments intact.
//!
//! ```
//! # fn main() -> Result<(), albert_looker_cst::ParseError> {
//! let source = "view: users {\n  dimension: id {\n    type: number\n  }\n}\n";
//! let doc = albert_looker_cst::parse(source)?;
//! assert_eq!(doc.to_string(), source);
//!
//! let view = doc.body.find("view", Some("users")).unwrap();
//! let id = view.read().body().unwrap().find("dimension", Some("id")).unwrap();
//! id.write().body_mut().unwrap().find("type", None).unwrap().write().set_text("string");
//! assert!(doc.to_string().contains("type: string"));
//! # Ok(())
//! # }
//! ```

pub mod cst;
pub mod edit;
pub mod parser;
#[cfg(feature = "python")]
mod python;

pub use cst::{Block, Body, Document, Expr, List, ListItem, ListValue, Pair, PairRef, Value};
pub use parser::{ParseError, is_expr_key, is_token_char, parse, parse_value};
