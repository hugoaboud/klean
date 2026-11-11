//! Re-prints a parsed project as human-readable Lean.
//!
//! The output keeps every expression and proof exactly as written and adds
//! comments that outline the file: a banner per file, a banner per
//! `namespace`/`section`, the original doc comments, and one-line descriptions
//! for composite containers. It is meant for reading and reviewing a project,
//! not for replacing its sources.

use super::syntax::{NodeKind, ParsedFile, SyntaxNode};

/// Renders a whole file, including a banner and the leading module doc comment.
pub fn print_file(rel: &str, source: &str, parsed: &ParsedFile) -> String {
    let mut out = String::new();
    let rule = "─".repeat(58usize.saturating_sub(rel.chars().count()).max(6));
    out.push_str(&format!("-- ─── {rel} {rule}\n"));
    let summary = describe_file(parsed);
    if !summary.is_empty() {
        out.push_str(&format!("-- {summary}\n"));
    }
    out.push('\n');
    if let Some(module_doc) = &parsed.module_doc {
        out.push_str(&block_comment(module_doc, ""));
        out.push('\n');
    }
    for node in &parsed.nodes {
        render(&mut out, node, source, 0);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn describe_file(parsed: &ParsedFile) -> String {
    let mut parts = Vec::new();
    let imports = parsed.count_kind(NodeKind::Import);
    if imports > 0 {
        parts.push(format!("{imports} import{}", plural(imports)));
    }
    let decls = parsed
        .nodes
        .iter()
        .filter(|node| node.kind.is_declaration())
        .count();
    if decls > 0 {
        parts.push(format!("{decls} top-level declaration{}", plural(decls)));
    }
    let containers = parsed
        .nodes
        .iter()
        .filter(|node| node.kind.is_block())
        .count();
    if containers > 0 {
        parts.push(format!("{containers} section{}", plural(containers)));
    }
    parts.join(" · ")
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn render(out: &mut String, node: &SyntaxNode, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);

    match &node.doc {
        Some(doc) => {
            out.push_str(&block_comment(doc, &indent));
            out.push('\n');
        }
        None => {
            if let Some(description) = describe_node(node) {
                out.push_str(&format!("{indent}-- {description}\n"));
            }
        }
    }

    if node.kind.is_block() {
        let header = node
            .header
            .map(|span| slice(source, span))
            .unwrap_or_else(|| node.kind.label().to_string())
            .trim()
            .to_string();
        out.push_str(&format!("{indent}-- ══ {} ══\n", header.replace('\n', " ")));
        let base = node
            .header
            .map(|span| column_of(source, span.start))
            .unwrap_or(0);
        out.push_str(&reindent(&header, &indent, base));
        out.push('\n');
        for child in &node.children {
            render(out, child, source, depth + 1);
        }
        out.push_str(&format!("{indent}end"));
        if let Some(name) = node.name.as_deref() {
            out.push(' ');
            out.push_str(name);
        }
        out.push('\n');
    } else {
        let text = slice(source, node.span);
        let base = column_of(source, node.span.start);
        out.push_str(&reindent(&text, &indent, base));
        out.push('\n');
    }

    if depth == 0 {
        out.push('\n');
    }
}

/// One-line description for declarations that have no doc comment. Nodes whose
/// header already says everything (a plain `def`) get no comment at all.
fn describe_node(node: &SyntaxNode) -> Option<String> {
    let name = node.name.as_deref();
    match node.kind {
        NodeKind::Structure | NodeKind::Class => {
            let fields = node
                .children
                .iter()
                .filter_map(|child| child.name.clone())
                .collect::<Vec<_>>();
            Some(match name {
                Some(name) if !fields.is_empty() => {
                    format!("{} {name} · {} fields: {}", node.kind.label(), fields.len(), fields.join(", "))
                }
                Some(name) => format!("{} {name}", node.kind.label()),
                None => format!("{} with {} fields", node.kind.label(), fields.len()),
            })
        }
        NodeKind::Inductive => {
            let ctors = node
                .children
                .iter()
                .filter_map(|child| child.name.clone())
                .collect::<Vec<_>>();
            Some(match name {
                Some(name) if !ctors.is_empty() => {
                    format!("inductive {name} · {} constructors: {}", ctors.len(), ctors.join(", "))
                }
                Some(name) => format!("inductive {name}"),
                None => "inductive type".to_string(),
            })
        }
        kind if kind.has_where_block() && !node.children.is_empty() => {
            let locals = node
                .children
                .iter()
                .filter_map(|child| child.name.clone())
                .collect::<Vec<_>>();
            Some(format!(
                "{} {} · {} local declaration{}: {}",
                kind.label(),
                name.unwrap_or("(anonymous)"),
                locals.len(),
                plural(locals.len()),
                locals.join(", ")
            ))
        }
        NodeKind::Mutual => Some(format!("mutual block · {} declarations", node.children.len())),
        _ => None,
    }
}

/// Wraps free text into a Lean block comment with the given indentation.
pub fn block_comment(text: &str, indent: &str) -> String {
    let mut out = String::new();
    out.push_str(indent);
    out.push_str("/-");
    for (idx, line) in text.lines().enumerate() {
        if idx == 0 && !line.trim().is_empty() {
            out.push(' ');
            out.push_str(line.trim_end());
            out.push('\n');
        } else {
            out.push_str(indent);
            out.push_str("  ");
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str("-/");
    out
}

fn slice(source: &str, span: super::syntax::Span) -> String {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len());
    source[start..end].to_string()
}

/// Indents a block of text. The first line starts at the node's own token, so
/// `base` is the source column of that token: subsequent lines are shifted left
/// by the same amount, which preserves their indentation relative to the node
/// head instead of flattening it.
fn reindent(text: &str, indent: &str, base: usize) -> String {
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if line.trim().is_empty() {
            continue;
        }
        if index == 0 {
            out.push_str(indent);
            out.push_str(line.trim_end());
            continue;
        }
        let lead = line.len() - line.trim_start().len();
        let cut = lead.min(base);
        out.push_str(indent);
        out.push_str(&line[cut..].trim_end());
    }
    out
}

/// Byte column of a source offset within its line.
fn column_of(source: &str, offset: u32) -> usize {
    let end = (offset as usize).min(source.len());
    let line_start = source[..end].rfind('\n').map(|index| index + 1).unwrap_or(0);
    end - line_start
}

/// Banner used between libraries in the project outline.
pub fn lib_banner(name: &str, files: usize, nodes: usize) -> String {
    format!(
        "-- ═══ LIB · {name} · {files} file{} · {nodes} nodes ═══\n",
        plural(files)
    )
}
