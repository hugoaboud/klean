//! The syntax model the board renders, built from Lean's own syntax tree.
//!
//! KLEAN does not parse Lean: [`crate::lean::engine`] asks the `klean-ast`
//! library, which runs Lean's parser, and this module is the only place that
//! knows what the resulting syntax *means* for the board. It picks the commands
//! out of the tree, maps Lean's syntax kinds to the short labels the UI uses,
//! nests `namespace`/`section`/`mutual` blocks, finds structure fields,
//! inductive constructors and `where` declarations, and reads the comments that
//! sit above a command (Lean's tree does not carry them).
//!
//! Every span is a byte offset into the file, which is what makes editing safe:
//! an edit replaces exactly the node's span and leaves everything around it
//! untouched.

use klean_ast::{FileAst, Node as SyntaxNode};

/// Byte range inside a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Converts a span reported by the parser library.
    pub fn from_lean(span: klean_ast::Span) -> Span {
        Span::new(span.start, span.end)
    }

    pub fn new(start: u32, end: u32) -> Self {
        Span { start, end }
    }

    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn contains(self, offset: u32) -> bool {
        offset >= self.start && offset <= self.end
    }

    /// The text this span covers in `source`.
    pub fn slice<'a>(self, source: &'a str) -> &'a str {
        let start = (self.start as usize).min(source.len());
        let end = (self.end as usize).min(source.len());
        if start > end {
            return "";
        }
        &source[start..end]
    }

    /// Union of two spans; `Span::EMPTY` is the identity.
    pub fn join(self, other: Span) -> Span {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    pub const EMPTY: Span = Span { start: 0, end: 0 };

}

/// Kinds that declare something and can carry a proof state.
pub fn is_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "def"
            | "theorem"
            | "lemma"
            | "axiom"
            | "structure"
            | "class"
            | "inductive"
            | "instance"
            | "abbrev"
            | "opaque"
            | "example"
            | "syntax"
            | "macro"
            | "macro_rules"
            | "notation"
            | "elab"
    )
}

/// Kinds whose body is a block of commands, closed by `end`.
pub fn is_block(kind: &str) -> bool {
    matches!(kind, "namespace" | "section" | "mutual")
}

/// Kinds that hold other nodes in the tree.
pub fn is_container(kind: &str) -> bool {
    is_block(kind) || matches!(kind, "structure" | "class" | "inductive" | "file")
}

/// `import` and `prelude` commands.
pub fn is_import(kind: &str) -> bool {
    matches!(kind, "import" | "prelude")
}

/// A node of the syntax tree.
#[derive(Debug, Clone)]
pub struct AstNode {
    /// Lean's short kind label: `def`, `theorem`, `#check`, `namespace`, …
    pub kind: String,
    pub name: Option<String>,
    /// Whole command, including attributes and the doc comment.
    pub span: Span,
    /// The command without its modifiers: what the editor shows and rewrites.
    pub inner: Span,
    pub name_span: Option<Span>,
    /// The `: type` / binders part of a declaration.
    pub sig_span: Option<Span>,
    /// Doc comment, already stripped of its markers.
    pub doc: Option<String>,
    /// Plain (`--`, `/- -/`) comments directly above the node, plain text.
    pub comments: Option<String>,
    pub comment_span: Option<Span>,
    /// Compact one-line signature, for tree labels.
    pub signature: Option<String>,
    /// Distinct identifiers used inside the node, for the logic board.
    pub idents: Vec<String>,
    pub children: Vec<AstNode>,
}

impl AstNode {
    /// Human readable label such as `def MerkleRoot : Type`.
    pub fn label(&self) -> String {
        match (self.name.as_deref(), self.signature.as_deref()) {
            (Some(name), Some(sig)) if !sig.is_empty() => format!("{} {} {}", self.kind, name, sig),
            (Some(name), _) => format!("{} {}", self.kind, name),
            (None, Some(sig)) if !sig.is_empty() => format!("{} {}", self.kind, sig),
            (None, _) => self.kind.clone(),
        }
    }

    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a AstNode)) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    /// Lean's `#check` output and other informational messages.
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// One elaborator message, positioned inside the file.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// 1-based line number the diagnostic starts on.
    pub line: u32,
    /// 1-based line it ends on (the same as `line` for a point).
    pub end_line: u32,
    /// Columns, as Lean reported them.
    pub span: Span,
    pub severity: Severity,
    pub message: String,
}

/// Result of handing one file to Lean.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub nodes: Vec<AstNode>,
    pub diagnostics: Vec<Diagnostic>,
    /// Module doc comment (`/-! … -/`) at the top of the file, if any.
    pub module_doc: Option<String>,
}

impl ParsedFile {
    pub fn node_count(&self) -> usize {
        let mut count = 0;
        for node in &self.nodes {
            node.walk(&mut |_| count += 1);
        }
        count
    }

    pub fn count_kind(&self, kind: &str) -> usize {
        let mut count = 0;
        for node in &self.nodes {
            node.walk(&mut |n| {
                if n.kind == kind {
                    count += 1;
                }
            });
        }
        count
    }

    /// Resolves an index path (`[0, 2, 1]`) to a node.
    pub fn get(&self, path: &[usize]) -> Option<&AstNode> {
        let (first, rest) = path.split_first()?;
        let mut node = self.nodes.get(*first)?;
        for index in rest {
            node = node.children.get(*index)?;
        }
        Some(node)
    }

    /// Distinct identifiers used anywhere in the file.
    pub fn idents(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for node in &self.nodes {
            node.walk(&mut |n| {
                for ident in &n.idents {
                    if !out.contains(ident) {
                        out.push(ident.clone());
                    }
                }
            });
        }
        out
    }
}

/// Helpers for turning byte offsets into 1-based line numbers.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0u32];
        for (offset, ch) in src.char_indices() {
            if ch == '\n' {
                starts.push(offset as u32 + 1);
            }
        }
        LineIndex { starts }
    }

    pub fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }

    /// 1-based line of a byte offset.
    pub fn line_of(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(index) => index as u32 + 1,
            Err(index) => index as u32,
        }
    }

    /// Byte offset of the start of a 1-based line.
    pub fn start_of(&self, line: u32) -> u32 {
        let index = (line.max(1) as usize - 1).min(self.starts.len().saturating_sub(1));
        self.starts.get(index).copied().unwrap_or(0)
    }
}

// ------------------------------------------------------------------- mapping

/// Lean's syntax kind → the short label the board shows.
fn label_of(short: &str) -> Option<&'static str> {
    Some(match short {
        "definition" | "def" => "def",
        "theorem" => "theorem",
        "lemma" => "lemma",
        "abbrev" => "abbrev",
        "opaque" => "opaque",
        "axiom" | "constant" => "axiom",
        "instance" => "instance",
        "example" => "example",
        "structure" => "structure",
        "class" => "class",
        "inductive" => "inductive",
        "coinductive" => "inductive",
        "namespace" => "namespace",
        "section" => "section",
        "mutual" => "mutual",
        "end" => "end",
        "import" => "import",
        "prelude" => "prelude",
        "open" => "open",
        "export" => "export",
        "variable" => "variable",
        "universe" | "universes" => "universe",
        "set_option" => "set_option",
        "attribute" => "attribute",
        "notation" | "mixfix" | "infix" | "infixl" | "infixr" | "prefix" | "postfix" => "notation",
        "syntax" | "notation3" => "syntax",
        "macro" => "macro",
        "macro_rules" => "macro_rules",
        "elab" | "elab_rules" => "elab",
        "deriving" => "deriving",
        "initialize" => "initialize",
        "check" => "#check",
        "eval" => "#eval",
        "reduce" => "#reduce",
        "print" => "#print",
        "synth" => "#synth",
        "unfold" => "#unfold",
        _ => return None,
    })
}

/// Kinds that carry no row of their own in the board.
fn hidden(short: &str) -> bool {
    matches!(short, "declModifiers" | "null" | "eoi" | "commentBody" | "moduleDoc")
}

/// Peels the wrappers Lean puts around a command until the command itself shows:
/// a `declaration` holds the modifiers (doc comment, attributes) and then the
/// `definition`/`theorem`/… it declares.
fn command_of<'a>(node: &'a SyntaxNode) -> Option<&'a SyntaxNode> {
    if node.short_kind() == "declaration" {
        return node
            .children
            .iter()
            .find(|child| !hidden(child.short_kind()) && child.short_kind() != "declaration");
    }
    if node.short_kind() == "in" {
        // `open Foo in <command>` / `set_option … in <command>`.
        return node
            .children
            .iter()
            .find(|child| child.short_kind() == "declaration")
            .and_then(command_of)
            .or_else(|| node.children.iter().rev().find(|child| !hidden(child.short_kind())));
    }
    Some(node)
}

/// The declared name of a command: its `declId`, else the identifier it starts
/// with (`namespace Foo`, `#check Nat`, …).
fn name_of(command: &SyntaxNode) -> Option<(String, Option<Span>)> {
    if let Some(decl) = command.find("declId") {
        let span = decl.span().map(Span::from_lean);
        // `def Foo.bar` has one identifier per component; the joined text is the
        // name the rest of the engine compares against.
        let mut idents = Vec::new();
        decl.find_all("ident", &mut idents);
        let name = idents
            .iter()
            .filter_map(|ident| ident.text.clone())
            .collect::<Vec<_>>()
            .join(".");
        if !name.is_empty() {
            return Some((name, span));
        }
    }
    let ident = command.first_ident()?;
    Some((ident.text.clone()?, ident.span().map(Span::from_lean)))
}

/// The `: type` / binder part of a declaration, as one line.
fn signature_of(command: &SyntaxNode) -> Option<Span> {
    command
        .find("optDeclSig")
        .or_else(|| command.find("declSig"))
        .and_then(|node| node.span())
        .map(Span::from_lean)
}

/// Structure fields, inductive constructors and `where` declarations: the rows
/// that sit inside a declaration rather than beside it.
fn children_of<'a>(command: &'a SyntaxNode, label: &str) -> Vec<&'a SyntaxNode> {
    match label {
        "structure" | "class" => command
            .find("structFields")
            .map(|fields| {
                let mut out = Vec::new();
                for kind in ["structSimpleBinder", "structExplicitBinder", "structSubobject"] {
                    fields.find_all(kind, &mut out);
                }
                out
            })
            .unwrap_or_default(),
        "inductive" => {
            let mut out = Vec::new();
            command.find_all("ctor", &mut out);
            out
        }
        _ => {
            let Some(where_decls) = command.find("whereDecls") else {
                return Vec::new();
            };
            let mut out = Vec::new();
            where_decls.find_all("letDecl", &mut out);
            out
        }
    }
}

/// The doc comment text (`/-- … -/`) sitting in a command's modifiers.
fn doc_of(source: &str, declaration: &SyntaxNode) -> Option<String> {
    let doc = declaration.find("docComment")?;
    let span = doc.span()?;
    let text = bare_comment(span.slice(source));
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Builds the board's tree from Lean's syntax tree.
pub fn build(source: &str, file: &FileAst) -> ParsedFile {
    let mut nodes: Vec<AstNode> = Vec::new();
    // Open `namespace`/`section`/`mutual` blocks, innermost last.
    let mut stack: Vec<AstNode> = Vec::new();

    for syntax in file.commands() {
        let Some(command) = command_of(syntax) else {
            continue;
        };
        let short = command.short_kind();
        if hidden(short) {
            continue;
        }
        // `end` closes the innermost block and becomes part of it.
        if short == "end" {
            if let Some(mut block) = stack.pop() {
                if let Some(span) = syntax.span().map(Span::from_lean) {
                    block.span = block.span.join(span);
                    block.inner = block.inner.join(span);
                }
                push(&mut nodes, &mut stack, block);
            }
            continue;
        }
        let Some(label) = label_of(short) else {
            continue;
        };
        let span = syntax.span().or_else(|| command.span()).map(Span::from_lean);
        let Some(span) = span else {
            continue;
        };
        let inner = command.span().map(Span::from_lean).unwrap_or(span);
        let mut node = AstNode {
            kind: label.to_string(),
            name: None,
            span,
            inner,
            name_span: None,
            sig_span: None,
            doc: doc_of(source, syntax),
            comments: None,
            comment_span: None,
            signature: None,
            idents: idents_of(command),
            children: Vec::new(),
        };
        if is_declaration(label) || matches!(label, "namespace" | "section" | "mutual") {
            if let Some((name, name_span)) = name_of(command) {
                node.name = Some(name);
                        node.name_span = name_span;
            }
        }
        if let Some(sig_span) = signature_of(command) {
            let text = compact(sig_span.slice(source).to_string());
            if !text.is_empty() {
                node.sig_span = Some(sig_span);
                node.signature = Some(text);
            }
        }
        let label_owned = node.kind.clone();
        for child in children_of(command, &label_owned) {
            let Some(child_span) = child.span().map(Span::from_lean) else {
                continue;
            };
            let kind = match label_owned.as_str() {
                "structure" | "class" => "field",
                "inductive" => "constructor",
                _ => "where",
            };
            let mut row = AstNode {
                kind: kind.to_string(),
                name: child
                    .find("declId")
                    .and_then(|decl| decl.first_ident())
                    .or_else(|| child.first_ident())
                    .and_then(|ident| ident.text.clone()),
                span: child_span,
                inner: child_span,
                name_span: child.find("declId").and_then(|decl| decl.span()).map(Span::from_lean),
                sig_span: None,
                doc: None,
                comments: None,
                comment_span: None,
                signature: None,
                idents: Vec::new(),
                children: Vec::new(),
            };
            if let Some(sig_span) = signature_of(child) {
                let text = compact(sig_span.slice(source).to_string());
                if !text.is_empty() {
                    row.sig_span = Some(sig_span);
                    row.signature = Some(text);
                }
            }
            node.children.push(row);
        }
        if is_block(label) {
            stack.push(node);
        } else {
            push(&mut nodes, &mut stack, node);
        }
    }
    // Unterminated blocks (a syntax error): keep whatever they hold.
    while let Some(block) = stack.pop() {
        push(&mut nodes, &mut stack, block);
    }

    decorate_comments(source, &mut nodes);

    ParsedFile {
        nodes,
        diagnostics: Vec::new(),
        module_doc: module_doc(source),
    }
}

/// Every module a file imports, in file order: what the parser needs to know so
/// that syntax extensions from those modules are available.
pub fn imports_of(file: &FileAst) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for node in file.commands() {
        let mut imports = Vec::new();
        node.find_all("import", &mut imports);
        for import in imports {
            let mut idents = Vec::new();
            import.find_all("ident", &mut idents);
            let name: String = idents
                .iter()
                .filter_map(|ident| ident.text.clone())
                .collect::<Vec<_>>()
                .join(".");
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    if !out.iter().any(|module| module == "prelude") {
        // Every Lean file sees `Init` and `Lean`, implicitly or through its
        // imports; asking for them explicitly keeps core syntax available even
        // when a module the file imports has not been built yet.
        for module in ["Init", "Lean"] {
            if !out.iter().any(|existing| existing == module) {
                out.push(module.to_string());
            }
        }
    }
    out
}

/// Distinct identifiers used inside a command, for the logic board's uses.
fn idents_of(command: &SyntaxNode) -> Vec<String> {
    let mut idents = Vec::new();
    command.find_all("ident", &mut idents);
    let mut out: Vec<String> = Vec::new();
    for ident in idents {
        if let Some(text) = &ident.text {
            if !out.iter().any(|existing| existing == text) {
                out.push(text.clone());
            }
        }
    }
    out
}

fn push(nodes: &mut Vec<AstNode>, stack: &mut [AstNode], node: AstNode) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => nodes.push(node),
    }
}

/// Slices the plain comments that sit directly above each node.
fn decorate_comments(source: &str, nodes: &mut [AstNode]) {
    let mut previous_end = 0u32;
    for node in nodes.iter_mut() {
        let region = Span::new(previous_end, node.span.start.max(previous_end));
        if let Some((text, span)) = trailing_comments(source, region) {
            node.comments = Some(text);
            node.comment_span = Some(span);
        }
        previous_end = node.span.end;
        // Fields and constructors carry comments of their own.
        decorate_children(source, node);
    }
}

fn decorate_children(source: &str, node: &mut AstNode) {
    let mut previous_end = node.inner.start;
    for child in node.children.iter_mut() {
        let region = Span::new(previous_end, child.span.start.max(previous_end));
        if let Some((text, span)) = trailing_comments(source, region) {
            child.comments = Some(text);
            child.comment_span = Some(span);
        }
        previous_end = child.span.end;
    }
}

/// The `--` comments that belong to a node, with the span they cover.
///
/// Only line comments count, and only when they sit *directly* above the node: a
/// blank line between the comments and the declaration means they belong to
/// whatever comes next, not to this node. Block comments (`/- … -/`) are treated
/// as prose, not as this node's note, which is also why KLEAN writes notes back
/// as `--` lines.
fn trailing_comments(source: &str, region: Span) -> Option<(String, Span)> {
    let text = slice(source, region);
    if text.trim().is_empty() {
        return None;
    }
    let mut offsets: Vec<(u32, &str)> = Vec::new();
    let mut offset = region.start;
    for line in text.split_inclusive('\n') {
        offsets.push((offset, line));
        offset += line.len() as u32;
    }

    // Walk up from the node. The first line has to be a comment: anything else
    // (a blank line included) means the node has no note of its own.
    let mut collected: Vec<usize> = Vec::new();
    for index in (0..offsets.len()).rev() {
        let line = offsets[index].1.trim();
        if line.is_empty() {
            if collected.is_empty() {
                // A blank line right above the node: no comment belongs to it.
                return None;
            }
            break;
        }
        if !line.starts_with("--") {
            break;
        }
        collected.push(index);
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    let first = *collected.first()?;
    let last = *collected.last()?;
    let raw: String = offsets[first..=last].iter().map(|(_, line)| *line).collect();
    let plain = bare_comment(&raw);
    if plain.trim().is_empty() {
        return None;
    }
    let span = Span::new(
        offsets[first].0,
        offsets[last].0 + offsets[last].1.trim_end().len() as u32,
    );
    Some((plain, span))
}

/// The module doc comment at the top of a file, if it opens with `/-!`.
fn module_doc(source: &str) -> Option<String> {
    let mut rest = source.trim_start();
    if !rest.starts_with("/-!") {
        return None;
    }
    let end = rest.find("-/")?;
    let body = &rest[3..end];
    rest = &rest[end + 2..];
    let _ = rest;
    Some(
        body.lines()
            .map(|line| line.trim_start_matches('*').trim())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
    )
}

/// One line of text out of a multi-line span.
fn compact(text: String) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let part = line.trim();
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
        if out.chars().count() > 96 {
            break;
        }
    }
    let mut out: String = out.chars().take(96).collect();
    if out.chars().count() >= 96 {
        out.push('…');
    }
    out
}

/// Drops Lean's comment markers from a comment block, keeping plain text.
pub fn bare_comment(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in raw.lines() {
        let mut text = line.trim();
        for marker in ["/--", "/-!", "/-", "-/", "--"] {
            if let Some(rest) = text.strip_prefix(marker) {
                text = rest.trim_start();
                break;
            }
        }
        if let Some(rest) = text.strip_suffix("-/") {
            text = rest.trim_end();
        }
        if !text.is_empty() {
            out.push(text.to_string());
        }
    }
    out.join("\n")
}

/// Byte range of `span` inside `source`, clipped to its length.
pub fn slice(source: &str, span: Span) -> String {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len());
    if start > end {
        return String::new();
    }
    source[start..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One node of Lean's syntax tree, built by hand: the mapper is what is under
    /// test here, not the parser (the `klean-ast` crate tests cover that).
    fn node(kind: &str, span: Option<[u32; 2]>, children: Vec<SyntaxNode>) -> SyntaxNode {
        SyntaxNode {
            kind: kind.to_string(),
            span,
            text: None,
            missing: false,
            children,
        }
    }

    fn ident(name: &str, span: [u32; 2]) -> SyntaxNode {
        SyntaxNode {
            kind: "ident".to_string(),
            span: Some(span),
            text: Some(name.to_string()),
            missing: false,
            children: Vec::new(),
        }
    }

    fn file(nodes: Vec<SyntaxNode>) -> FileAst {
        FileAst {
            nodes,
            syntax_errors: 0,
        }
    }

    /// `def foo : Nat := 1`, wrapped the way Lean wraps declarations.
    fn definition(start: u32, end: u32) -> SyntaxNode {
        node(
            "Lean.Parser.Command.declaration",
            Some([start, end]),
            vec![
                node("Lean.Parser.Command.declModifiers", None, vec![]),
                node(
                    "Lean.Parser.Command.definition",
                    Some([start, end]),
                    vec![
                        node("def", Some([start, start + 3]), vec![]),
                        node(
                            "Lean.Parser.Command.declId",
                            Some([start + 4, start + 7]),
                            vec![ident("foo", [start + 4, start + 7])],
                        ),
                        node(
                            "Lean.Parser.Command.optDeclSig",
                            Some([start + 8, start + 13]),
                            vec![],
                        ),
                    ],
                ),
            ],
        )
    }

    const SOURCE: &str = "/-- Doc. -/\ndef foo : Nat := 1\n";

    #[test]
    fn a_declaration_becomes_a_def_row() {
        let parsed = build(SOURCE, &file(vec![definition(12, 30)]));
        assert_eq!(parsed.nodes.len(), 1);
        let node = &parsed.nodes[0];
        assert_eq!(node.kind, "def");
        assert_eq!(node.name.as_deref(), Some("foo"));
        // The row spans the whole command, including the doc comment…
        assert_eq!(node.span.len(), 18);
        // …while the editor rewrites the command without it.
        assert_eq!(slice(SOURCE, node.inner), "def foo : Nat := 1");
    }

    #[test]
    fn doc_comments_lose_their_markers() {
        let with_doc = node(
            "Lean.Parser.Command.declaration",
            Some([0, 31]),
            vec![
                node(
                    "Lean.Parser.Command.declModifiers",
                    Some([0, 11]),
                    vec![node("Lean.Parser.Command.docComment", Some([0, 11]), vec![])],
                ),
                node(
                    "Lean.Parser.Command.definition",
                    Some([13, 31]),
                    vec![node("def", Some([13, 16]), vec![])],
                ),
            ],
        );
        let parsed = build(SOURCE, &file(vec![with_doc]));
        assert_eq!(parsed.nodes[0].doc.as_deref(), Some("Doc."));
    }

    #[test]
    fn blocks_nest_until_their_end() {
        let namespace = node(
            "Lean.Parser.Command.namespace",
            Some([0, 13]),
            vec![
                node("namespace", Some([0, 9]), vec![]),
                ident("N", [10, 11]),
            ],
        );
        let end = node("Lean.Parser.Command.end", Some([32, 35]), vec![]);
        let parsed = build(SOURCE, &file(vec![namespace, definition(0, 30), end]));
        assert_eq!(parsed.nodes.len(), 1);
        let block = &parsed.nodes[0];
        assert_eq!(block.kind, "namespace");
        assert_eq!(block.name.as_deref(), Some("N"));
        assert_eq!(block.children.len(), 1, "the definition is inside");
        assert_eq!(block.children[0].kind, "def");
        // The block now runs to its `end`.
        assert_eq!(block.inner.end, 35);
    }

    #[test]
    fn structure_fields_become_children() {
        let structure = node(
            "Lean.Parser.Command.declaration",
            Some([0, 40]),
            vec![
                node("Lean.Parser.Command.declModifiers", None, vec![]),
                node(
                    "Lean.Parser.Command.structure",
                    Some([0, 40]),
                    vec![
                        node("structure", Some([0, 9]), vec![]),
                        node(
                            "Lean.Parser.Command.declId",
                            Some([10, 11]),
                            vec![ident("P", [10, 11])],
                        ),
                        node(
                            "Lean.Parser.Command.structFields",
                            Some([18, 40]),
                            vec![node(
                                "Lean.Parser.Command.structSimpleBinder",
                                Some([20, 27]),
                                vec![ident("x", [20, 21])],
                            )],
                        ),
                    ],
                ),
            ],
        );
        let parsed = build(SOURCE, &file(vec![structure]));
        let node = &parsed.nodes[0];
        assert_eq!(node.kind, "structure");
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].kind, "field");
        assert_eq!(node.children[0].name.as_deref(), Some("x"));
    }

    #[test]
    fn imports_are_read_from_the_header() {
        let import = node(
            "Lean.Parser.Command.import",
            Some([0, 12]),
            vec![
                node("import", Some([0, 6]), vec![]),
                ident("Init", [7, 11]),
                ident("Data", [7, 11]),
            ],
        );
        let parsed = imports_of(&file(vec![import]));
        // `import Init.Data` plus the implicit default modules.
        assert_eq!(parsed, vec!["Init.Data", "Init", "Lean"]);
    }

    #[test]
    fn line_index_maps_offsets_and_back() {
        let lines = LineIndex::new("a\nbb\nccc");
        assert_eq!(lines.line_of(0), 1);
        assert_eq!(lines.line_of(2), 2);
        assert_eq!(lines.line_of(5), 3);
        assert_eq!(lines.start_of(2), 2);
        assert_eq!(lines.line_count(), 3);
    }

    #[test]
    fn plain_comments_above_a_node_are_kept() {
        let source = "#check Nat\n\n-- a note\ndef foo : Nat := 1\n";
        let start = source.find("def foo").unwrap() as u32;
        let parsed = build(source, &file(vec![definition(start, start + 18)]));
        let node = &parsed.nodes[0];
        assert_eq!(node.comments.as_deref(), Some("a note"));
        let span = node.comment_span.expect("comment span");
        assert_eq!(slice(source, span), "-- a note");
    }

    #[test]
    fn a_blank_line_breaks_the_note() {
        let source = "#check Nat\n\n-- a note\n\ndef foo : Nat := 1\n";
        let start = source.find("def foo").unwrap() as u32;
        let parsed = build(source, &file(vec![definition(start, start + 18)]));
        assert_eq!(parsed.nodes[0].comments, None, "the note belongs to nothing");
    }

    #[test]
    fn block_comments_are_not_notes() {
        let source = "#check Nat\n\n/- a block -/\ndef foo : Nat := 1\n";
        let start = source.find("def foo").unwrap() as u32;
        let parsed = build(source, &file(vec![definition(start, start + 18)]));
        assert_eq!(parsed.nodes[0].comments, None);
    }

    #[test]
    fn a_note_may_span_several_lines() {
        let source = "#check Nat\n\n-- first\n-- second\ndef foo : Nat := 1\n";
        let start = source.find("def foo").unwrap() as u32;
        let parsed = build(source, &file(vec![definition(start, start + 18)]));
        assert_eq!(parsed.nodes[0].comments.as_deref(), Some("first\nsecond"));
    }

    #[test]
    fn module_doc_is_read() {
        let source = "/-!\n  Module doc.\n-/\n\ndef foo : Nat := 1\n";
        let start = source.find("def foo").unwrap() as u32;
        let parsed = build(source, &file(vec![definition(start, start + 18)]));
        assert_eq!(parsed.module_doc.as_deref(), Some("Module doc."));
    }

    #[test]
    fn signature_is_compacted() {
        let source = "def f :\n  Nat := 0\n";
        let command = node(
            "Lean.Parser.Command.declaration",
            Some([0, 18]),
            vec![
                node("Lean.Parser.Command.declModifiers", None, vec![]),
                node(
                    "Lean.Parser.Command.definition",
                    Some([0, 18]),
                    vec![
                        node(
                            "Lean.Parser.Command.declId",
                            Some([4, 5]),
                            vec![ident("f", [4, 5])],
                        ),
                        node("Lean.Parser.Command.optDeclSig", Some([6, 13]), vec![]),
                    ],
                ),
            ],
        );
        let parsed = build(source, &file(vec![command]));
        assert_eq!(parsed.nodes[0].signature.as_deref(), Some(": Nat"));
        assert_eq!(parsed.nodes[0].label(), "def f : Nat");
    }
}
