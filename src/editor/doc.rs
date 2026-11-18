//! The map file being edited: the OSM data, where its nodes are on the map, which ways use
//! each node, and undo/redo. Every edit is a `Change` (each touched element before and
//! after), made through `Doc::edit`, which marks elements the way JOSM does: new ones get
//! negative ids, changed ones `action='modify'`, deleted ones stay in the file as
//! `action='delete'` (new ones that are deleted again just go).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bevy::prelude::*;

use crate::osm::{Frame, Meta, Node, Osm, Tags, Way};

/// The edits kept for undo.
const UNDO_DEPTH: usize = 200;

/// One edit: every element it touched, as it was before and after (`None`: not there).
#[derive(Default)]
pub struct Change {
    pub label: String,
    nodes: Vec<(i64, Option<Node>, Option<Node>)>,
    ways: Vec<(i64, Option<Way>, Option<Way>)>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.ways.is_empty()
    }
}

#[derive(Resource)]
pub struct Doc {
    pub osm: Osm,
    pub frame: Frame,
    pub path: PathBuf,
    /// Local map position of each node (x east, z south).
    pos: HashMap<i64, Vec2>,
    /// The ways (not deleted) each node is in.
    node_ways: HashMap<i64, Vec<i64>>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Changes since the last save.
    pub dirty: bool,
    /// Counts every change (and undo), for caches built from the data.
    pub revision: u64,
    /// The next new element gets this id (negative, going down).
    next_id: i64,
}

impl Doc {
    pub fn load(path: PathBuf, frame: Frame) -> Result<Self, String> {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let osm = Osm::parse(&text)?;
        let lowest = osm
            .nodes
            .keys()
            .chain(osm.ways.keys())
            .chain(osm.relations.keys())
            .copied()
            .min()
            .unwrap_or(0);
        let mut doc = Self {
            osm,
            frame,
            path,
            pos: HashMap::new(),
            node_ways: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            revision: 0,
            next_id: lowest.min(0) - 1,
        };
        doc.index();
        Ok(doc)
    }

    /// Builds the node positions and the ways per node.
    fn index(&mut self) {
        let frame = self.frame;
        self.pos = self
            .osm
            .nodes
            .iter()
            .map(|(&id, n)| (id, local(frame, n)))
            .collect();
        self.node_ways.clear();
        for (&id, way) in self.osm.ways.iter().filter(|(_, w)| !w.meta.deleted()) {
            for r in unique(&way.refs) {
                self.node_ways.entry(r).or_default().push(id);
            }
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        let tmp = self.path.with_extension("osm.saving");
        std::fs::write(&tmp, self.osm.write()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// A node that isn't deleted.
    pub fn node(&self, id: i64) -> Option<&Node> {
        self.osm.nodes.get(&id).filter(|n| !n.meta.deleted())
    }

    /// A way that isn't deleted.
    pub fn way(&self, id: i64) -> Option<&Way> {
        self.osm.ways.get(&id).filter(|w| !w.meta.deleted())
    }

    pub fn pos(&self, id: i64) -> Option<Vec2> {
        self.pos.get(&id).copied()
    }

    /// The ways a node is in.
    pub fn ways_of(&self, node: i64) -> &[i64] {
        self.node_ways.get(&node).map_or(&[], Vec::as_slice)
    }

    /// A way's points on the map (nodes that exist).
    pub fn points(&self, way: &Way) -> Vec<Vec2> {
        way.refs.iter().filter_map(|r| self.pos(*r)).collect()
    }

    pub fn can_undo(&self) -> Option<&str> {
        self.undo.last().map(|c| c.label.as_str())
    }

    pub fn can_redo(&self) -> Option<&str> {
        self.redo.last().map(|c| c.label.as_str())
    }

    /// Makes an edit; nothing is recorded if it changed nothing.
    pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Edit) -> R) -> R {
        let mut edit = Edit {
            doc: self,
            change: Change {
                label: label.into(),
                ..default()
            },
            touched_nodes: HashSet::new(),
            touched_ways: HashSet::new(),
        };
        let result = f(&mut edit);
        let change = edit.finish();
        if !change.is_empty() {
            self.undo.push(change);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
            self.revision += 1;
        }
        result
    }

    pub fn undo(&mut self) -> Option<String> {
        let change = self.undo.pop()?;
        self.apply(&change, false);
        let label = change.label.clone();
        self.redo.push(change);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let change = self.redo.pop()?;
        self.apply(&change, true);
        let label = change.label.clone();
        self.undo.push(change);
        Some(label)
    }

    /// Puts each element of a change as it was before it (or after, `forward`).
    fn apply(&mut self, change: &Change, forward: bool) {
        for (id, before, after) in &change.nodes {
            let state = if forward { after } else { before };
            self.put_node(*id, state.clone());
        }
        for (id, before, after) in &change.ways {
            let state = if forward { after } else { before };
            self.put_way(*id, state.clone());
        }
        self.dirty = true;
        self.revision += 1;
    }

    fn put_node(&mut self, id: i64, node: Option<Node>) {
        match node {
            Some(node) => {
                self.pos.insert(id, local(self.frame, &node));
                self.osm.nodes.insert(id, node);
            }
            None => {
                self.pos.remove(&id);
                self.osm.nodes.remove(&id);
            }
        }
    }

    fn put_way(&mut self, id: i64, way: Option<Way>) {
        if let Some(old) = self.osm.ways.get(&id).filter(|w| !w.meta.deleted()) {
            for r in unique(&old.refs) {
                if let Some(list) = self.node_ways.get_mut(&r) {
                    list.retain(|&w| w != id);
                }
            }
        }
        match way {
            Some(way) => {
                if !way.meta.deleted() {
                    for r in unique(&way.refs) {
                        self.node_ways.entry(r).or_default().push(id);
                    }
                }
                self.osm.ways.insert(id, way);
            }
            None => drop(self.osm.ways.remove(&id)),
        }
    }
}

/// An edit in progress (see `Doc::edit`).
pub struct Edit<'a> {
    pub doc: &'a mut Doc,
    change: Change,
    touched_nodes: HashSet<i64>,
    touched_ways: HashSet<i64>,
}

impl Edit<'_> {
    fn touch_node(&mut self, id: i64) {
        if self.touched_nodes.insert(id) {
            let before = self.doc.osm.nodes.get(&id).cloned();
            self.change.nodes.push((id, before, None));
        }
    }

    fn touch_way(&mut self, id: i64) {
        if self.touched_ways.insert(id) {
            let before = self.doc.osm.ways.get(&id).cloned();
            self.change.ways.push((id, before, None));
        }
    }

    fn new_id(&mut self) -> i64 {
        let id = self.doc.next_id;
        self.doc.next_id -= 1;
        id
    }

    /// A new node at a map point.
    pub fn add_node(&mut self, at: Vec2, tags: Tags) -> i64 {
        let id = self.new_id();
        let (lat, lon) = self.doc.frame.to_geo((at.x as f64, at.y as f64));
        self.touch_node(id);
        self.doc.put_node(
            id,
            Some(Node {
                meta: Meta {
                    version: 0,
                    action: Some("modify".into()),
                    tags,
                },
                lat: round7(lat),
                lon: round7(lon),
            }),
        );
        id
    }

    /// A new way through these nodes.
    pub fn add_way(&mut self, refs: Vec<i64>, tags: Tags) -> i64 {
        let id = self.new_id();
        self.touch_way(id);
        self.doc.put_way(
            id,
            Some(Way {
                meta: Meta {
                    version: 0,
                    action: Some("modify".into()),
                    tags,
                },
                refs,
            }),
        );
        id
    }

    /// Changes a node (that exists) through `f`.
    pub fn modify_node(&mut self, id: i64, f: impl FnOnce(&mut Node)) {
        let Some(mut node) = self.doc.node(id).cloned() else {
            return;
        };
        f(&mut node);
        self.touch_node(id);
        node.meta.action = Some("modify".into());
        self.doc.put_node(id, Some(node));
    }

    pub fn move_node(&mut self, id: i64, to: Vec2) {
        let (lat, lon) = self.doc.frame.to_geo((to.x as f64, to.y as f64));
        self.modify_node(id, |n| {
            n.lat = round7(lat);
            n.lon = round7(lon);
        });
    }

    /// Changes a way (that exists) through `f`.
    pub fn modify_way(&mut self, id: i64, f: impl FnOnce(&mut Way)) {
        let Some(mut way) = self.doc.way(id).cloned() else {
            return;
        };
        f(&mut way);
        self.touch_way(id);
        way.meta.action = Some("modify".into());
        self.doc.put_way(id, Some(way));
    }

    pub fn set_way_tags(&mut self, id: i64, tags: Tags) {
        self.modify_way(id, |w| w.meta.tags = tags);
    }

    pub fn set_node_tags(&mut self, id: i64, tags: Tags) {
        self.modify_node(id, |n| n.meta.tags = tags);
    }

    /// Deletes a node: kept in the file as deleted if it came from OSM, gone if new.
    fn delete_node_only(&mut self, id: i64) {
        let Some(node) = self.doc.node(id).cloned() else {
            return;
        };
        self.touch_node(id);
        if id < 0 {
            self.doc.put_node(id, None);
        } else {
            let mut node = node;
            node.meta.action = Some("delete".into());
            self.doc.put_node(id, Some(node));
        }
    }

    /// Deletes a way, and its nodes that nothing else uses (and that have no tags).
    pub fn delete_way(&mut self, id: i64) {
        let Some(way) = self.doc.way(id).cloned() else {
            return;
        };
        self.touch_way(id);
        if id < 0 {
            self.doc.put_way(id, None);
        } else {
            let mut deleted = way.clone();
            deleted.meta.action = Some("delete".into());
            self.doc.put_way(id, Some(deleted));
        }
        for r in unique(&way.refs) {
            let unused = self.doc.ways_of(r).is_empty() && !self.in_relation(r);
            if unused && self.doc.node(r).is_some_and(|n| n.meta.tags.is_empty()) {
                self.delete_node_only(r);
            }
        }
    }

    fn in_relation(&self, node: i64) -> bool {
        self.doc
            .osm
            .relations
            .values()
            .filter(|r| !r.meta.deleted())
            .any(|r| r.members.iter().any(|m| m.kind == "node" && m.id == node))
    }

    /// Takes a node out of every way and deletes it. Ways left with fewer than two
    /// points (closed ones: fewer than three) are deleted too.
    pub fn delete_node(&mut self, id: i64) {
        for way_id in self.doc.ways_of(id).to_vec() {
            let Some(way) = self.doc.way(way_id) else {
                continue;
            };
            let closed = way.refs.len() > 2 && way.refs.first() == way.refs.last();
            let mut refs: Vec<i64> = way.refs.iter().copied().filter(|&r| r != id).collect();
            if closed && refs.first() != refs.last() {
                refs.push(refs[0]);
            }
            refs.dedup();
            let too_few = if closed {
                refs.len() < 4
            } else {
                refs.len() < 2
            };
            if too_few {
                self.delete_way(way_id);
            } else {
                self.modify_way(way_id, |w| w.refs = refs);
            }
        }
        self.delete_node_only(id);
    }

    /// Puts a new node at `at` into a way, after its `index`th point.
    pub fn insert_node(&mut self, way: i64, index: usize, at: Vec2, tags: Tags) -> i64 {
        let node = self.add_node(at, tags);
        self.modify_way(way, |w| w.refs.insert(index + 1, node));
        node
    }

    /// Splits a way at one of its inner nodes; the second half becomes a new way with
    /// the same tags. Returns it.
    pub fn split_way(&mut self, way_id: i64, node: i64) -> Option<i64> {
        let way = self.doc.way(way_id)?.clone();
        let at = way.refs[1..way.refs.len() - 1]
            .iter()
            .position(|&r| r == node)?
            + 1;
        let (first, second) = (way.refs[..=at].to_vec(), way.refs[at..].to_vec());
        self.modify_way(way_id, |w| w.refs = first);
        Some(self.add_way(second, way.meta.tags.clone()))
    }

    fn finish(mut self) -> Change {
        for (id, before, after) in &mut self.change.nodes {
            *after = self.doc.osm.nodes.get(id).cloned();
            if same_node(before, after) {
                *id = i64::MIN;
            }
        }
        for (id, before, after) in &mut self.change.ways {
            *after = self.doc.osm.ways.get(id).cloned();
            if same_way(before, after) {
                *id = i64::MIN;
            }
        }
        self.change.nodes.retain(|(id, ..)| *id != i64::MIN);
        self.change.ways.retain(|(id, ..)| *id != i64::MIN);
        self.change
    }
}

fn same_node(a: &Option<Node>, b: &Option<Node>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.lat == b.lat
                && a.lon == b.lon
                && a.meta.tags == b.meta.tags
                && a.meta.action == b.meta.action
        }
        _ => false,
    }
}

fn same_way(a: &Option<Way>, b: &Option<Way>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.refs == b.refs && a.meta.tags == b.meta.tags && a.meta.action == b.meta.action
        }
        _ => false,
    }
}

fn local(frame: Frame, node: &Node) -> Vec2 {
    let (x, z) = frame.to_local(node.lat, node.lon);
    Vec2::new(x as f32, z as f32)
}

/// Seven decimals, as OSM stores coordinates (about a centimetre).
fn round7(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

/// A way's nodes, each once (a closed way repeats its first).
fn unique(refs: &[i64]) -> Vec<i64> {
    let mut seen = Vec::with_capacity(refs.len());
    for &r in refs {
        if !seen.contains(&r) {
            seen.push(r);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Doc {
        let mut doc = Doc {
            osm: Osm::default(),
            frame: Frame::around(50.0, 10.8),
            path: PathBuf::new(),
            pos: HashMap::new(),
            node_ways: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            revision: 0,
            next_id: -1,
        };
        doc.index();
        doc
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut doc = doc();
        let way = doc.edit("draw", |e| {
            let a = e.add_node(Vec2::ZERO, Tags::new());
            let b = e.add_node(Vec2::new(10.0, 0.0), Tags::new());
            let c = e.add_node(Vec2::new(20.0, 5.0), Tags::new());
            e.add_way(
                vec![a, b, c],
                Tags::from([("highway".into(), "track".into())]),
            )
        });
        assert_eq!(doc.osm.nodes.len(), 3);
        let b = doc.way(way).unwrap().refs[1];
        assert!(doc.pos(b).unwrap().distance(Vec2::new(10.0, 0.0)) < 0.05);
        let second = doc.edit("split", |e| e.split_way(way, b)).unwrap();
        assert_eq!(doc.way(way).unwrap().refs.len(), 2);
        assert_eq!(doc.ways_of(b).len(), 2);
        assert_eq!(doc.way(second).unwrap().meta.tags["highway"], "track");
        doc.edit("delete", |e| e.delete_node(b));
        // Both halves have one point left: gone, and so are their new nodes.
        assert!(doc.osm.ways.is_empty() && doc.osm.nodes.is_empty());
        doc.undo();
        doc.undo();
        assert_eq!(doc.way(way).unwrap().refs.len(), 3);
        assert_eq!(doc.ways_of(b), [way]);
        doc.redo();
        assert_eq!(doc.ways_of(b).len(), 2);
    }

    #[test]
    fn mapped_elements_stay_in_the_file_when_deleted() {
        let mut doc = doc();
        let node = Node {
            meta: Meta {
                version: 3,
                action: None,
                tags: Tags::new(),
            },
            lat: 50.0,
            lon: 10.8,
        };
        doc.osm.nodes.insert(7, node.clone());
        doc.osm.nodes.insert(
            8,
            Node {
                lat: 50.001,
                ..node
            },
        );
        doc.osm.ways.insert(
            9,
            Way {
                meta: Meta::default(),
                refs: vec![7, 8],
            },
        );
        doc.index();
        doc.edit("delete", |e| e.delete_way(9));
        assert!(doc.osm.ways[&9].meta.deleted());
        assert!(doc.osm.nodes[&7].meta.deleted() && doc.way(9).is_none());
        assert!(doc.dirty);
        doc.undo();
        assert_eq!(doc.osm.ways[&9].meta.action, None);
        assert_eq!(doc.ways_of(7), [9]);
    }
}
