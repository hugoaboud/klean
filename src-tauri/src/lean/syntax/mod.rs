//! Structural syntax layer: lexer, tree types and the command parser.

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::{Diagnostic, LineIndex, NodeKind, ParsedFile, Severity, Span, SyntaxNode};
pub use parser::parse;
