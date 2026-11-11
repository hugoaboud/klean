//! Fixed graph layout, stored in `.klean.json` inside the project.
//!
//! Every node is pinned: a node is placed once (near the rest of its namespace,
//! without overlapping), the position is written to disk, and from then on it
//! only moves when the user moves it. Nodes are identified by their kind and
//! name, so a position survives edits to the file and disappears with the node.
//!
//! The logic board also lives here: which theories exist, which theory every
//! term belongs to, and the layer dividers of each theory graph.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::error::{LeanError, LeanResult};
use super::fs::ProjectFs;

pub const FILE: &str = ".klean.json";
const PADDING: f64 = 12.0;
/// Bumped when the stored shape changes; older files are migrated on load.
const VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayoutFile {
    #[serde(default)]
    pub version: u32,
    /// Graph key (`ast`, `logic` or `logic:<theory>`) -> node key -> [x, y].
    #[serde(default)]
    pub positions: BTreeMap<String, BTreeMap<String, [f64; 2]>>,
    /// Theories the user created, in creation order.
    #[serde(default)]
    pub theories: Vec<TheoryDef>,
    /// Term key -> theory key. Terms without an entry are unassigned.
    #[serde(default)]
    pub assignments: BTreeMap<String, String>,
    /// Theory key -> its layer dividers, top to bottom.
    #[serde(default)]
    pub layers: BTreeMap<String, Vec<Layer>>,
    /// Graph key -> camera, as `[x, y, zoom]`.
    #[serde(default)]
    pub camera: BTreeMap<String, [f64; 3]>,
    /// Setting id -> value.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Graph key -> link key -> position of the link's middle segment.
    #[serde(default)]
    pub bends: BTreeMap<String, BTreeMap<String, f64>>,
}

/// A theory the user created: a node of the root graph, or of another theory's
/// graph when it has a parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoryDef {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
}

/// A horizontal divider inside a theory graph. It labels the band under it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub y: f64,
}

/// A graph view, remembered so a page looks the same when reopened.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub k: f64,
}

/// What the arranger needs to know about one node.
pub struct Placement<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub label: &'a str,
    pub depth: usize,
    pub parent: Option<&'a str>,
    /// Number of visible children.
    pub children: usize,
}

/// A key-safe form of a user supplied name.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "theory".to_string()
    } else {
        trimmed
    }
}

impl LayoutFile {
    pub fn load(fs: &ProjectFs) -> Self {
        if !fs.exists(FILE) {
            return LayoutFile::default();
        }
        let mut layout: LayoutFile = fs
            .read(FILE)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if layout.version < 4 {
            // Section-based logic coordinates mean nothing in the theory model.
            layout.positions.remove("logic");
            layout.version = 4;
        }
        layout
    }

    pub fn save(&self, fs: &ProjectFs) -> LeanResult<()> {
        let mut copy = self.clone();
        copy.version = VERSION;
        // Keep the stored defaults tidy: a setting that matches its default is
        // not worth writing down.
        let defaults = crate::lean::settings::defaults();
        copy.settings
            .retain(|id, value| !defaults.iter().any(|(key, default)| key == id && value == default));
        let text = serde_json::to_string_pretty(&copy)
            .map_err(|error| LeanError::msg(error.to_string()))?;
        fs.write(FILE, &format!("{text}\n"))
    }

    pub fn camera(&self, graph: &str) -> Option<Camera> {
        self.camera.get(graph).map(|value| Camera {
            x: value[0],
            y: value[1],
            k: value[2],
        })
    }

    pub fn set_camera(&mut self, graph: &str, camera: Camera) {
        self.camera
            .insert(graph.to_string(), [camera.x, camera.y, camera.k]);
    }

    pub fn settings(&self) -> &BTreeMap<String, String> {
        &self.settings
    }

    pub fn set_setting(&mut self, id: &str, value: &str) {
        self.settings.insert(id.to_string(), value.to_string());
    }

    // ---------------------------------------------------------------- theories

    pub fn theories(&self) -> &[TheoryDef] {
        &self.theories
    }

    pub fn theory(&self, key: &str) -> Option<&TheoryDef> {
        self.theories.iter().find(|theory| theory.key == key)
    }

    /// Creates a theory under `parent` (or in the root graph). The key is
    /// derived from the name and kept unique.
    pub fn add_theory(&mut self, name: &str, parent: Option<&str>) -> TheoryDef {
        let base = format!("theory:{}", slug(name));
        let mut key = base.clone();
        let mut suffix = 2;
        while self.theory(&key).is_some() {
            key = format!("{base}-{suffix}");
            suffix += 1;
        }
        let theory = TheoryDef {
            key,
            name: name.trim().to_string(),
            parent: parent.map(str::to_string),
        };
        self.theories.push(theory.clone());
        theory
    }

    /// Renames a theory; its key stays, so positions and assignments survive.
    pub fn rename_theory(&mut self, key: &str, name: &str) {
        if let Some(theory) = self.theories.iter_mut().find(|theory| theory.key == key) {
            theory.name = name.trim().to_string();
        }
    }

    /// Removes a theory. Its terms go back to unassigned and the theories nested
    /// inside it become root theories, so nothing is lost.
    pub fn remove_theory(&mut self, key: &str) {
        self.assignments.retain(|_, theory| theory != key);
        for theory in &mut self.theories {
            if theory.parent.as_deref() == Some(key) {
                theory.parent = None;
            }
        }
        self.theories.retain(|theory| theory.key != key);
        self.layers.remove(key);
    }

    /// Assigns term keys to a theory (or back to unassigned when `None`).
    pub fn assign(&mut self, terms: &[String], theory: Option<&str>) {
        for term in terms {
            match theory {
                Some(theory) => {
                    self.assignments.insert(term.clone(), theory.to_string());
                }
                None => {
                    self.assignments.remove(term);
                }
            }
        }
    }

    pub fn theory_of(&self, term: &str) -> Option<&str> {
        self.assignments.get(term).map(String::as_str)
    }

    // ------------------------------------------------------------------ layers

    pub fn layers(&self, theory: &str) -> Vec<Layer> {
        let mut layers = self.layers.get(theory).cloned().unwrap_or_default();
        layers.sort_by(|left, right| left.y.partial_cmp(&right.y).unwrap_or(std::cmp::Ordering::Equal));
        layers
    }

    pub fn add_layer(&mut self, theory: &str, name: &str, y: f64) -> Layer {
        let layers = self.layers.entry(theory.to_string()).or_default();
        let layer = Layer {
            id: format!("layer:{}-{}", slug(name), layers.len() + 1),
            name: name.trim().to_string(),
            y,
        };
        layers.push(layer.clone());
        layer
    }

    pub fn remove_layer(&mut self, theory: &str, id: &str) {
        if let Some(layers) = self.layers.get_mut(theory) {
            layers.retain(|layer| layer.id != id);
        }
    }

    pub fn rename_layer(&mut self, theory: &str, id: &str, name: &str) {
        if let Some(layers) = self.layers.get_mut(theory) {
            if let Some(layer) = layers.iter_mut().find(|layer| layer.id == id) {
                layer.name = name.trim().to_string();
            }
        }
    }

    pub fn move_layer(&mut self, theory: &str, id: &str, y: f64) {
        if let Some(layers) = self.layers.get_mut(theory) {
            if let Some(layer) = layers.iter_mut().find(|layer| layer.id == id) {
                layer.y = y;
            }
        }
    }

    /// Position of a link's middle segment, if the user moved it.
    pub fn bend(&self, graph: &str, key: &str) -> Option<f64> {
        self.bends.get(graph).and_then(|bends| bends.get(key)).copied()
    }

    pub fn set_bend(&mut self, graph: &str, key: &str, value: Option<f64>) {
        let bends = self.bends.entry(graph.to_string()).or_default();
        match value {
            Some(value) => {
                bends.insert(key.to_string(), value);
            }
            None => {
                bends.remove(key);
            }
        }
    }

    /// Moves every stored position of `keys` by `dx`.
    pub fn shift_x(&mut self, graph: &str, keys: &[&str], dx: f64) {
        if dx == 0.0 {
            return;
        }
        if let Some(positions) = self.positions.get_mut(graph) {
            for key in keys {
                if let Some(position) = positions.get_mut(*key) {
                    position[0] += dx;
                }
            }
        }
    }

    /// Positions every node that has none, drops the ones whose node is gone,
    /// and returns the position of every node in the graph.
    ///
    /// Leaves are walked in tree order and laid out in rows, so a namespace's
    /// declarations end up together; each new leaf spirals out from that spot
    /// until it no longer touches anything already placed. Containers take the
    /// middle of what they hold, so a group's header sits on its content.
    pub fn arrange(
        &mut self,
        graph: &str,
        group_kinds: &[&str],
        nodes: &[Placement<'_>],
    ) -> BTreeMap<String, [f64; 2]> {
        let mut stored = self.positions.remove(graph).unwrap_or_default();
        let live: HashSet<&str> = nodes.iter().map(|node| node.key).collect();
        stored.retain(|key, _| live.contains(key.as_str()));

        let index_of: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.key, index))
            .collect();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        let mut roots: Vec<usize> = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            match node.parent.and_then(|parent| index_of.get(parent).copied()) {
                Some(parent) => children[parent].push(index),
                None => roots.push(index),
            }
        }

        let size = |node: &Placement<'_>| -> (f64, f64) {
            if group_kinds.contains(&node.kind) {
                (60.0_f64.max(node.label.chars().count() as f64 * 5.5 + 24.0), 19.0)
            } else {
                (
                    60.0_f64.max(190.0_f64.min(36.0 + node.label.chars().count() as f64 * 5.9 + 15.0)),
                    21.0,
                )
            }
        };

        let leaves = nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| children[*index].is_empty())
            .count();
        let per_row = (leaves as f64).sqrt().ceil().max(3.0) as usize;

        let mut positions: BTreeMap<String, [f64; 2]> = BTreeMap::new();
        let mut placed: Vec<(f64, f64, f64, f64)> = Vec::new();
        let mut ordinal = 0usize;

        for root in roots {
            place_subtree(
                root,
                0,
                &nodes,
                &children,
                &size,
                per_row,
                &mut ordinal,
                &mut stored,
                &mut positions,
                &mut placed,
            );
        }

        self.positions.insert(graph.to_string(), stored);
        positions
    }

    /// Stores hand-made positions (drag results) for the nodes that own one.
    pub fn store(&mut self, graph: &str, entries: &[(String, f64, f64)]) {
        let positions = self.positions.entry(graph.to_string()).or_default();
        for (key, x, y) in entries {
            positions.insert(key.clone(), [*x, *y]);
        }
    }
}

/// Spiral out from `anchor` until a rectangle of `size` fits without touching.
fn free_spot(anchor: [f64; 2], size: (f64, f64), placed: &[(f64, f64, f64, f64)]) -> [f64; 2] {
    let half = (size.0 / 2.0 + PADDING, size.1 / 2.0 + PADDING);
    let fits = |x: f64, y: f64| {
        placed.iter().all(|&(px, py, pw, ph)| {
            (x - px).abs() >= half.0 + pw / 2.0 || (y - py).abs() >= half.1 + ph / 2.0
        })
    };
    if fits(anchor[0], anchor[1]) {
        return anchor;
    }
    for ring in 1..400 {
        let radius = ring as f64 * 14.0;
        let steps = 8 + ring * 4;
        for step in 0..steps {
            let angle = step as f64 * std::f64::consts::TAU / steps as f64;
            let x = anchor[0] + radius * angle.cos();
            let y = anchor[1] + radius * angle.sin() * 0.75;
            if fits(x, y) {
                return [x, y];
            }
        }
    }
    [anchor[0] + 40.0, anchor[1] + 40.0]
}

#[allow(clippy::too_many_arguments)]
fn place_subtree(
    index: usize,
    depth: usize,
    nodes: &[Placement<'_>],
    children: &[Vec<usize>],
    size: &impl Fn(&Placement<'_>) -> (f64, f64),
    per_row: usize,
    ordinal: &mut usize,
    stored: &mut BTreeMap<String, [f64; 2]>,
    positions: &mut BTreeMap<String, [f64; 2]>,
    placed: &mut Vec<(f64, f64, f64, f64)>,
) -> [f64; 2] {
    let node = &nodes[index];
    // A parent that already has a place is the best anchor for a new node: it
    // appears next to the group it belongs to instead of in a global grid row.
    let parent_point = node.parent.and_then(|parent| positions.get(parent).copied());

    if children[index].is_empty() {
        let column = *ordinal % per_row;
        let row = *ordinal / per_row;
        *ordinal += 1;
        let anchor = match parent_point {
            Some(point) => [point[0] + 16.0, point[1] + 44.0],
            None => [column as f64 * 210.0, depth as f64 * 96.0 + row as f64 * 84.0],
        };
        let point = match stored.get(node.key).copied() {
            Some(saved) => saved,
            None => {
                let spot = free_spot(anchor, size(node), placed);
                stored.insert(node.key.to_string(), spot);
                spot
            }
        };
        let (width, height) = size(node);
        placed.push((point[0], point[1], width, height));
        positions.insert(node.key.to_string(), point);
        return point;
    }

    let mut sum = [0.0_f64, 0.0_f64];
    let mut count = 0.0_f64;
    for &child in &children[index] {
        let point = place_subtree(
            child, depth + 1, nodes, children, size, per_row, ordinal, stored, positions, placed,
        );
        sum[0] += point[0];
        sum[1] += point[1];
        count += 1.0;
    }
    // A container keeps the place it was given — dragging a group must stick —
    // and otherwise settles on the middle of what it holds, once.
    let point = match stored.get(node.key).copied() {
        Some(saved) => saved,
        None => {
            let centre = [sum[0] / count, sum[1] / count];
            stored.insert(node.key.to_string(), centre);
            centre
        }
    };
    positions.insert(node.key.to_string(), point);
    let (width, height) = size(node);
    placed.push((point[0], point[1], width, height));
    point
}
