//! Tokenizer for the Lean 4 surface syntax.
//!
//! The lexer is deliberately tolerant: it never fails. Any byte sequence it
//! does not understand becomes a [`TokenKind::Symbol`] run, so a half-typed
//! declaration still yields a usable token stream for the structural parser.

use serde::Serialize;

/// Lexical class of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Ident,
    Number,
    Str,
    Char,
    Symbol,
    /// Bracket opener: `(`, `[`, `{`, `⟨`, `⦃`, `⟦`, `‹`.
    Open,
    /// Bracket closer: `)`, `]`, `}`, `⟩`, `⦄`, `⟧`, `›`.
    Close,
    LineComment,
    BlockComment,
    DocComment,
    ModuleDoc,
}

impl TokenKind {
    pub fn is_comment(self) -> bool {
        matches!(
            self,
            Self::LineComment | Self::BlockComment | Self::DocComment | Self::ModuleDoc
        )
    }

    pub fn is_bracket(self) -> bool {
        matches!(self, Self::Open | Self::Close)
    }
}

/// A lexed token. Offsets are byte offsets into the source.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u32,
    pub end: u32,
    /// True when a line break separates this token from the previous one.
    pub newline_before: bool,
    /// Byte column of the token start within its line.
    pub col: u32,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start as usize..self.end as usize]
    }
}

/// Maps a bracket character to its counterpart, if it is a bracket.
pub fn bracket_pair(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '⟨' => '⟩',
        '⟩' => '⟨',
        '⦃' => '⦄',
        '⦄' => '⦃',
        '⟦' => '⟧',
        '⟧' => '⟦',
        '‹' => '›',
        '›' => '‹',
        _ => return None,
    })
}

pub fn is_open_bracket(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '⟨' | '⦃' | '⟦' | '‹')
}

pub fn is_close_bracket(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '⟩' | '⦄' | '⟧' | '›')
}

fn is_subscript(c: char) -> bool {
    matches!(c, '\u{2070}'..='\u{209F}' | '\u{1D62}'..='\u{1D6A}' | '\u{2C7C}' | '\u{00B5}')
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || is_subscript(c)
}

fn is_symbol_char(c: char) -> bool {
    if c.is_whitespace() || c.is_alphanumeric() || is_subscript(c) {
        return false;
    }
    if matches!(c, '_' | '\'' | '"' | '«' | '»' | '`') {
        return false;
    }
    if bracket_pair(c).is_some() {
        return false;
    }
    true
}

fn byte_end(chars: &[(usize, char)], idx: usize, len: u32) -> u32 {
    match chars.get(idx) {
        Some((offset, _)) => *offset as u32,
        None => len,
    }
}

fn peek(chars: &[(usize, char)], idx: usize) -> Option<char> {
    chars.get(idx).map(|(_, c)| *c)
}

/// Detects a character literal starting at `chars[i] == '\''`.
/// Returns the index just past the closing quote.
fn scan_char_literal(chars: &[(usize, char)], i: usize) -> Option<usize> {
    let total = chars.len();
    match peek(chars, i + 1) {
        Some('\\') => {
            let mut j = i + 2;
            while j < total && j < i + 12 {
                if chars[j].1 == '\'' {
                    return Some(j + 1);
                }
                j += 1;
            }
            None
        }
        Some(_) if peek(chars, i + 2) == Some('\'') => Some(i + 3),
        _ => None,
    }
}

/// Tokenizes a Lean source file. Comments are included in the stream so the
/// parser can attach doc comments to the following declaration.
pub fn tokenize(src: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let total = chars.len();
    let len = src.len() as u32;
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    let mut newline_before = true;
    let mut line_start = 0u32;

    while i < total {
        let (raw_start, c) = chars[i];
        if c.is_whitespace() {
            if c == '\n' {
                newline_before = true;
                line_start = raw_start as u32 + 1;
            }
            i += 1;
            continue;
        }
        let start = raw_start as u32;

        // Line comment: `-- ...` up to the end of the line.
        if c == '-' && peek(&chars, i + 1) == Some('-') {
            let mut j = i + 2;
            while j < total && chars[j].1 != '\n' {
                j += 1;
            }
            out.push(Token {
                kind: TokenKind::LineComment,
                start,
                end: byte_end(&chars, j, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i = j;
            continue;
        }

        // Block comment: `/- ... -/`, nestable. `/--` is a doc comment,
        // `/-!` a module doc comment.
        if c == '/' && peek(&chars, i + 1) == Some('-') {
            let is_doc = peek(&chars, i + 2) == Some('-') && peek(&chars, i + 3) != Some('/');
            let is_module_doc = peek(&chars, i + 2) == Some('!');
            let mut depth = 0i32;
            let mut j = i;
            let mut end = len;
            while j < total {
                if chars[j].1 == '/' && peek(&chars, j + 1) == Some('-') {
                    depth += 1;
                    j += 2;
                    continue;
                }
                if chars[j].1 == '-' && peek(&chars, j + 1) == Some('/') {
                    depth -= 1;
                    j += 2;
                    if depth <= 0 {
                        end = byte_end(&chars, j, len);
                        break;
                    }
                    continue;
                }
                j += 1;
            }
            let kind = if is_doc {
                TokenKind::DocComment
            } else if is_module_doc {
                TokenKind::ModuleDoc
            } else {
                TokenKind::BlockComment
            };
            out.push(Token {
                kind,
                start,
                end,
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i = if j > i { j } else { i + 1 };
            continue;
        }

        // String literal, possibly interpolated (`s!"...{x}..."`); the `s!`
        // prefix is lexed separately as an identifier, which is harmless.
        if c == '"' {
            let mut j = i + 1;
            while j < total {
                match chars[j].1 {
                    '\\' => j += 2,
                    '"' => {
                        j += 1;
                        break;
                    }
                    _ => j += 1,
                }
            }
            let j = j.min(total);
            out.push(Token {
                kind: TokenKind::Str,
                start,
                end: byte_end(&chars, j, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i = j;
            continue;
        }

        // Character literal, or a lone `'` (used as a postfix in names).
        if c == '\'' {
            if let Some(next) = scan_char_literal(&chars, i) {
                out.push(Token {
                    kind: TokenKind::Char,
                    start,
                    end: byte_end(&chars, next, len),
                    newline_before,
                    col: start - line_start,
                });
                newline_before = false;
                i = next;
                continue;
            }
            out.push(Token {
                kind: TokenKind::Symbol,
                start,
                end: byte_end(&chars, i + 1, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i += 1;
            continue;
        }

        // Identifier, escaped identifier (`«name»`), dotted name or `#command`.
        if is_ident_start(c)
            || c == '«'
            || (c == '#' && peek(&chars, i + 1).is_some_and(is_ident_start))
        {
            let mut j = i;
            if c == '«' {
                j += 1;
                while j < total && chars[j].1 != '»' {
                    j += 1;
                }
                j = (j + 1).min(total);
            } else {
                j += 1;
                loop {
                    if j >= total {
                        break;
                    }
                    let d = chars[j].1;
                    if is_ident_char(d) {
                        j += 1;
                        continue;
                    }
                    if d == '«' {
                        j += 1;
                        while j < total && chars[j].1 != '»' {
                            j += 1;
                        }
                        j = (j + 1).min(total);
                        continue;
                    }
                    if d == '.'
                        && peek(&chars, j + 1).is_some_and(|e| is_ident_start(e) || e == '«')
                    {
                        j += 1;
                        continue;
                    }
                    break;
                }
            }
            out.push(Token {
                kind: TokenKind::Ident,
                start,
                end: byte_end(&chars, j, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i = j;
            continue;
        }

        // Number literal.
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < total {
                let d = chars[j].1;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else if d == '.'
                    && peek(&chars, j + 1).is_some_and(|e| e.is_ascii_digit())
                {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(Token {
                kind: TokenKind::Number,
                start,
                end: byte_end(&chars, j, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i = j;
            continue;
        }

        if bracket_pair(c).is_some() {
            let kind = if is_open_bracket(c) {
                TokenKind::Open
            } else {
                TokenKind::Close
            };
            out.push(Token {
                kind,
                start,
                end: byte_end(&chars, i + 1, len),
                newline_before,
                col: start - line_start,
            });
            newline_before = false;
            i += 1;
            continue;
        }

        // Operator run.
        let mut j = i;
        while j < total {
            let d = chars[j].1;
            if !is_symbol_char(d) {
                break;
            }
            if j > i {
                if d == '-' && peek(&chars, j + 1) == Some('-') {
                    break;
                }
                if d == '/' && peek(&chars, j + 1) == Some('-') {
                    break;
                }
            }
            j += 1;
        }
        if j == i {
            j = i + 1;
        }
        out.push(Token {
            kind: TokenKind::Symbol,
            start,
            end: byte_end(&chars, j, len),
            newline_before,
            col: start - line_start,
        });
        newline_before = false;
        i = j;
    }

    out
}
