//! The editor's window (top right): file and bake, tools, the selected way or node with its
//! tags, the bake's conflicts, what the overlay shows, and the controls.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use super::bake::Bake;
use super::camera::{FlyCamera, look_at};
use super::conflicts::Conflicts;
use super::doc::Doc;
use super::index::{Class, TERRAIN};
use super::overlay::Layers;
use super::tools::{
    AREAS, Command, FENCES, Place, ROADS, Selection, TERRAIN_OPS, Tool, Tools, run, set_tool,
};
use crate::map::MapView;
use crate::osm::Tags;
use crate::terrain::Terrain;

pub struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TagTable>()
            .add_systems(EguiPrimaryContextPass, panel);
    }
}

/// The tag table being edited: rows are typed into and written to the element when a
/// field is left (one undo step), and read again when the element or the data changes.
/// One for the selected node, one for the selected way.
#[derive(Resource, Default)]
struct TagTable([Rows; 2]);

#[derive(Default)]
struct Rows {
    /// The element's id and the data revision the rows were read at.
    of: Option<(i64, u64)>,
    rows: Vec<(String, String)>,
}

/// Values offered for tags, by key (the table's value field has them in a menu).
const VALUES: &[(&str, &[&str])] = &[
    ("highway", &ROADS),
    (
        "surface",
        &[
            "asphalt",
            "concrete",
            "paving_stones",
            "sett",
            "cobblestone",
            "compacted",
            "fine_gravel",
            "gravel",
            "ground",
            "dirt",
            "grass",
        ],
    ),
    (
        "smoothness",
        &[
            "excellent",
            "good",
            "intermediate",
            "bad",
            "very_bad",
            "horrible",
            "very_horrible",
            "impassable",
        ],
    ),
    (
        "tracktype",
        &["grade1", "grade2", "grade3", "grade4", "grade5"],
    ),
    ("bridge", &["yes", "no"]),
    ("barrier", &FENCES),
    (
        "fence_type",
        &[
            "wood",
            "chain_link",
            "metal",
            "railing",
            "wire",
            "panel",
            "concrete",
            "palisade",
        ],
    ),
    ("waterway", &["river", "stream", "canal", "ditch", "drain"]),
    ("building", &["yes", "house", "garage", "barn", "shed"]),
    (TERRAIN, &["flatten", "raise", "smooth", "slope"]),
    ("traffic_calming", &["bump", "hump", "table"]),
];

/// Tags worth adding, by what the way is.
fn suggestions(class: Option<Class>) -> &'static [&'static str] {
    match class {
        Some(Class::Road) => &[
            "width",
            "surface",
            "smoothness",
            "tracktype",
            "lanes",
            "bridge",
            "layer",
            "name",
        ],
        Some(Class::Water) => &["width", "tunnel"],
        Some(Class::Fence) => &["fence_type", "height"],
        Some(Class::Building) => &["building:levels", "height"],
        Some(Class::Terrain) => &[
            "carsim:height",
            "carsim:offset",
            "carsim:radius",
            "carsim:fade",
        ],
        Some(_) => &["name"],
        None => &["ele", "height", "width", "diameter", "depth"],
    }
}

#[derive(SystemParam)]
struct State<'w, 's> {
    doc: ResMut<'w, Doc>,
    tools: ResMut<'w, Tools>,
    selection: ResMut<'w, Selection>,
    table: ResMut<'w, TagTable>,
    layers: ResMut<'w, Layers>,
    bake: ResMut<'w, Bake>,
    conflicts: ResMut<'w, Conflicts>,
    terrain: Res<'w, Terrain>,
    map: Res<'w, MapView>,
    cameras: Query<'w, 's, (&'static mut FlyCamera, &'static mut Transform)>,
}

fn panel(mut contexts: EguiContexts, mut s: State) -> Result {
    let ctx = contexts.ctx_mut()?;
    if s.map.open {
        return Ok(());
    }
    let height = ctx.content_rect().height() - 60.0;
    egui::Window::new("Map editor")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .default_width(340.0)
        .resizable(false)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(height)
                .show(ui, |ui| {
                    ui.set_width(330.0);
                    file(ui, &mut s);
                    ui.separator();
                    tools(ui, &mut s);
                    ui.separator();
                    selected(ui, &mut s);
                    ui.separator();
                    conflicts(ui, &mut s);
                    ui.separator();
                    layers(ui, &mut s);
                    help(ui);
                });
        });
    Ok(())
}

fn command(s: &mut State, command: Command) {
    run(
        command,
        &mut s.doc,
        &mut s.tools,
        &mut s.selection,
        &s.terrain,
    );
}

fn file(ui: &mut egui::Ui, s: &mut State) {
    ui.horizontal(|ui| {
        let name = s
            .doc
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.strong(name);
        if s.doc.dirty {
            ui.colored_label(egui::Color32::from_rgb(255, 190, 80), "● unsaved");
        }
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(s.doc.dirty, egui::Button::new("Save"))
            .on_hover_text("Ctrl+S")
            .clicked()
        {
            command(s, Command::Save);
        }
        let undo = s.doc.can_undo().map(str::to_owned);
        if ui
            .add_enabled(undo.is_some(), egui::Button::new("Undo"))
            .on_hover_text(format!("Ctrl+Z: {}", undo.unwrap_or_default()))
            .clicked()
        {
            command(s, Command::Undo);
        }
        let redo = s.doc.can_redo().map(str::to_owned);
        if ui
            .add_enabled(redo.is_some(), egui::Button::new("Redo"))
            .on_hover_text(format!("Ctrl+Y: {}", redo.unwrap_or_default()))
            .clicked()
        {
            command(s, Command::Redo);
        }
        let running = s.bake.running();
        if ui
            .add_enabled(running.is_none(), egui::Button::new("Save and bake"))
            .on_hover_text("F5: save, bake the map and reload it here")
            .clicked()
        {
            s.bake.requested = true;
        }
    });
    if let Some((elapsed, line)) = s.bake.running() {
        ui.add(
            egui::ProgressBar::new(Bake::progress(elapsed)).text(format!("Baking… {elapsed:.0} s")),
        );
        ui.small(line);
    } else if let Some((ok, message)) = &s.bake.last {
        let color = if *ok {
            egui::Color32::from_rgb(140, 220, 140)
        } else {
            egui::Color32::from_rgb(255, 120, 100)
        };
        ui.colored_label(color, message);
    }
    if !s.tools.status.is_empty() {
        ui.label(&s.tools.status);
    }
}

fn tools(ui: &mut egui::Ui, s: &mut State) {
    ui.horizontal_wrapped(|ui| {
        for (i, tool) in Tool::ALL.into_iter().enumerate() {
            let label = format!("{} {}", i + 1, tool.name());
            if ui.selectable_label(s.tools.tool == tool, label).clicked() {
                set_tool(&mut s.tools, tool);
            }
        }
    });
    let tools = &mut *s.tools;
    match tools.tool {
        Tool::Select => {
            ui.small("Click a way or node; drag nodes; Ctrl+click a way adds a node.");
        }
        Tool::Draw(class) => {
            match class {
                Class::Road => pick(ui, "highway", &mut tools.road, &ROADS),
                Class::Fence => pick(ui, "barrier", &mut tools.fence, &FENCES),
                Class::Area => {
                    let names: Vec<&str> = AREAS.iter().map(|a| a.0).collect();
                    pick(ui, "kind", &mut tools.area, &names);
                }
                Class::Terrain => {
                    let names: Vec<&str> = TERRAIN_OPS.iter().map(|o| o.1).collect();
                    pick(ui, "operation", &mut tools.terrain, &names);
                }
                _ => {}
            }
            let points = tools.drawing.len();
            ui.small(format!(
                "Click to add points ({points} so far); Enter or the last point again \
                 finishes{}, Backspace takes one back, Esc cancels.",
                if tools.tool.closed() {
                    ", the first point closes"
                } else {
                    ""
                }
            ));
        }
        Tool::Place => {
            ui.horizontal_wrapped(|ui| {
                for kind in Place::ALL {
                    ui.selectable_value(&mut tools.place, kind, kind.name());
                }
            });
        }
    }
}

/// A menu choosing one of `options` by index.
fn pick(ui: &mut egui::Ui, label: &str, index: &mut usize, options: &[&str]) {
    egui::ComboBox::from_label(label)
        .selected_text(options[*index])
        .show_ui(ui, |ui| {
            for (i, option) in options.iter().enumerate() {
                ui.selectable_value(index, i, *option);
            }
        });
}

fn selected(ui: &mut egui::Ui, s: &mut State) {
    let (way, node) = (s.selection.way, s.selection.node);
    if way.is_none() && node.is_none() {
        ui.label("Nothing selected.");
        return;
    }
    if let Some(id) = way.filter(|w| s.doc.way(*w).is_some()) {
        let w = s.doc.way(id).expect("checked").clone();
        let class = Class::of(&w.meta.tags);
        let points = s.doc.points(&w);
        let length: f32 = points.windows(2).map(|p| p[0].distance(p[1])).sum();
        ui.horizontal(|ui| {
            ui.strong(format!("Way {id}"));
            ui.label(format!(
                "{}, {} nodes, {length:.0} m",
                class.name().trim_end_matches('s'),
                w.refs.len()
            ));
        });
        ui.horizontal(|ui| {
            if ui.button("Delete way").on_hover_text("Delete").clicked() {
                s.selection.node = None;
                command(s, Command::Delete);
            }
            if ui.button("Fly here").clicked()
                && let Some(&p) = points.get(points.len() / 2)
            {
                fly_to(s, p);
            }
        });
        tag_table(ui, s, true, id, Some(class));
    }
    if let Some(id) = node.filter(|n| s.doc.node(*n).is_some()) {
        ui.separator();
        let p = s.doc.pos(id).unwrap_or_default();
        let ground = s.terrain.sample(p.x, p.y).0;
        let ways = s.doc.ways_of(id).to_vec();
        ui.horizontal(|ui| {
            ui.strong(format!("Node {id}"));
            ui.label(format!("in {} way(s), ground {ground:.2} m", ways.len()));
        });
        let ele = s
            .doc
            .node(id)
            .and_then(|n| n.meta.tags.get("ele"))
            .and_then(|e| e.parse::<f32>().ok());
        ui.horizontal(|ui| {
            match ele {
                Some(ele) => ui.label(format!("Height pin {ele:.2} m ({:+.2})", ele - ground)),
                None => ui.label("No height pin"),
            };
            for (label, cm) in [("▲", 10), ("▼", -10), ("▲1m", 100), ("▼1m", -100)] {
                if ui
                    .small_button(label)
                    .on_hover_text("Page Up/Down (Shift: a metre)")
                    .clicked()
                {
                    command(s, Command::Raise(cm));
                }
            }
            if ele.is_some() && ui.small_button("Unpin").clicked() {
                command(s, Command::Unpin);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Delete node").on_hover_text("Delete").clicked() {
                command(s, Command::Delete);
            }
            let inner = way
                .and_then(|w| s.doc.way(w))
                .is_some_and(|w| w.refs[1..w.refs.len().saturating_sub(1)].contains(&id));
            if ui
                .add_enabled(inner, egui::Button::new("Split way here"))
                .on_hover_text("X")
                .clicked()
            {
                command(s, Command::Split);
            }
        });
        tag_table(ui, s, false, id, None);
    }
}

/// The element's tags as rows of key and value fields, a menu of values for keys that
/// have them, and a row to add one.
fn tag_table(ui: &mut egui::Ui, s: &mut State, is_way: bool, id: i64, class: Option<Class>) {
    let tags = if is_way {
        s.doc.way(id).map(|w| w.meta.tags.clone())
    } else {
        s.doc.node(id).map(|n| n.meta.tags.clone())
    }
    .unwrap_or_default();
    let of = Some((id, s.doc.revision));
    let table = &mut s.table.0[is_way as usize];
    if table.of != of {
        table.of = of;
        table.rows = tags.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        table.rows.push(Default::default());
    }
    let mut commit = false;
    egui::Grid::new(("tags", is_way))
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            let mut remove = None;
            for (i, (key, value)) in table.rows.iter_mut().enumerate() {
                let k = ui.add(
                    egui::TextEdit::singleline(key)
                        .desired_width(120.0)
                        .hint_text("key"),
                );
                let v = ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(110.0)
                        .hint_text("value"),
                );
                commit |= k.lost_focus() || v.lost_focus();
                ui.horizontal(|ui| {
                    if let Some((_, options)) = VALUES.iter().find(|(k, _)| k == key) {
                        ui.menu_button("▾", |ui| {
                            for option in *options {
                                if ui.button(*option).clicked() {
                                    *value = option.to_string();
                                    commit = true;
                                    ui.close();
                                }
                            }
                        });
                    }
                    if !key.is_empty() && ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
                ui.end_row();
            }
            if let Some(i) = remove {
                table.rows.remove(i);
                commit = true;
            }
        });
    ui.horizontal_wrapped(|ui| {
        ui.small("Add:");
        for key in suggestions(class) {
            if !table.rows.iter().any(|(k, _)| k == key) && ui.small_button(*key).clicked() {
                let at = table.rows.len().saturating_sub(1);
                table.rows.insert(at, (key.to_string(), String::new()));
            }
        }
    });
    if commit {
        let new: Tags = table
            .rows
            .iter()
            .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        if new != tags {
            s.doc.edit("tags", |e| {
                if is_way {
                    e.set_way_tags(id, new);
                } else {
                    e.set_node_tags(id, new);
                }
            });
        }
        // Read back (sorted, empty rows gone, a fresh row at the end).
        s.table.0[is_way as usize].of = None;
    }
}

fn fly_to(s: &mut State, p: Vec2) {
    let target = p.extend(s.terrain.sample(p.x, p.y).0).xzy();
    for (mut camera, mut transform) in &mut s.cameras {
        look_at(&mut camera, &mut transform, target, 40.0);
    }
}

fn conflicts(ui: &mut egui::Ui, s: &mut State) {
    let count = s.conflicts.list.len();
    ui.horizontal(|ui| {
        ui.strong("Conflicts");
        ui.label(format!("{count} from the last bake"));
        ui.checkbox(&mut s.conflicts.shown, "show");
    });
    if count == 0 {
        return;
    }
    let mut step = None;
    ui.horizontal(|ui| {
        if ui.button("◀ Previous").clicked() {
            step = Some(false);
        }
        if ui.button("Next ▶").clicked() {
            step = Some(true);
        }
        if let Some(i) = s.conflicts.current {
            ui.label(format!("{} of {count}", i + 1));
        }
    });
    if let Some(forward) = step
        && let Some(at) = s.conflicts.step(forward).map(|c| c.at)
    {
        fly_to(s, at);
    }
    if let Some(c) = s.conflicts.current.and_then(|i| s.conflicts.list.get(i)) {
        ui.label(format!("{}: {}", c.name, c.desc));
    }
}

fn layers(ui: &mut egui::Ui, s: &mut State) {
    egui::CollapsingHeader::new("Show")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for class in Class::ALL {
                    let shown = s.layers.shown.entry(class).or_insert(true);
                    ui.checkbox(shown, class.name());
                }
                ui.checkbox(&mut s.layers.points, "Points");
            });
            ui.add(egui::Slider::new(&mut s.layers.radius, 50.0..=1500.0).text("m around"));
        });
    if let Ok((camera, transform)) = s.cameras.single() {
        let p = transform.translation;
        ui.small(format!(
            "Camera at x {:.0}, z {:.0}, {:.0} m up, {:.0} m/s",
            p.x,
            p.z,
            p.y - s.terrain.sample(p.x, p.z).0,
            camera.speed()
        ));
    }
}

fn help(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("Controls")
        .default_open(false)
        .show(ui, |ui| {
            for line in [
                "Right mouse button held: look around",
                "WASD: move, Q/E: down/up, Shift: faster",
                "Mouse wheel: flying speed",
                "M: map (click to fly there)",
                "1–7: tools, Esc: back to Select / cancel",
                "Delete: delete the selected node or way",
                "X: split the way at the selected node",
                "Page Up/Down: height pin ±10 cm (Shift: 1 m)",
                "Ctrl+Z / Ctrl+Y: undo / redo, Ctrl+S: save",
                "F5: save, bake and reload",
            ] {
                ui.small(line);
            }
        });
}
