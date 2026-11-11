//! Structural parser for Lean 4 source files.
//!
//! Lean has no command terminator: a command ends where the next one starts.
//! This parser therefore splits a file at *command starts*, detected as a
//! command keyword (or an attribute/modifier run) at the top bracket level of
//! the current block, at the start of a line, and not indented deeper than the
//! command it would otherwise continue. That is the same structural evidence
//! Lean's layout rules provide, and keeping expressions and proofs verbatim
//! means an edit can be spliced back into the file without rewriting anything.

use super::ast::{Diagnostic, LineIndex, NodeKind, ParsedFile, Severity, Span, SyntaxNode};
use super::lexer::{bracket_pair, tokenize, Token, TokenKind};

const MODIFIERS: &[&str] = &[
    "private",
    "protected",
    "noncomputable",
    "partial",
    "unsafe",
    "nonrec",
    "local",
    "scoped",
    "deprecated",
];

const EXTRA_COMMANDS: &[&str] = &[
    "run_cmd",
    "run_elab",
    "run_tac",
    "assert_not_exists",
    "add_decl_doc",
    "include",
    "omit",
    "suppress_compilation",
    "implemented_by",
];

/// How a body block is split into child nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryMode {
    Fields,
    Ctors,
    WhereDecls,
}

struct Classified {
    kind: NodeKind,
    keyword: Option<usize>,
    name: Option<String>,
    name_span: Option<Span>,
    span_start: u32,
}

struct Pending {
    kind: NodeKind,
    keyword: Option<usize>,
    name: Option<String>,
    name_span: Option<Span>,
    span_start: u32,
    start_col: u32,
    first_sig: usize,
    doc: Option<String>,
    doc_span: Option<Span>,
    comments: Option<String>,
    comment_span: Option<Span>,
}

struct Frame {
    kind: NodeKind,
    name: Option<String>,
    name_span: Option<Span>,
    span_start: u32,
    header_end: u32,
    doc: Option<String>,
    doc_span: Option<Span>,
    children: Vec<SyntaxNode>,
    pending: Option<Pending>,
}

impl Frame {
    fn root() -> Self {
        Frame {
            kind: NodeKind::File,
            name: None,
            name_span: None,
            span_start: 0,
            header_end: 0,
            doc: None,
            doc_span: None,
            children: Vec::new(),
            pending: None,
        }
    }
}

/// Parses a Lean source file into its structural command tree.
pub fn parse(src: &str) -> ParsedFile {
    Parser::new(src).run()
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    /// Indices into `toks` of the non-comment tokens.
    sig: Vec<usize>,
    /// Doc comment attached to each significant token, aligned with `sig`.
    docs: Vec<Option<(Span, String)>>,
    /// Plain comments above each significant token, aligned with `sig`.
    leading: Vec<Option<(Span, String)>>,
    module_doc: Option<String>,
    lines: LineIndex,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        let toks = tokenize(src);
        let mut sig: Vec<usize> = Vec::with_capacity(toks.len());
        let mut docs: Vec<Option<(Span, String)>> = Vec::new();
        let mut leading: Vec<Option<(Span, String)>> = Vec::new();
        let mut pending_doc: Option<(Span, String)> = None;
        let mut module_doc: Option<String> = None;
        let mut comment_start: Option<u32> = None;
        let mut comment_end: u32 = 0;
        let mut comment_lines: Vec<String> = Vec::new();
        for (idx, tok) in toks.iter().enumerate() {
            if tok.kind.is_comment() {
                let text = &src[tok.start as usize..tok.end as usize];
                match tok.kind {
                    TokenKind::DocComment => {
                        pending_doc =
                            Some((Span::new(tok.start, tok.end), strip_comment(text, 3)));
                    }
                    TokenKind::ModuleDoc => {
                        module_doc = Some(strip_comment(text, 3));
                        pending_doc = None;
                    }
                    _ => {
                        if comment_start.is_none() {
                            comment_start = Some(tok.start);
                        }
                        comment_end = tok.end;
                        comment_lines.push(text.trim_end().to_string());
                    }
                }
                continue;
            }
            sig.push(idx);
            docs.push(pending_doc.take());
            leading.push(comment_start.map(|start| {
                (
                    Span::new(start, comment_end),
                    comment_lines.join("\n"),
                )
            }));
            comment_start = None;
            comment_lines.clear();
        }
        Parser {
            src,
            toks,
            sig,
            docs,
            leading,
            module_doc,
            lines: LineIndex::new(src),
            diagnostics: Vec::new(),
        }
    }

    fn run(mut self) -> ParsedFile {
        let mut frames: Vec<Frame> = vec![Frame::root()];
        let mut depth: i32 = 0;
        let mut bracket_stack: Vec<(char, u32)> = Vec::new();

        let mut k = 0usize;
        while k < self.sig.len() {
            let tok = self.toks[self.sig[k]];
            let text = tok.text(self.src);
            let at_top = depth == 0;
            let is_end = at_top && tok.kind == TokenKind::Ident && text == "end";

            let keeps_deriving = text == "deriving"
                && frames
                    .last()
                    .and_then(|f| f.pending.as_ref())
                    .is_some_and(|p| {
                        matches!(
                            p.kind,
                            NodeKind::Structure | NodeKind::Class | NodeKind::Inductive
                        )
                    });
            let can_start = at_top
                && tok.newline_before
                && !keeps_deriving
                && self.is_command_start(k)
                && frames
                    .last()
                    .and_then(|f| f.pending.as_ref())
                    .is_none_or(|p| tok.col <= p.start_col);

            if is_end {
                self.flush(&mut frames, k);
                let mut end_name: Option<String> = None;
                let mut end_span = Span::new(tok.start, tok.end);
                if let Some(next) = self.sig.get(k + 1).copied() {
                    let name_tok = self.toks[next];
                    if name_tok.kind == TokenKind::Ident && !name_tok.newline_before {
                        end_name = Some(name_tok.text(self.src).to_string());
                        end_span = Span::new(tok.start, name_tok.end);
                        k += 1;
                    }
                }
                self.close_frame(&mut frames, tok.start, end_span, end_name.as_deref());
                depth = 0;
                k += 1;
                continue;
            }

            if can_start {
                self.flush(&mut frames, k);
                depth = 0;
                let class = self.classify(k);
                if class.kind.is_block() {
                    let header_end = match class.name_span {
                        Some(span) => span.end,
                        None => class
                            .keyword
                            .map(|kw| self.toks[self.sig[kw]].end)
                            .unwrap_or(tok.end),
                    };
                    frames.push(Frame {
                        kind: class.kind,
                        name: class.name,
                        name_span: class.name_span,
                        span_start: class.span_start,
                        header_end,
                        doc: self.docs[k].as_ref().map(|(_, text)| text.clone()),
                        doc_span: self.docs[k].as_ref().map(|(span, _)| *span),
                        children: Vec::new(),
                        pending: None,
                    });
                    k += 1;
                    continue;
                }
                let (doc, doc_span) = match &self.docs[k] {
                    Some((span, text)) => (Some(text.clone()), Some(*span)),
                    None => (None, None),
                };
                let (comments, comment_span) = match &self.leading[k] {
                    Some((span, text)) => (Some(text.clone()), Some(*span)),
                    None => (None, None),
                };
                frames.last_mut().unwrap().pending = Some(Pending {
                    kind: class.kind,
                    keyword: class.keyword,
                    name: class.name,
                    name_span: class.name_span,
                    span_start: class.span_start,
                    start_col: tok.col,
                    first_sig: k,
                    doc,
                    doc_span,
                    comments,
                    comment_span,
                });
            }

            if tok.kind == TokenKind::Open {
                depth += 1;
                bracket_stack.push((text.chars().next().unwrap_or('('), tok.start));
            } else if tok.kind == TokenKind::Close {
                if depth > 0 {
                    depth -= 1;
                }
                let closer = text.chars().next().unwrap_or(' ');
                if let Some(idx) = bracket_stack
                    .iter()
                    .rposition(|(open, _)| bracket_pair(*open) == Some(closer))
                {
                    bracket_stack.remove(idx);
                }
            }
            k += 1;
        }

        self.flush(&mut frames, self.sig.len());

        for (open, offset) in &bracket_stack {
            let line = self.lines.line_of(*offset);
            self.diagnostics.push(Diagnostic {
                line,
                severity: Severity::Error,
                message: format!("bracket `{open}` is never closed"),
            });
        }

        let last_end = self
            .sig
            .last()
            .map(|idx| self.toks[*idx].end)
            .unwrap_or(0);
        while frames.len() > 1 {
            let frame = frames.pop().unwrap();
            let line = self.lines.line_of(frame.span_start);
            let label = match frame.name.as_deref() {
                Some(name) => format!("{} `{name}`", frame.kind.label()),
                None => frame.kind.label().to_string(),
            };
            self.diagnostics.push(Diagnostic {
                line,
                severity: Severity::Error,
                message: format!("{label} is never closed by `end`"),
            });
            let node = self.frame_node(frame, last_end, last_end);
            frames.last_mut().unwrap().children.push(node);
        }

        let root = frames.pop().unwrap();
        ParsedFile {
            nodes: root.children,
            diagnostics: self.diagnostics,
            module_doc: self.module_doc,
        }
    }

    // ---------------------------------------------------------------- helpers

    fn text(&self, k: usize) -> &'a str {
        let tok = self.toks[self.sig[k]];
        &self.src[tok.start as usize..tok.end as usize]
    }

    fn is_command_start(&self, k: usize) -> bool {
        let tok = self.toks[self.sig[k]];
        let text = self.text(k);
        match tok.kind {
            TokenKind::Ident => {
                text.starts_with('#')
                    || NodeKind::from_keyword(text).is_some()
                    || MODIFIERS.contains(&text)
                    || EXTRA_COMMANDS.contains(&text)
            }
            TokenKind::Symbol => {
                text == "@"
                    && self
                        .sig
                        .get(k + 1)
                        .is_some_and(|next| {
                            let t = self.toks[*next];
                            t.kind == TokenKind::Open && t.text(self.src) == "["
                        })
            }
            _ => false,
        }
    }

    fn skip_attribute(&self, at: usize) -> Option<usize> {
        let open = self.sig.get(at + 1).copied()?;
        if self.toks[open].kind != TokenKind::Open {
            return None;
        }
        let mut depth = 0i32;
        for j in at + 1..self.sig.len() {
            let tok = self.toks[self.sig[j]];
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
        }
        None
    }

    fn classify(&self, from: usize) -> Classified {
        let span_start = self.toks[self.sig[from]].start;
        let mut i = from;
        loop {
            if i >= self.sig.len() {
                return Classified {
                    kind: NodeKind::Command,
                    keyword: None,
                    name: None,
                    name_span: None,
                    span_start,
                };
            }
            let tok = self.toks[self.sig[i]];
            let text = self.text(i);
            if tok.kind == TokenKind::Symbol && text == "@" {
                if let Some(next) = self.skip_attribute(i) {
                    i = next;
                    continue;
                }
            }
            if tok.kind == TokenKind::Ident && MODIFIERS.contains(&text) {
                i += 1;
                continue;
            }
            break;
        }
        if i >= self.sig.len() {
            return Classified {
                kind: NodeKind::Command,
                keyword: None,
                name: None,
                name_span: None,
                span_start,
            };
        }
        let tok = self.toks[self.sig[i]];
        let text = self.text(i);
        let kind = if tok.kind == TokenKind::Ident {
            NodeKind::from_keyword(text).unwrap_or(NodeKind::Command)
        } else {
            NodeKind::Command
        };

        let mut name_at = i + 1;
        if kind == NodeKind::Import {
            while name_at < self.sig.len()
                && self.toks[self.sig[name_at]].kind == TokenKind::Ident
                && matches!(self.text(name_at), "all" | "runtime")
            {
                name_at += 1;
            }
        }
        let (name, name_span) = match kind {
            NodeKind::Prelude
            | NodeKind::Variable
            | NodeKind::Universe
            | NodeKind::Attribute => (None, None),
            NodeKind::Notation | NodeKind::Syntax | NodeKind::Macro => {
                match self.sig.get(name_at).copied() {
                    Some(idx) => {
                        let t = self.toks[idx];
                        (
                            Some(t.text(self.src).to_string()),
                            Some(Span::new(t.start, t.end)),
                        )
                    }
                    None => (None, None),
                }
            }
            _ => match self.sig.get(name_at).copied() {
                Some(idx) if self.toks[idx].kind == TokenKind::Ident => {
                    let t = self.toks[idx];
                    (
                        Some(t.text(self.src).to_string()),
                        Some(Span::new(t.start, t.end)),
                    )
                }
                _ => (None, None),
            },
        };

        Classified {
            kind,
            keyword: Some(i),
            name,
            name_span,
            span_start,
        }
    }

    fn flush(&mut self, frames: &mut Vec<Frame>, k: usize) {
        let pending = match frames.last_mut().and_then(|f| f.pending.take()) {
            Some(p) => p,
            None => return,
        };
        if k == 0 || pending.first_sig >= k {
            return;
        }
        let node = self.build_command(&pending, k - 1);
        if let Some(frame) = frames.last_mut() {
            frame.children.push(node);
        }
    }

    fn build_command(&self, pending: &Pending, last_sig: usize) -> SyntaxNode {
        let span_end = self.toks[self.sig[last_sig]].end;
        let span = Span::new(pending.span_start, span_end);
        let delim = self.find_delimiter(pending.first_sig, last_sig);
        let header_end = match delim {
            Some((idx, _)) => self.toks[self.sig[idx]].start,
            None => span_end,
        };
        let header = self.trim(pending.span_start, header_end);
        let body = delim.map(|(idx, _)| {
            let start = self.toks[self.sig[idx]].end;
            self.trim(start, span_end)
        });
        let children = self.command_children(pending.kind, pending.first_sig, last_sig, delim);
        let signature = self.signature_text(pending, header);
        SyntaxNode {
            kind: pending.kind,
            name: pending.name.clone(),
            span,
            name_span: pending.name_span,
            header: Some(header),
            body,
            doc: pending.doc.clone(),
            doc_span: pending.doc_span,
            comments: pending.comments.clone(),
            comment_span: pending.comment_span,
            signature,
            children,
        }
    }

    fn signature_text(&self, pending: &Pending, header: Span) -> Option<String> {
        let start = pending
            .name_span
            .map(|span| span.end)
            .or_else(|| {
                pending
                    .keyword
                    .map(|kw| self.toks[self.sig[kw]].end)
            })
            .unwrap_or(header.start);
        if start >= header.end {
            return None;
        }
        let text = normalize(&strip_comments(&self.src[start as usize..header.end as usize]));
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn find_delimiter(&self, first: usize, last: usize) -> Option<(usize, Delimiter)> {
        let mut depth = 0i32;
        for k in first..=last {
            let tok = self.toks[self.sig[k]];
            let text = self.text(k);
            if depth == 0 {
                match tok.kind {
                    TokenKind::Symbol if text == ":=" => return Some((k, Delimiter::Assign)),
                    TokenKind::Symbol if text == "|" => return Some((k, Delimiter::Bar)),
                    TokenKind::Ident if text == "where" => return Some((k, Delimiter::Where)),
                    _ => {}
                }
            }
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }
        None
    }

    fn find_keyword(&self, first: usize, last: usize, keyword: &str) -> Option<usize> {
        let mut depth = 0i32;
        for k in first..=last {
            let tok = self.toks[self.sig[k]];
            if depth == 0 && tok.kind == TokenKind::Ident && self.text(k) == keyword {
                return Some(k);
            }
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }
        None
    }

    fn command_children(
        &self,
        kind: NodeKind,
        first: usize,
        last: usize,
        delim: Option<(usize, Delimiter)>,
    ) -> Vec<SyntaxNode> {
        match kind {
            NodeKind::Structure | NodeKind::Class => match self.find_keyword(first, last, "where") {
                Some(where_idx) => self.split_entries(where_idx + 1, last, EntryMode::Fields),
                None => Vec::new(),
            },
            NodeKind::Inductive => {
                let from = self
                    .find_keyword(first, last, "where")
                    .map(|idx| idx + 1)
                    .or_else(|| {
                        // Fall back to the token after the declared name.
                        let name = self.find_keyword(first, last, "inductive")?;
                        Some(name + 2)
                    })
                    .unwrap_or(last + 1);
                self.split_entries(from, last, EntryMode::Ctors)
            }
            kind if kind.has_where_block() => match delim {
                Some((idx, Delimiter::Where)) => {
                    self.split_entries(idx + 1, last, EntryMode::WhereDecls)
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn split_entries(&self, from: usize, to: usize, mode: EntryMode) -> Vec<SyntaxNode> {
        if from > to || from >= self.sig.len() {
            return Vec::new();
        }
        let to = to.min(self.sig.len() - 1);

        let mut depths: Vec<i32> = Vec::with_capacity(to - from + 1);
        let mut depth = 0i32;
        for k in from..=to {
            depths.push(depth);
            let tok = self.toks[self.sig[k]];
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }

        let mut starts: Vec<usize> = Vec::new();
        for k in from..=to {
            if depths[k - from] != 0 {
                continue;
            }
            let tok = self.toks[self.sig[k]];
            let text = self.text(k);
            let candidate = match mode {
                EntryMode::Ctors => tok.kind == TokenKind::Symbol && text == "|",
                _ => {
                    if k == from {
                        tok.kind == TokenKind::Ident
                    } else {
                        tok.newline_before
                            && (tok.kind == TokenKind::Ident
                                || (tok.kind == TokenKind::Symbol && text == "@"))
                    }
                }
            };
            if candidate {
                starts.push(k);
            }
        }

        let mut out = Vec::new();
        for (i, &start) in starts.iter().enumerate() {
            let end = if i + 1 < starts.len() {
                starts[i + 1] - 1
            } else {
                to
            };
            if end < start {
                continue;
            }
            if mode != EntryMode::Ctors && !self.segment_has_colon(start, end) {
                continue;
            }
            out.push(self.build_entry(mode, start, end));
        }
        out
    }

    fn segment_has_colon(&self, first: usize, last: usize) -> bool {
        let mut depth = 0i32;
        for k in first..=last {
            let tok = self.toks[self.sig[k]];
            let text = self.text(k);
            if depth == 0 && tok.kind == TokenKind::Symbol && (text == ":" || text == ":=") {
                return true;
            }
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }
        false
    }

    fn build_entry(&self, mode: EntryMode, first: usize, last: usize) -> SyntaxNode {
        let start = self.toks[self.sig[first]].start;
        let end = self.toks[self.sig[last]].end;
        let mut name_span: Option<Span> = None;
        let mut colon_span: Option<Span> = None;
        let mut depth = 0i32;
        for k in first..=last {
            let tok = self.toks[self.sig[k]];
            let text = self.text(k);
            if depth == 0 {
                if name_span.is_none() && tok.kind == TokenKind::Ident {
                    name_span = Some(Span::new(tok.start, tok.end));
                }
                if colon_span.is_none()
                    && tok.kind == TokenKind::Symbol
                    && (text == ":" || text == ":=")
                {
                    colon_span = Some(Span::new(tok.start, tok.end));
                }
            }
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }

        let kind = match mode {
            EntryMode::Fields => NodeKind::Field,
            EntryMode::Ctors => NodeKind::Constructor,
            EntryMode::WhereDecls => NodeKind::WhereDecl,
        };

        let signature = match mode {
            EntryMode::Ctors => {
                let from = name_span.map(|span| span.end).unwrap_or(start);
                if from < end {
                    Some(normalize(&self.src[from as usize..end as usize]))
                } else {
                    None
                }
            }
            _ => match (colon_span, name_span) {
                (Some(colon), Some(name)) => {
                    let type_start = colon.end;
                    let type_end = self
                        .find_assign_end(first, last, type_start)
                        .unwrap_or(end);
                    if type_start < type_end {
                        let text = normalize(&self.src[type_start as usize..type_end as usize]);
                        if text.is_empty() {
                            None
                        } else {
                            let _ = name;
                            Some(text)
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
        };

        let name = name_span.map(|span| self.src[span.start as usize..span.end as usize].to_string());
        SyntaxNode {
            kind,
            name,
            span: Span::new(start, end),
            name_span,
            header: Some(Span::new(start, colon_span.map(|c| c.start).unwrap_or(end))),
            body: colon_span.map(|colon| Span::new(colon.end, end)),
            doc: None,
            doc_span: None,
            comments: None,
            comment_span: None,
            signature,
            children: Vec::new(),
        }
    }

    fn find_assign_end(&self, first: usize, last: usize, from: u32) -> Option<u32> {
        let mut depth = 0i32;
        for k in first..=last {
            let tok = self.toks[self.sig[k]];
            if tok.start < from {
                continue;
            }
            if depth == 0 && tok.kind == TokenKind::Symbol && self.text(k) == ":=" {
                return Some(tok.start);
            }
            if tok.kind == TokenKind::Open {
                depth += 1;
            } else if tok.kind == TokenKind::Close && depth > 0 {
                depth -= 1;
            }
        }
        None
    }

    fn close_frame(
        &mut self,
        frames: &mut Vec<Frame>,
        end_start: u32,
        end_span: Span,
        end_name: Option<&str>,
    ) {
        if frames.len() < 2 {
            let line = self.lines.line_of(end_start);
            self.diagnostics.push(Diagnostic {
                line,
                severity: Severity::Error,
                message: "`end` without a matching `namespace`, `section` or `mutual`"
                    .to_string(),
            });
            return;
        }
        let frame = frames.pop().unwrap();
        if let (Some(expected), Some(found)) = (frame.name.as_deref(), end_name) {
            if expected != found {
                let line = self.lines.line_of(end_start);
                self.diagnostics.push(Diagnostic {
                    line,
                    severity: Severity::Warning,
                    message: format!("`end {found}` closes `{} {expected}`", frame.kind.label()),
                });
            }
        }
        let node = self.frame_node(frame, end_span.end, end_start);
        if let Some(parent) = frames.last_mut() {
            parent.children.push(node);
        }
    }

    fn frame_node(&self, frame: Frame, span_end: u32, body_end: u32) -> SyntaxNode {
        let span = Span::new(frame.span_start, span_end);
        let header = self.trim(frame.span_start, frame.header_end.min(span_end));
        let body = if frame.header_end < body_end {
            let trimmed = self.trim(frame.header_end, body_end);
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        } else {
            None
        };
        SyntaxNode {
            kind: frame.kind,
            name: frame.name,
            span,
            name_span: frame.name_span,
            header: Some(header),
            body,
            doc: frame.doc,
            doc_span: frame.doc_span,
            comments: None,
            comment_span: None,
            signature: None,
            children: frame.children,
        }
    }

    fn trim(&self, start: u32, end: u32) -> Span {
        let bytes = self.src.as_bytes();
        let mut s = start.min(end);
        let mut e = end.min(self.src.len() as u32);
        while s < e && is_ascii_ws(bytes[s as usize]) {
            s += 1;
        }
        while e > s && is_ascii_ws(bytes[e as usize - 1]) {
            e -= 1;
        }
        Span::new(s, e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delimiter {
    Assign,
    Where,
    Bar,
}

fn is_ascii_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// Strips a comment's opening marker (`/--`, `/-!`, ...) and its closing `-/`.
fn strip_comment(text: &str, opener_len: usize) -> String {
    let mut body = text;
    if body.len() >= opener_len {
        body = &body[opener_len..];
    }
    if let Some(stripped) = body.strip_suffix("-/") {
        body = stripped;
    } else if let Some(stripped) = body.strip_suffix('/') {
        // Unterminated comment while typing: drop the dangling slash.
        body = stripped;
    }
    body.trim().to_string()
}

/// Drops comments from a fragment so signatures stay one clean line.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            let mut depth = 0i32;
            while i < bytes.len() {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    if depth <= 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            continue;
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&text[i..(i + ch_len).min(text.len())]);
        i += ch_len;
    }
    out
}

fn utf8_len(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if byte >> 5 == 0b110 {
        2
    } else if byte >> 4 == 0b1110 {
        3
    } else if byte >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Collapses whitespace runs into single spaces and truncates long text.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    let out = out.trim().to_string();
    truncate(&out, 96)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}
