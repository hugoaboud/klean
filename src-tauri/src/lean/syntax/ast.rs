//! Structural syntax tree for a Lean file.
//!
//! This is a *structural* tree, not an elaborated Lean `Syntax` tree: it
//! groups a file into commands, and containers (`namespace`, `section`,
//! `mutual`, `structure` fields, `inductive` constructors, `where` blocks)
//! into nesting levels. Expressions and proofs are kept verbatim as text so
//! that edits can be spliced back into the file without rewriting anything.

use serde::Serialize;

/// Byte range inside a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
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
}

/// Kind of a structural node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// File-level grouping, produced by the project layer.
    File,
    Prelude,
    Import,
    Namespace,
    Section,
    Mutual,
    Def,
    Theorem,
    Lemma,
    Axiom,
    Structure,
    Class,
    Inductive,
    Instance,
    Abbrev,
    Opaque,
    Example,
    Syntax,
    Macro,
    MacroRules,
    Notation,
    Elab,
    Attribute,
    Open,
    Export,
    Variable,
    Universe,
    Option,
    Deriving,
    Initialize,
    Field,
    Constructor,
    WhereDecl,
    /// Any command the parser does not classify further.
    Command,
}

impl NodeKind {
    /// Short keyword-style label used in the UI and in printed output.
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Prelude => "prelude",
            NodeKind::Import => "import",
            NodeKind::Namespace => "namespace",
            NodeKind::Section => "section",
            NodeKind::Mutual => "mutual",
            NodeKind::Def => "def",
            NodeKind::Theorem => "theorem",
            NodeKind::Lemma => "lemma",
            NodeKind::Axiom => "axiom",
            NodeKind::Structure => "structure",
            NodeKind::Class => "class",
            NodeKind::Inductive => "inductive",
            NodeKind::Instance => "instance",
            NodeKind::Abbrev => "abbrev",
            NodeKind::Opaque => "opaque",
            NodeKind::Example => "example",
            NodeKind::Syntax => "syntax",
            NodeKind::Macro => "macro",
            NodeKind::MacroRules => "macro_rules",
            NodeKind::Notation => "notation",
            NodeKind::Elab => "elab",
            NodeKind::Attribute => "attribute",
            NodeKind::Open => "open",
            NodeKind::Export => "export",
            NodeKind::Variable => "variable",
            NodeKind::Universe => "universe",
            NodeKind::Option => "set_option",
            NodeKind::Deriving => "deriving",
            NodeKind::Initialize => "initialize",
            NodeKind::Field => "field",
            NodeKind::Constructor => "constructor",
            NodeKind::WhereDecl => "where",
            NodeKind::Command => "command",
        }
    }

    /// True for nodes that open an `end`-terminated block.
    pub fn is_block(self) -> bool {
        matches!(self, NodeKind::Namespace | NodeKind::Section | NodeKind::Mutual)
    }

    /// True for nodes whose body is a block of sub-nodes.
    pub fn is_container(self) -> bool {
        matches!(
            self,
            NodeKind::Namespace
                | NodeKind::Section
                | NodeKind::Mutual
                | NodeKind::Structure
                | NodeKind::Class
                | NodeKind::Inductive
                | NodeKind::File
        )
    }

    /// True for declarations that may carry a `where` block.
    pub fn has_where_block(self) -> bool {
        matches!(
            self,
            NodeKind::Def
                | NodeKind::Theorem
                | NodeKind::Lemma
                | NodeKind::Abbrev
                | NodeKind::Instance
                | NodeKind::Opaque
                | NodeKind::Example
        )
    }

    /// True when the node represents a declaration worth describing.
    pub fn is_declaration(self) -> bool {
        matches!(
            self,
            NodeKind::Def
                | NodeKind::Theorem
                | NodeKind::Lemma
                | NodeKind::Axiom
                | NodeKind::Structure
                | NodeKind::Class
                | NodeKind::Inductive
                | NodeKind::Instance
                | NodeKind::Abbrev
                | NodeKind::Opaque
                | NodeKind::Example
                | NodeKind::Syntax
                | NodeKind::Macro
                | NodeKind::MacroRules
                | NodeKind::Notation
                | NodeKind::Elab
        )
    }

    pub fn from_keyword(keyword: &str) -> Option<NodeKind> {
        Some(match keyword {
            "prelude" => NodeKind::Prelude,
            "import" => NodeKind::Import,
            "namespace" => NodeKind::Namespace,
            "section" => NodeKind::Section,
            "mutual" => NodeKind::Mutual,
            "def" => NodeKind::Def,
            "theorem" => NodeKind::Theorem,
            "lemma" => NodeKind::Lemma,
            "axiom" | "constant" => NodeKind::Axiom,
            "structure" => NodeKind::Structure,
            "class" => NodeKind::Class,
            "inductive" => NodeKind::Inductive,
            "instance" => NodeKind::Instance,
            "abbrev" => NodeKind::Abbrev,
            "opaque" => NodeKind::Opaque,
            "example" => NodeKind::Example,
            "syntax" | "notation3" => NodeKind::Syntax,
            "macro" => NodeKind::Macro,
            "macro_rules" => NodeKind::MacroRules,
            "notation" | "infix" | "infixl" | "infixr" | "prefix" | "postfix" => {
                NodeKind::Notation
            }
            "elab" | "elab_rules" => NodeKind::Elab,
            "attribute" => NodeKind::Attribute,
            "open" => NodeKind::Open,
            "export" => NodeKind::Export,
            "variable" => NodeKind::Variable,
            "universe" | "universes" => NodeKind::Universe,
            "set_option" => NodeKind::Option,
            "deriving" => NodeKind::Deriving,
            "initialize" => NodeKind::Initialize,
            _ => return None,
        })
    }
}

/// A node of the structural syntax tree.
#[derive(Debug, Clone)]
pub struct SyntaxNode {
    pub kind: NodeKind,
    /// Declared name when the command has one (`def foo`, `namespace Bar`, ...).
    pub name: Option<String>,
    /// Span of the whole command, from its first modifier/attribute to the end
    /// of its last significant token.
    pub span: Span,
    /// Span of the declared name token.
    pub name_span: Option<Span>,
    /// Signature part: modifiers, attributes, keyword, name and type.
    pub header: Option<Span>,
    /// Body part, after `:=`, `where` or the first `|` clause.
    pub body: Option<Span>,
    /// Doc comment text, already stripped of the `/--` and `-/` markers.
    pub doc: Option<String>,
    pub doc_span: Option<Span>,
    /// Plain (`--`, `/- -/`) comments directly above the node, verbatim.
    pub comments: Option<String>,
    pub comment_span: Option<Span>,
    /// Compact one-line signature for tree labels.
    pub signature: Option<String>,
    pub children: Vec<SyntaxNode>,
}

impl SyntaxNode {
    /// Human readable label such as `def MerkleRoot : Type`.
    pub fn label(&self) -> String {
        match (self.name.as_deref(), self.signature.as_deref()) {
            (Some(name), Some(sig)) if !sig.is_empty() => {
                format!("{} {} {}", self.kind.label(), name, sig)
            }
            (Some(name), _) => format!("{} {}", self.kind.label(), name),
            (None, Some(sig)) if !sig.is_empty() => format!("{} {}", self.kind.label(), sig),
            (None, _) => self.kind.label().to_string(),
        }
    }

    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a SyntaxNode)) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// 1-based line number.
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// Result of parsing one file.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub nodes: Vec<SyntaxNode>,
    pub diagnostics: Vec<Diagnostic>,
    /// Module doc comment (`/-! ... -/`) at the top of the file, if any.
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

    pub fn count_kind(&self, kind: NodeKind) -> usize {
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
    pub fn get(&self, path: &[usize]) -> Option<&SyntaxNode> {
        let (first, rest) = path.split_first()?;
        let mut node = self.nodes.get(*first)?;
        for idx in rest {
            node = node.children.get(*idx)?;
        }
        Some(node)
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

    pub fn line_of(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(idx) => idx as u32 + 1,
            Err(idx) => idx as u32,
        }
    }
}
