//! The logic board.
//!
//! The board is a stack of graphs. On top sits the root graph, made of the
//! theories the user created; opening one of them with a double click shows the
//! graph of that theory — its terms, the theories nested inside it and the layer
//! dividers that order them. Nothing is computed or packed here: a node sits
//! where the user left it, and the only geometry derived from the layout is the
//! rectangle drawn around a theory graph and the layer a node falls into.

use std::collections::{BTreeMap, HashMap};

use super::dto::{
    GraphExternal, GraphLayer, GraphLink, GraphNode, GraphTarget, LogicGraph, TheoryRef, TreeNode,
};
use super::layout::LayoutFile;

/// Theory key of the pseudo theory that holds everything not assigned yet.
pub const UNASSIGNED: &str = "theory:unassigned";

/// A declaration that can live on the logic board.
pub struct Term {
    pub id: String,
    pub key: String,
    pub label: String,
    pub qual: String,
    pub kind: &'static str,
    pub status: Option<String>,
    pub path: String,
    pub line: u32,
    pub signature: Option<String>,
    /// Namespace chain, outermost first, as fully qualified names.
    pub namespaces: Vec<String>,
    /// Span of the declared name inside `text`, so it is not counted as a use.
    pub own_name: Option<(u32, u32)>,
    /// The declaration as written, for name resolution.
    pub text: String,
}

/// Every term of the project plus the usage links between them.
#[derive(Default)]
pub struct Catalog {
    pub terms: Vec<Term>,
    /// `(source, target)` indices into `terms`.
    pub uses: Vec<(usize, usize)>,
}

impl Catalog {
    pub fn index_by_id(&self) -> HashMap<&str, usize> {
        self.terms
            .iter()
            .enumerate()
            .map(|(index, term)| (term.id.as_str(), index))
            .collect()
    }
}

/// Positions and the camera are kept per graph: the root graph and every theory
/// graph remember their own.
pub fn graph_key(theory: Option<&str>) -> String {
    match theory {
        None => "logic".to_string(),
        Some(key) => format!("logic:{key}"),
    }
}

/// Namespace of a fully qualified name; empty when it has none.
fn namespace_of(qual: &str) -> String {
    match qual.rsplit_once('.') {
        Some((namespace, _)) => namespace.to_string(),
        None => String::new(),
    }
}

fn stored_point(layout: &LayoutFile, graph: &str, key: &str) -> (f64, f64) {
    layout
        .positions
        .get(graph)
        .and_then(|positions| positions.get(key))
        .map(|point| (point[0], point[1]))
        .unwrap_or((0.0, 0.0))
}

fn theory_node(layout: &LayoutFile, graph: &str, theory: &super::layout::TheoryDef, terms: usize) -> GraphNode {
    let key = theory.key.clone();
    let (x, y) = stored_point(layout, graph, &key);
    GraphNode {
        id: key.clone(),
        key,
        label: theory.name.clone(),
        kind: "theory".to_string(),
        parent: None,
        child_count: terms,
        expanded: false,
        path: None,
        detail: Some(format!("{terms} term{}", if terms == 1 { "" } else { "s" })),
        x,
        y,
        status: None,
    }
}

fn term_node(layout: &LayoutFile, graph: &str, term: &Term) -> GraphNode {
    let (x, y) = stored_point(layout, graph, &term.key);
    GraphNode {
        id: term.id.clone(),
        key: term.key.clone(),
        label: term.label.clone(),
        kind: term.kind.to_string(),
        parent: None,
        child_count: 0,
        expanded: false,
        path: Some(term.path.clone()),
        detail: term.signature.clone(),
        x,
        y,
        status: term.status.clone(),
    }
}

fn terms_of<'a>(catalog: &'a Catalog, layout: &LayoutFile, theory: &str) -> Vec<&'a Term> {
    catalog
        .terms
        .iter()
        .filter(|term| layout.theory_of(&term.key) == Some(theory))
        .collect()
}

/// Terms that belong to no theory yet.
fn unassigned<'a>(catalog: &'a Catalog, layout: &LayoutFile) -> Vec<&'a Term> {
    catalog
        .terms
        .iter()
        .filter(|term| layout.theory_of(&term.key).is_none())
        .collect()
}

fn child_theories<'a>(
    layout: &'a LayoutFile,
    parent: Option<&str>,
) -> Vec<&'a super::layout::TheoryDef> {
    layout
        .theories()
        .iter()
        .filter(|theory| theory.parent.as_deref() == parent)
        .collect()
}

/// The chain of theories from a root down to `key`, as the breadcrumbs show it.
pub fn trail(layout: &LayoutFile, open: Option<&str>) -> Vec<TheoryRef> {
    let mut steps = vec![TheoryRef {
        key: String::new(),
        name: "Logic".to_string(),
    }];
    let Some(target) = open.and_then(|key| layout.theory(key)) else {
        return steps;
    };
    let mut chain = vec![target.clone()];
    let mut current = target.parent.clone();
    // A cycle would be a bug, but walking forever would be worse.
    while let Some(parent) = current {
        if chain.len() > 32 {
            break;
        }
        match layout.theory(&parent) {
            Some(theory) => {
                chain.push(theory.clone());
                current = theory.parent.clone();
            }
            None => break,
        }
    }
    chain.reverse();
    steps.extend(chain.into_iter().map(|theory| TheoryRef {
        key: theory.key,
        name: theory.name,
    }));
    steps
}

/// Builds the graph of `open` (the root graph when it is `None`).
///
/// Coordinates come from the layout file; the caller seeds the ones that are
/// missing, then asks for [`links`] once the positions are final.
pub fn build(catalog: &Catalog, layout: &LayoutFile, open: Option<&str>) -> LogicGraph {
    let open = open.filter(|key| layout.theory(key).is_some());
    let graph = graph_key(open);
    let mut nodes: Vec<GraphNode> = Vec::new();

    match open {
        None => {
            for theory in child_theories(layout, None) {
                let count = terms_of(catalog, layout, &theory.key).len();
                nodes.push(theory_node(layout, &graph, theory, count));
            }
        }
        Some(key) => {
            for theory in child_theories(layout, Some(key)) {
                let count = terms_of(catalog, layout, &theory.key).len();
                nodes.push(theory_node(layout, &graph, theory, count));
            }
            for term in terms_of(catalog, layout, key) {
                nodes.push(term_node(layout, &graph, term));
            }
        }
    }

    LogicGraph {
        trail: trail(layout, open),
        nodes,
        links: Vec::new(),
        layers: open
            .map(|key| {
                layout
                    .layers(key)
                    .into_iter()
                    .map(|layer| GraphLayer {
                        id: layer.id,
                        name: layer.name,
                        y: layer.y,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        external: Vec::new(),
        tree: tree(catalog, layout),
        graph,
        camera: None,
    }
}

/// Resolves which terms use which, by name: the full name first, then the last
/// component. KLEAN does not elaborate, so shadowed or ambiguous names can add a
/// use — a documented heuristic.
pub fn resolve_uses(catalog: &mut Catalog) {
    use std::collections::HashMap;

    let mut by_short: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_full: HashMap<String, usize> = HashMap::new();
    for (index, term) in catalog.terms.iter().enumerate() {
        by_short
            .entry(term.label.clone())
            .or_default()
            .push(index);
        by_full.entry(term.qual.clone()).or_insert(index);
    }

    let mut uses = Vec::new();
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for (index, term) in catalog.terms.iter().enumerate() {
        for token in super::syntax::lexer::tokenize(&term.text) {
            if token.kind != super::syntax::lexer::TokenKind::Ident {
                continue;
            }
            if let Some((start, end)) = term.own_name {
                if token.start == start && token.end == end {
                    continue;
                }
            }
            let raw = token.text(&term.text);
            let short = raw.rsplit('.').next().unwrap_or(raw);
            if short.chars().count() < 2 {
                continue;
            }
            let mut targets: Vec<usize> = Vec::new();
            if let Some(&target) = by_full.get(raw) {
                targets.push(target);
            } else if let Some(list) = by_short.get(short) {
                targets.extend(list.iter().copied());
            }
            for target in targets {
                if target == index || !seen.insert((index, target)) {
                    continue;
                }
                uses.push((index, target));
            }
        }
    }
    catalog.uses = uses;
}

/// Usage links of the graph in `view`, already filtered by the layer rule, plus
/// the references that leave the theory graph.
pub fn links(
    catalog: &Catalog,
    layout: &LayoutFile,
    open: Option<&str>,
    nodes: &[GraphNode],
) -> (Vec<GraphLink>, Vec<GraphExternal>) {
    let open = open.filter(|key| layout.theory(key).is_some());
    let graph = graph_key(open);
    let term_index: HashMap<&str, usize> = catalog.index_by_id();
    let node_of: HashMap<&str, &GraphNode> = nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut out: Vec<GraphLink> = Vec::new();
    let mut external: Vec<GraphExternal> = Vec::new();

    match open {
        None => {
            // One link per pair of theories that share at least one use.
            let theory_of_term = |index: usize| -> Option<&str> {
                layout.theory_of(&catalog.terms[index].key)
            };
            let mut pairs: Vec<(String, String)> = Vec::new();
            let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
            for (source, target) in &catalog.uses {
                let (Some(left), Some(right)) = (theory_of_term(*source), theory_of_term(*target))
                else {
                    continue;
                };
                if left == right {
                    continue;
                }
                if !layout.theory(left).is_some_and(|theory| theory.parent.is_none())
                    || !layout.theory(right).is_some_and(|theory| theory.parent.is_none())
                {
                    continue;
                }
                let pair = if left < right {
                    (left.to_string(), right.to_string())
                } else {
                    (right.to_string(), left.to_string())
                };
                if seen.insert(pair.clone(), 0).is_none() {
                    pairs.push(pair);
                }
            }
            for (left, right) in pairs {
                let Some(source) = node_of.get(left.as_str()) else {
                    continue;
                };
                let Some(target) = node_of.get(right.as_str()) else {
                    continue;
                };
                out.push(GraphLink {
                    source: source.id.clone(),
                    target: target.id.clone(),
                    relation: None,
                    bend: layout.bend(&graph, &format!("{}->{}", source.id, target.id)),
                    structural: true,
                });
            }
        }
        Some(key) => {
            let layers = layout.layers(key);
            let _ = &layers;
            // A term inside a nested theory is represented by that theory's node,
            // so a reference to it is drawn as a normal link, not as a stub.
            let nested: HashMap<&str, &str> = nested_owners(layout, key);
            let left_theory = |term: &Term| layout.theory_of(&term.key);
            let right_theory = |term: &Term| layout.theory_of(&term.key);
            let centre = {
                let mut low = f64::INFINITY;
                let mut high = f64::NEG_INFINITY;
                for node in nodes {
                    low = low.min(node.x);
                    high = high.max(node.x);
                }
                if low.is_finite() {
                    (low + high) / 2.0
                } else {
                    0.0
                }
            };
            // A term is drawn as its own node, or as the node of the nested
            // theory that holds it - references into a sub-theory attach there.
            let node_for = |term: &Term| -> Option<&GraphNode> {
                if let Some(node) = node_of.get(term.id.as_str()) {
                    return Some(*node);
                }
                layout
                    .theory_of(&term.key)
                    .and_then(|theory| nested.get(theory).copied())
                    .and_then(|owner| node_of.get(owner).copied())
            };

            for (source, target) in &catalog.uses {
                let left = &catalog.terms[*source];
                let right = &catalog.terms[*target];
                let from = node_for(left);
                let to = node_for(right);
                match (from, to) {
                    (Some(a), Some(b)) => {
                        if a.id == b.id {
                            continue;
                        }
                        // Every internal link is sent: which ones are drawn
                        // depends on the layers, and the board re-decides that
                        // on every frame while a node is dragged.
                        out.push(GraphLink {
                            source: a.id.clone(),
                            target: b.id.clone(),
                            relation: Some(right.kind.to_string()),
                            bend: layout.bend(&graph, &format!("{}->{}", a.id, b.id)),
                            structural: a.kind == "theory" || b.kind == "theory",
                        });
                    }
                    (Some(_), None) => {
                        let Some(a) = node_of.get(left.id.as_str()).copied() else {
                            // The stub belongs to a nested theory node; it is
                            // drawn there when that theory is opened.
                            continue;
                        };
                        if left_theory(left) == right_theory(right) {
                            continue;
                        }
                        let entry = match external
                            .iter_mut()
                            .find(|stub: &&mut GraphExternal| stub.source == a.id)
                        {
                            Some(stub) => stub,
                            None => {
                                external.push(GraphExternal {
                                    source: a.id.clone(),
                                    side: if a.x < centre { "left" } else { "right" }
                                        .to_string(),
                                    targets: Vec::new(),
                                });
                                external.last_mut().expect("just pushed")
                            }
                        };
                        if !entry.targets.iter().any(|target| target.label == right.label) {
                            entry.targets.push(GraphTarget {
                                label: right.label.clone(),
                                kind: right.kind.to_string(),
                                namespace: namespace_of(&right.qual),
                            });
                        }
                    }
                    _ => {}
                }
            }
            let _ = term_index;
        }
    }
    (out, external)
}

/// Terms of the theories nested in `open`, mapped to the node that stands for
/// them in this graph: the direct child theory holding them.
fn nested_owners<'a>(layout: &'a LayoutFile, open: &str) -> HashMap<&'a str, &'a str> {
    let mut owners: HashMap<&str, &str> = HashMap::new();
    for child in layout
        .theories()
        .iter()
        .filter(|theory| theory.parent.as_deref() == Some(open))
    {
        owners.insert(child.key.as_str(), child.key.as_str());
        // Anything nested deeper belongs to the same visible node.
        let mut frontier: Vec<&str> = vec![child.key.as_str()];
        while let Some(current) = frontier.pop() {
            for theory in layout
                .theories()
                .iter()
                .filter(|theory| theory.parent.as_deref() == Some(current))
            {
                owners.insert(theory.key.as_str(), child.key.as_str());
                frontier.push(theory.key.as_str());
            }
        }
    }
    owners
}

/// The logic tree: one root per theory, its namespaces and terms inside, plus the
/// unassigned pile at the end.
pub fn tree(catalog: &Catalog, layout: &LayoutFile) -> Vec<TreeNode> {
    let mut rows: Vec<TreeNode> = Vec::new();
    let mut push_theory = |theory: &super::layout::TheoryDef| {
        let terms = terms_of(catalog, layout, &theory.key);
        let mut children = namespace_tree(&terms, &theory.key);
        children.extend(
            child_theories(layout, Some(&theory.key))
                .into_iter()
                .map(|child| theory_row(catalog, layout, child))
                .collect::<Vec<_>>(),
        );
        let count = terms.len();
        rows.push(TreeNode {
            id: theory.key.clone(),
            label: theory.name.clone(),
            qual: String::new(),
            kind: "theory".to_string(),
            detail: Some(format!("{count} term{}", if count == 1 { "" } else { "s" })),
            doc: None,
            path: None,
            line: None,
            status: None,
            child_count: children.len(),
            children,
        });
    };

    for theory in child_theories(layout, None) {
        push_theory(theory);
    }

    let loose = unassigned(catalog, layout);
    rows.push(TreeNode {
        id: UNASSIGNED.to_string(),
        label: "Unassigned".to_string(),
        qual: String::new(),
        kind: "unassigned".to_string(),
        detail: Some(format!("{} terms", loose.len())),
        doc: None,
        path: None,
        line: None,
        status: None,
        child_count: loose.len(),
        children: namespace_tree(&loose, UNASSIGNED),
    });
    rows
}

fn theory_row(
    catalog: &Catalog,
    layout: &LayoutFile,
    theory: &super::layout::TheoryDef,
) -> TreeNode {
    let terms = terms_of(catalog, layout, &theory.key);
    let count = terms.len();
    let mut children = namespace_tree(&terms, &theory.key);
    children.extend(
        child_theories(layout, Some(&theory.key))
            .into_iter()
            .map(|child| theory_row(catalog, layout, child))
            .collect::<Vec<_>>(),
    );
    TreeNode {
        id: theory.key.clone(),
        label: theory.name.clone(),
        qual: String::new(),
        kind: "theory".to_string(),
        detail: Some(format!("{count} term{}", if count == 1 { "" } else { "s" })),
        doc: None,
        path: None,
        line: None,
        status: None,
        child_count: children.len(),
        children,
    }
}

/// Groups a theory's terms into their namespaces, so the tree reads like Lean.
fn namespace_tree(terms: &[&Term], theory: &str) -> Vec<TreeNode> {
    #[derive(Default)]
    struct Group {
        children: BTreeMap<String, Group>,
        terms: Vec<usize>,
    }

    let mut root = Group::default();
    for (index, term) in terms.iter().enumerate() {
        let mut node = &mut root;
        for namespace in &term.namespaces {
            node = node.children.entry(namespace.clone()).or_default();
        }
        node.terms.push(index);
    }

    fn render(group: &Group, path: &str, theory: &str, terms: &[&Term]) -> Vec<TreeNode> {
        let mut out: Vec<TreeNode> = Vec::new();
        for (name, child) in &group.children {
            let full = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            let children = render(child, &full, theory, terms);
            let count = count_terms(child);
            out.push(TreeNode {
                id: format!("logicns:{theory}:{full}"),
                label: name.clone(),
                qual: full.clone(),
                kind: "namespace".to_string(),
                detail: Some(format!("{count} terms")),
                doc: None,
                path: None,
                line: None,
                status: None,
                child_count: children.len(),
                children,
            });
        }
        for index in &group.terms {
            let term = terms[*index];
            out.push(TreeNode {
                id: term.id.clone(),
                label: term.label.clone(),
                qual: term.qual.clone(),
                kind: term.kind.to_string(),
                detail: term.signature.clone(),
                doc: None,
                path: Some(term.path.clone()),
                line: (term.line > 0).then_some(term.line),
                status: term.status.clone(),
                child_count: 0,
                children: Vec::new(),
            });
        }
        out
    }

    fn count_terms(group: &Group) -> usize {
        group.terms.len() + group.children.values().map(count_terms).sum::<usize>()
    }

    render(&root, "", theory, terms)
}

/// Every term key the tree row `id` stands for: the term itself, or everything
/// inside a namespace or theory row.
pub fn terms_under(catalog: &Catalog, layout: &LayoutFile, id: &str) -> Vec<String> {
    fn collect(row: &TreeNode, keys: &mut Vec<String>) {
        if row.id.starts_with("ast:") {
            keys.push(row.id.clone());
        }
        for child in &row.children {
            collect(child, keys);
        }
    }

    let mut ids: Vec<String> = Vec::new();
    for row in tree(catalog, layout) {
        if row.id == id {
            collect(&row, &mut ids);
            break;
        }
        let mut found = false;
        walk(&row, id, &mut ids, &mut found);
        if found {
            break;
        }
    }

    // Rows carry the AST id; the layout is keyed by term key.
    ids.iter()
        .filter_map(|ast| {
            catalog
                .terms
                .iter()
                .find(|term| &term.id == ast)
                .map(|term| term.key.clone())
        })
        .collect()
}

fn walk(row: &TreeNode, id: &str, keys: &mut Vec<String>, found: &mut bool) {
    for child in &row.children {
        if child.id == id {
            fn collect(row: &TreeNode, keys: &mut Vec<String>) {
                if row.id.starts_with("ast:") {
                    keys.push(row.id.clone());
                }
                for child in &row.children {
                    collect(child, keys);
                }
            }
            collect(child, keys);
            *found = true;
            return;
        }
        walk(child, id, keys, found);
        if *found {
            return;
        }
    }
}
