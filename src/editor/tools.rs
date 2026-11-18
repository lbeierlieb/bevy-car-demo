//! The editing tools, by mouse and keys:
//!
//! - Select (1): click a way or node; drag a node to move it; Ctrl+click on a way puts a
//!   new node there (and drags it). Delete removes the selected node (or way), X splits
//!   the selected way at the selected node, Page Up/Down raise and lower the selected
//!   node's height pin (`ele`; Shift: by a metre).
//! - Draw (2–6: road, fence, building, land use, terrain area): click to add points
//!   (clicking a node joins it, clicking a way puts a shared node into it), Enter or a
//!   click on the last point finishes, a click on the first point closes an area,
//!   Backspace takes the last point back, Escape cancels.
//! - Place (7): bumps, humps, tables, potholes, gates and height pins (the kind is picked
//!   in the panel), on the node or way clicked.
//! - Ctrl+Z undo, Ctrl+Y (or Ctrl+Shift+Z) redo, Ctrl+S save, Escape back to Select.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;

use super::doc::{Doc, Edit};
use super::index::{Class, TERRAIN};
use crate::map::MapView;
use crate::osm::Tags;
use crate::terrain::Terrain;

pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tools>()
            .init_resource::<Selection>()
            .init_resource::<Hover>()
            .add_systems(Update, act);
    }
}

#[derive(Resource, Default)]
pub struct Selection {
    pub way: Option<i64>,
    pub node: Option<i64>,
}

/// Something under the cursor: a node, or the nearest point on a way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Node(i64),
    Segment { way: i64, at: Vec2 },
}

#[derive(Resource, Default)]
pub struct Hover {
    /// The ground under the cursor.
    pub ground: Option<Vec3>,
    pub target: Option<Target>,
    pub screen: Option<Vec2>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Draw(Class),
    Place,
}

impl Tool {
    /// The tools by their number key (1–7).
    pub const ALL: [Tool; 7] = [
        Tool::Select,
        Tool::Draw(Class::Road),
        Tool::Draw(Class::Fence),
        Tool::Draw(Class::Building),
        Tool::Draw(Class::Area),
        Tool::Draw(Class::Terrain),
        Tool::Place,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Draw(Class::Road) => "Road",
            Tool::Draw(Class::Fence) => "Fence",
            Tool::Draw(Class::Building) => "Building",
            Tool::Draw(Class::Area) => "Land use",
            Tool::Draw(Class::Terrain) => "Terrain",
            Tool::Draw(_) => "Way",
            Tool::Place => "Place",
        }
    }

    /// Whether it draws closed ways (areas).
    pub fn closed(self) -> bool {
        matches!(
            self,
            Tool::Draw(Class::Building | Class::Area | Class::Terrain)
        )
    }
}

/// What the Place tool puts down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Bump,
    Hump,
    Table,
    Pothole,
    Gate,
    Height,
}

impl Place {
    pub const ALL: [Place; 6] = [
        Place::Bump,
        Place::Hump,
        Place::Table,
        Place::Pothole,
        Place::Gate,
        Place::Height,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Place::Bump => "Speed bump",
            Place::Hump => "Hump",
            Place::Table => "Table",
            Place::Pothole => "Pothole",
            Place::Gate => "Gate (on a fence)",
            Place::Height => "Height pin (on a road)",
        }
    }

    fn tag(self) -> (&'static str, &'static str) {
        match self {
            Place::Bump => ("traffic_calming", "bump"),
            Place::Hump => ("traffic_calming", "hump"),
            Place::Table => ("traffic_calming", "table"),
            Place::Pothole => ("hazard", "pothole"),
            Place::Gate => ("barrier", "gate"),
            Place::Height => ("ele", ""),
        }
    }
}

/// A point of the way being drawn.
#[derive(Clone, Copy, Debug)]
pub enum DrawPoint {
    /// A node that's there already.
    Node(i64),
    /// A new node put into a way (joining it).
    OnWay(i64, Vec2),
    New(Vec2),
}

impl DrawPoint {
    pub fn at(&self, doc: &Doc) -> Option<Vec2> {
        match *self {
            DrawPoint::Node(id) => doc.pos(id),
            DrawPoint::OnWay(_, p) | DrawPoint::New(p) => Some(p),
        }
    }
}

/// Values the panel offers for new ways, as (key, value) pairs.
pub const ROADS: [&str; 9] = [
    "residential",
    "unclassified",
    "tertiary",
    "secondary",
    "primary",
    "service",
    "living_street",
    "track",
    "path",
];
pub const FENCES: [&str; 6] = [
    "fence",
    "wall",
    "hedge",
    "guard_rail",
    "railing",
    "jersey_barrier",
];
/// Land use kinds: a label and its tag.
pub const AREAS: [(&str, &str, &str); 9] = [
    ("Paved", "area:highway", "yes"),
    ("Parking", "amenity", "parking"),
    ("Forest", "landuse", "forest"),
    ("Field", "landuse", "farmland"),
    ("Meadow", "landuse", "meadow"),
    ("Grass", "landuse", "grass"),
    ("Scrub", "natural", "scrub"),
    ("Water", "natural", "water"),
    ("Residential", "landuse", "residential"),
];
/// Terrain area operations (`carsim:terrain`).
pub const TERRAIN_OPS: [(&str, &str); 4] = [
    ("flatten", "Flatten to a height"),
    ("raise", "Raise or lower"),
    ("smooth", "Smooth"),
    ("slope", "Slope through corner heights"),
];

#[derive(Resource)]
pub struct Tools {
    pub tool: Tool,
    pub road: usize,
    pub fence: usize,
    pub area: usize,
    pub terrain: usize,
    pub place: Place,
    /// The way being drawn.
    pub drawing: Vec<DrawPoint>,
    pub drag: Option<Drag>,
    /// The last thing that happened, for the panel.
    pub status: String,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            road: 1,
            fence: 0,
            area: 0,
            terrain: 0,
            place: Place::Bump,
            drawing: Vec::new(),
            drag: None,
            status: String::new(),
        }
    }
}

/// A node being dragged.
pub struct Drag {
    pub node: i64,
    /// Where the press was (screen), to tell a click from a drag.
    pub start: Vec2,
    pub to: Vec2,
    pub moved: bool,
}

/// What the panel and the keys ask for (shared, so buttons and keys do the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Redo,
    Save,
    Delete,
    Split,
    Finish,
    Cancel,
    /// Height pin up or down by this many centimetres.
    Raise(i32),
    Unpin,
}

#[allow(clippy::too_many_arguments)]
pub fn act(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    view: Res<MapView>,
    terrain: Res<Terrain>,
    hover: Res<Hover>,
    mut doc: ResMut<Doc>,
    mut tools: ResMut<Tools>,
    mut selection: ResMut<Selection>,
) {
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let mut commands = Vec::new();
    if !view.open && !egui.wants_keyboard_input() {
        if ctrl && keys.just_pressed(KeyCode::KeyZ) {
            commands.push(if shift { Command::Redo } else { Command::Undo });
        }
        if ctrl && keys.just_pressed(KeyCode::KeyY) {
            commands.push(Command::Redo);
        }
        if ctrl && keys.just_pressed(KeyCode::KeyS) {
            commands.push(Command::Save);
        }
        if keys.just_pressed(KeyCode::Delete) {
            commands.push(Command::Delete);
        }
        if keys.just_pressed(KeyCode::Backspace) {
            commands.push(if tools.drawing.is_empty() {
                Command::Delete
            } else {
                Command::Cancel
            });
        }
        if keys.just_pressed(KeyCode::KeyX) && !ctrl {
            commands.push(Command::Split);
        }
        if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
            commands.push(Command::Finish);
        }
        let step = if shift { 100 } else { 10 };
        if keys.just_pressed(KeyCode::PageUp) {
            commands.push(Command::Raise(step));
        }
        if keys.just_pressed(KeyCode::PageDown) {
            commands.push(Command::Raise(-step));
        }
        if keys.just_pressed(KeyCode::Escape) {
            if !tools.drawing.is_empty() {
                tools.drawing.clear();
            } else if tools.tool != Tool::Select {
                tools.tool = Tool::Select;
            } else {
                *selection = Selection::default();
            }
        }
        for (i, key) in [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
        ]
        .into_iter()
        .enumerate()
        {
            if keys.just_pressed(key) {
                set_tool(&mut tools, Tool::ALL[i]);
            }
        }
    }
    for command in commands {
        run(command, &mut doc, &mut tools, &mut selection, &terrain);
    }

    let pointer_free = !(view.open || egui.is_pointer_over_area() || egui.wants_pointer_input());
    if !buttons.pressed(MouseButton::Left) && !buttons.just_released(MouseButton::Left) {
        tools.drag = None;
    }
    if !pointer_free && tools.drag.is_none() {
        return;
    }
    match tools.tool {
        Tool::Select => select(&buttons, ctrl, &hover, &mut doc, &mut tools, &mut selection),
        Tool::Draw(_) => {
            if buttons.just_pressed(MouseButton::Left) {
                draw_click(&hover, &terrain, &mut doc, &mut tools, &mut selection);
            }
        }
        Tool::Place => {
            if buttons.just_pressed(MouseButton::Left) {
                place(&hover, &terrain, &mut doc, &mut tools, &mut selection);
            }
        }
    }
}

pub fn set_tool(tools: &mut Tools, tool: Tool) {
    if tools.tool != tool {
        tools.drawing.clear();
        tools.tool = tool;
    }
}

/// Carries out a command from the keys or the panel.
pub fn run(
    command: Command,
    doc: &mut Doc,
    tools: &mut Tools,
    selection: &mut Selection,
    terrain: &Terrain,
) {
    let status = match command {
        Command::Undo => doc.undo().map(|l| format!("Undid: {l}")),
        Command::Redo => doc.redo().map(|l| format!("Redid: {l}")),
        Command::Save => Some(match doc.save() {
            Ok(()) => format!("Saved {}", doc.path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        }),
        Command::Delete => {
            if let Some(node) = selection.node.filter(|&n| doc.node(n).is_some()) {
                doc.edit("delete node", |e| e.delete_node(node));
                selection.node = None;
                Some("Deleted the node".into())
            } else if let Some(way) = selection.way.filter(|&w| doc.way(w).is_some()) {
                doc.edit("delete way", |e| e.delete_way(way));
                selection.way = None;
                Some(format!("Deleted way {way}"))
            } else {
                None
            }
        }
        Command::Split => match (selection.way, selection.node) {
            (Some(way), Some(node)) => match doc.edit("split way", |e| e.split_way(way, node)) {
                Some(second) => Some(format!("Split: the second half is way {second}")),
                None => Some("Only a way's inner nodes split it".into()),
            },
            _ => Some("Select a way and one of its nodes to split it".into()),
        },
        Command::Finish => {
            finish(doc, tools, selection, terrain);
            None
        }
        Command::Cancel => {
            tools.drawing.pop();
            None
        }
        Command::Raise(cm) => selection.node.map(|node| {
            let ground = doc
                .pos(node)
                .map_or(0.0, |p| terrain.sample(p.x, p.y).0 + terrain.base);
            let mut now = 0.0;
            doc.edit("height pin", |e| {
                e.modify_node(node, |n| {
                    let ele = n
                        .meta
                        .tags
                        .get("ele")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(ground);
                    now = ele + cm as f32 / 100.0;
                    n.meta.tags.insert("ele".into(), format!("{now:.2}"));
                })
            });
            format!(
                "Height pin at {now:.2} m ({:+.2} m from the ground)",
                now - ground
            )
        }),
        Command::Unpin => selection.node.map(|node| {
            doc.edit("remove height pin", |e| {
                e.modify_node(node, |n| drop(n.meta.tags.remove("ele")))
            });
            "Removed the height pin".into()
        }),
    };
    if let Some(status) = status {
        tools.status = status;
    }
}

fn select(
    buttons: &ButtonInput<MouseButton>,
    ctrl: bool,
    hover: &Hover,
    doc: &mut Doc,
    tools: &mut Tools,
    selection: &mut Selection,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let start = hover.screen.unwrap_or_default();
        match hover.target {
            Some(Target::Segment { way, at }) if ctrl => {
                let node = doc.edit("add node", |e| insert_on_way(e, way, at, Tags::new()));
                *selection = Selection {
                    way: Some(way),
                    node: Some(node),
                };
                tools.drag = Some(Drag {
                    node,
                    start,
                    to: at,
                    moved: false,
                });
            }
            Some(Target::Node(node)) => {
                let ways = doc.ways_of(node);
                let way = selection
                    .way
                    .filter(|w| ways.contains(w))
                    .or(ways.first().copied());
                *selection = Selection {
                    way,
                    node: Some(node),
                };
                tools.drag = Some(Drag {
                    node,
                    start,
                    to: doc.pos(node).unwrap_or_default(),
                    moved: false,
                });
            }
            Some(Target::Segment { way, .. }) => {
                *selection = Selection {
                    way: Some(way),
                    node: None,
                };
            }
            None => *selection = Selection::default(),
        }
        return;
    }
    let Some(drag) = &mut tools.drag else {
        return;
    };
    if let (Some(screen), Some(ground)) = (hover.screen, hover.ground) {
        if screen.distance(drag.start) > 4.0 {
            drag.moved = true;
        }
        if drag.moved {
            drag.to = ground.xz();
        }
    }
    if buttons.just_released(MouseButton::Left) {
        let drag = tools.drag.take().expect("dragging");
        if drag.moved {
            doc.edit("move node", |e| e.move_node(drag.node, drag.to));
        }
    }
}

fn draw_click(
    hover: &Hover,
    terrain: &Terrain,
    doc: &mut Doc,
    tools: &mut Tools,
    selection: &mut Selection,
) {
    let Some(ground) = hover.ground else {
        return;
    };
    let point = match hover.target {
        Some(Target::Node(id)) => DrawPoint::Node(id),
        Some(Target::Segment { way, at, .. }) => DrawPoint::OnWay(way, at),
        None => DrawPoint::New(ground.xz()),
    };
    let at = point.at(doc).unwrap_or(ground.xz());
    let near = |p: &DrawPoint| p.at(doc).is_some_and(|q| q.distance(at) < 0.5);
    let same = |a: &DrawPoint, b: &DrawPoint| match (a, b) {
        (DrawPoint::Node(a), DrawPoint::Node(b)) => a == b,
        _ => near(a) && near(b),
    };
    if let Some(first) = tools.drawing.first()
        && tools.tool.closed()
        && tools.drawing.len() >= 3
        && same(first, &point)
    {
        let first = *first;
        tools.drawing.push(first);
        finish(doc, tools, selection, terrain);
        return;
    }
    if let Some(last) = tools.drawing.last()
        && same(last, &point)
    {
        finish(doc, tools, selection, terrain);
        return;
    }
    tools.drawing.push(point);
}

/// The tags a new way of the drawing tool gets.
fn new_tags(tools: &Tools, terrain_heights: &[f32]) -> Tags {
    let mut tags = Tags::new();
    let mut set = |k: &str, v: &str| drop(tags.insert(k.into(), v.into()));
    match tools.tool {
        Tool::Draw(Class::Road) => set("highway", ROADS[tools.road]),
        Tool::Draw(Class::Fence) => set("barrier", FENCES[tools.fence]),
        Tool::Draw(Class::Building) => set("building", "yes"),
        Tool::Draw(Class::Area) => {
            let (_, k, v) = AREAS[tools.area];
            set(k, v);
        }
        Tool::Draw(Class::Terrain) => {
            let (op, _) = TERRAIN_OPS[tools.terrain];
            set(TERRAIN, op);
            set("carsim:fade", "5");
            let mean = terrain_heights.iter().sum::<f32>() / terrain_heights.len().max(1) as f32;
            match op {
                "flatten" => set("carsim:height", &format!("{mean:.1}")),
                "raise" => set("carsim:offset", "1"),
                "smooth" => set("carsim:radius", "6"),
                _ => {}
            }
        }
        _ => {}
    }
    tags
}

/// Makes the way being drawn.
fn finish(doc: &mut Doc, tools: &mut Tools, selection: &mut Selection, terrain: &Terrain) {
    let closed = tools.tool.closed();
    let mut points = std::mem::take(&mut tools.drawing);
    if closed {
        if points.len() >= 2
            && points.first().map(|p| p.at(doc)) == points.last().map(|p| p.at(doc))
        {
            points.pop();
        }
        if points.len() < 3 {
            tools.status = "An area needs three points or more".into();
            return;
        }
    } else if points.len() < 2 {
        tools.status = "A way needs two points or more".into();
        return;
    }
    // The bare terrain's heights at the points (terrain areas work on it).
    let heights: Vec<f32> = points
        .iter()
        .filter_map(|p| p.at(doc))
        .map(|p| terrain.land(p.x, p.y).0 + terrain.base)
        .collect();
    let tags = new_tags(tools, &heights);
    let slope = tags.get(TERRAIN).is_some_and(|op| op == "slope");
    let label = format!("draw {}", tools.tool.name().to_lowercase());
    let way = doc.edit(&label, |e| {
        let mut refs: Vec<i64> = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ele = slope.then(|| heights.get(i).copied()).flatten();
                let mut node_tags = Tags::new();
                if let Some(ele) = ele {
                    node_tags.insert("ele".into(), format!("{ele:.1}"));
                }
                match *p {
                    DrawPoint::Node(id) => id,
                    DrawPoint::OnWay(way, at) => insert_on_way(e, way, at, node_tags),
                    DrawPoint::New(at) => e.add_node(at, node_tags),
                }
            })
            .collect();
        refs.dedup();
        if closed {
            refs.push(refs[0]);
        }
        e.add_way(refs, tags)
    });
    *selection = Selection {
        way: Some(way),
        node: None,
    };
    tools.status = format!("Drew way {way}");
}

/// Puts a new node into a way at the segment nearest `at`.
pub fn insert_on_way(e: &mut Edit, way: i64, at: Vec2, tags: Tags) -> i64 {
    let Some(w) = e.doc.way(way) else {
        return e.add_node(at, tags);
    };
    let points: Vec<Vec2> = w
        .refs
        .iter()
        .map(|r| e.doc.pos(*r).unwrap_or(Vec2::splat(f32::MAX)))
        .collect();
    let (index, at) = nearest_segment(&points, at);
    e.insert_node(way, index, at, tags)
}

/// The segment of a polyline nearest a point (by its first point), and the nearest point
/// on it.
pub fn nearest_segment(points: &[Vec2], p: Vec2) -> (usize, Vec2) {
    let mut best = (f32::MAX, 0, p);
    for (i, pair) in points.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
        let q = a.lerp(b, t);
        if q.distance(p) < best.0 {
            best = (q.distance(p), i, q);
        }
    }
    (best.1, best.2)
}

fn place(
    hover: &Hover,
    terrain: &Terrain,
    doc: &mut Doc,
    tools: &mut Tools,
    selection: &mut Selection,
) {
    let Some(ground) = hover.ground else {
        return;
    };
    let kind = tools.place;
    let (key, value) = kind.tag();
    let wanted = match kind {
        Place::Gate => Some(Class::Fence),
        Place::Height | Place::Bump | Place::Hump | Place::Table => Some(Class::Road),
        Place::Pothole => None,
    };
    let class_of = |doc: &Doc, way: i64| doc.way(way).map(|w| Class::of(&w.meta.tags));
    let tag_value = |at: Vec2| -> String {
        if kind == Place::Height {
            format!("{:.2}", terrain.sample(at.x, at.y).0 + terrain.base)
        } else {
            value.into()
        }
    };
    let node = match hover.target {
        Some(Target::Node(id))
            if wanted.is_none() || doc.ways_of(id).iter().any(|&w| class_of(doc, w) == wanted) =>
        {
            let v = tag_value(doc.pos(id).unwrap_or_default());
            doc.edit(kind.name(), |e| {
                e.modify_node(id, |n| drop(n.meta.tags.insert(key.into(), v)))
            });
            Some(id)
        }
        Some(Target::Segment { way, at }) if wanted.is_none() || class_of(doc, way) == wanted => {
            let tags = Tags::from([(key.to_string(), tag_value(at))]);
            Some(doc.edit(kind.name(), |e| insert_on_way(e, way, at, tags)))
        }
        _ if matches!(
            kind,
            Place::Pothole | Place::Bump | Place::Hump | Place::Table
        ) =>
        {
            // The bake puts these on the nearest road anyway.
            let at = ground.xz();
            let tags = Tags::from([(key.to_string(), tag_value(at))]);
            Some(doc.edit(kind.name(), |e| e.add_node(at, tags)))
        }
        _ => None,
    };
    match node {
        Some(node) => {
            selection.node = Some(node);
            selection.way = doc.ways_of(node).first().copied();
            tools.status = format!("Placed: {}", kind.name());
        }
        None => {
            tools.status = match wanted {
                Some(Class::Fence) => "Click on a fence".into(),
                _ => "Click on a road".into(),
            }
        }
    }
}
