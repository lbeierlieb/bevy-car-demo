//! The race editor's window (top right): the tool, loop or sprint, what the server would
//! say, the grid spots and checkpoints, undo, Open and Export (each a dialog asking for
//! the file's name, in the working directory), and the controls.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use super::edit::{AimAt, Mode, Selected, Target, Tool, remove};
use super::show::CarBox;
use super::{Draft, file_path};
use crate::events::race;
use crate::map::MapView;

pub struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dialog>()
            .add_systems(EguiPrimaryContextPass, panel);
    }
}

/// A dialog open over the window.
#[derive(Resource, Default)]
enum Dialog {
    #[default]
    None,
    Open {
        file: String,
    },
    Export {
        file: String,
        name: String,
        /// The file is there already: asked whether to write over it.
        overwrite: bool,
    },
}

const RED: egui::Color32 = egui::Color32::from_rgb(255, 110, 100);
const GREEN: egui::Color32 = egui::Color32::from_rgb(120, 230, 120);

#[allow(clippy::too_many_arguments)]
fn panel(
    mut contexts: EguiContexts,
    mut draft: ResMut<Draft>,
    (mut tool, mut selected, mode): (ResMut<Tool>, ResMut<Selected>, Res<Mode>),
    mut dialog: ResMut<Dialog>,
    mut aim: ResMut<AimAt>,
    view: Res<MapView>,
    car: Res<CarBox>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if view.open {
        return Ok(());
    }
    dialogs(ctx, &mut dialog, &mut draft, &mut aim);
    let idle = matches!(*mode, Mode::Idle) && matches!(*dialog, Dialog::None);
    let height = ctx.content_rect().height() - 60.0;
    // An area sized by its content each frame (as the map editor's).
    egui::Area::new(egui::Id::new("race editor"))
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.heading("Race editor");
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .min_scrolled_height(height.min(560.0))
                    .show(ui, |ui| {
                        ui.set_width(330.0);
                        ui.add_enabled_ui(idle, |ui| {
                            file(ui, &mut draft, &mut dialog);
                            ui.separator();
                            route(ui, &mut draft, &mut tool, &car);
                            ui.separator();
                            grid(ui, &mut draft, &mut selected, &mut aim);
                            ui.separator();
                            checkpoints(ui, &mut draft, &mut aim);
                        });
                        ui.separator();
                        help(ui);
                    });
            });
        });
    Ok(())
}

fn file(ui: &mut egui::Ui, draft: &mut Draft, dialog: &mut Dialog) {
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut draft.name);
    });
    ui.horizontal(|ui| {
        if ui.button("Open…").clicked() {
            *dialog = Dialog::Open {
                file: draft.file.clone(),
            };
        }
        if ui.button("Export…").clicked() {
            let file = if draft.file.is_empty() {
                file_name(&draft.name)
            } else {
                draft.file.clone()
            };
            *dialog = Dialog::Export {
                file,
                name: draft.name.clone(),
                overwrite: false,
            };
        }
        ui.separator();
        if ui
            .add_enabled(draft.can_undo(), egui::Button::new("Undo"))
            .clicked()
        {
            draft.undo();
        }
        if ui
            .add_enabled(draft.can_redo(), egui::Button::new("Redo"))
            .clicked()
        {
            draft.redo();
        }
    });
    if let Some((note, bad)) = &draft.note {
        let color = if *bad { RED } else { GREEN };
        ui.colored_label(color, note);
    }
}

/// A file name for a route's name: its letters and digits, the rest as dashes.
fn file_name(name: &str) -> String {
    let mut file: String = (name.trim().chars())
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while file.contains("--") {
        file = file.replace("--", "-");
    }
    let file = file.trim_matches('-');
    if file.is_empty() {
        "route.cfg".into()
    } else {
        format!("{file}.cfg")
    }
}

fn route(ui: &mut egui::Ui, draft: &mut Draft, tool: &mut Tool, car: &CarBox) {
    ui.horizontal(|ui| {
        ui.label("Click places");
        ui.selectable_value(tool, Tool::Grid, "Grid spots (G)");
        ui.selectable_value(tool, Tool::Checkpoints, "Checkpoints (C)");
    });
    let mut looped = draft.plan.looped;
    let can_loop = draft.plan.checkpoints.len() >= 2;
    ui.add_enabled_ui(can_loop, |ui| {
        ui.checkbox(&mut looped, "Loop (laps end at the first checkpoint)");
    });
    if looped != draft.plan.looped {
        draft.change(|plan| plan.looped = looped);
    }
    let route = &draft.route;
    let kind = if draft.plan.looped && can_loop {
        "Loop"
    } else {
        "Sprint"
    };
    let length = if route.line.len() >= 2 {
        format!(", {:.2} km", route.length() / 1000.0)
    } else {
        String::new()
    };
    ui.label(format!(
        "{kind}{length}: {} checkpoints, {} on the grid",
        draft.plan.checkpoints.len(),
        draft.plan.grid.len()
    ));
    match &draft.problem {
        Some(problem) => {
            ui.colored_label(RED, format!("The server would refuse it: {problem}."));
        }
        None => {
            ui.colored_label(GREEN, "The server would take it.");
        }
    }
    let grid = &draft.plan.grid;
    let overlapping = (0..grid.len())
        .filter(|&i| (0..grid.len()).any(|j| j != i && car.overlap(&grid[i], &grid[j])))
        .count();
    if overlapping > 0 {
        ui.colored_label(
            RED,
            format!(
                "{overlapping} grid spots overlap (red boxes): cars would start inside each other."
            ),
        );
    }
}

fn grid(ui: &mut egui::Ui, draft: &mut Draft, selected: &mut Selected, aim: &mut AimAt) {
    egui::CollapsingHeader::new(format!(
        "Grid ({} of {})",
        draft.plan.grid.len(),
        race::MAX_RACERS
    ))
    .default_open(true)
    .show(ui, |ui| {
        if draft.plan.grid.is_empty() {
            ui.label("None yet: pick \"Grid spots\" and click on the road.");
        }
        let mut gone = None;
        for (n, i) in draft.pole_first().into_iter().enumerate() {
            let spot = draft.plan.grid[i];
            ui.horizontal(|ui| {
                let label = format!(
                    "P{}  {:.1} {:.1}  {:.0}°",
                    n + 1,
                    spot.at.x,
                    spot.at.y,
                    spot.heading
                );
                if ui.selectable_label(selected.0 == Some(i), label).clicked() {
                    selected.0 = Some(i);
                    aim.0 = Some(spot.at);
                }
                if ui.small_button("Remove").clicked() {
                    gone = Some(i);
                }
            });
        }
        if let Some(i) = gone {
            remove(draft, Target::Spot(i));
            selected.0 = None;
        }
        if let Some(i) = selected.0.filter(|&i| i < draft.plan.grid.len()) {
            let mut heading = draft.plan.grid[i].heading;
            ui.horizontal(|ui| {
                ui.label("Selected spot faces");
                ui.add(
                    egui::DragValue::new(&mut heading)
                        .range(0.0..=359.9)
                        .speed(0.5)
                        .max_decimals(1)
                        .suffix("°"),
                );
            });
            if heading != draft.plan.grid[i].heading {
                draft.change_merged((1, i), |plan| plan.grid[i].heading = heading);
            }
        }
    });
}

fn checkpoints(ui: &mut egui::Ui, draft: &mut Draft, aim: &mut AimAt) {
    egui::CollapsingHeader::new(format!("Checkpoints ({})", draft.plan.checkpoints.len()))
        .default_open(true)
        .show(ui, |ui| {
            if draft.plan.checkpoints.is_empty() {
                ui.label("None yet: pick \"Checkpoints\" and click on the road.");
            }
            let n = draft.plan.checkpoints.len();
            let looped = draft.plan.looped && n >= 2;
            let mut gone = None;
            for (i, p) in draft.plan.checkpoints.iter().enumerate() {
                let what = match i {
                    0 if looped => "  start/finish",
                    0 => "  start",
                    _ if i == n - 1 && !looped && n >= 2 => "  finish",
                    _ => "",
                };
                ui.horizontal(|ui| {
                    let label = format!("{}  {:.1} {:.1}{what}", i + 1, p.x, p.y);
                    if ui.selectable_label(false, label).clicked() {
                        aim.0 = Some(*p);
                    }
                    if ui.small_button("Remove").clicked() {
                        gone = Some(i);
                    }
                });
            }
            if let Some(i) = gone {
                remove(draft, Target::Check(i));
            }
        });
}

fn help(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("Controls")
        .default_open(false)
        .show(ui, |ui| {
            for line in [
                "Right mouse button held: look around",
                "W A S D: move, Q / E: down / up, Shift: faster",
                "Mouse wheel: flying speed",
                "M: the map (a click there flies to it)",
                "G: grid spots, C: checkpoints",
                "Click on the ground: place one (a grid spot's box then turns with the \
                 mouse: click again to keep it, Esc to call it off)",
                "Drag a disc: move it (the gates follow)",
                "Click a grid spot's disc or box: turn it again",
                "Shift + mouse wheel: turn the selected grid spot by a degree",
                "Click the first checkpoint (green): make the route a loop",
                "Ctrl + click: a checkpoint between the two around that place",
                "Delete: remove what's under the pointer (or the selected spot)",
                "Ctrl+Z / Ctrl+Y: undo / redo",
                "Pole is the grid spot nearest the first checkpoint; every checkpoint is a \
                 gate. The route goes the shortest way along the roads between them: \
                 add a checkpoint where it should go another way.",
            ] {
                ui.label(line);
            }
        });
}

fn dialogs(ctx: &egui::Context, dialog: &mut Dialog, draft: &mut Draft, aim: &mut AimAt) {
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let folder = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let mut close = escape;
    match dialog {
        Dialog::None => {}
        Dialog::Open { file } => {
            egui::Window::new("Open a route")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("A file in {folder}:"));
                    let field = ui.text_edit_singleline(file);
                    field.request_focus();
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button("Open").clicked() || enter {
                            let path = file_path(file);
                            match draft.open(&path) {
                                Ok(()) => {
                                    aim.0 = draft.plan.checkpoints.first().copied();
                                    close = true;
                                }
                                Err(e) => {
                                    draft.note = Some((
                                        format!("Couldn't open {}: {e}", path.display()),
                                        true,
                                    ))
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if let Some((note, true)) = &draft.note {
                        ui.colored_label(RED, note);
                    }
                });
        }
        Dialog::Export {
            file,
            name,
            overwrite,
        } => {
            egui::Window::new("Export the route")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("File name (in {folder}):"));
                    if ui.text_edit_singleline(file).changed() {
                        *overwrite = false;
                    }
                    ui.label("The route's name (as racers see it):");
                    ui.text_edit_singleline(name);
                    if let Some(problem) = &draft.problem {
                        ui.colored_label(
                            RED,
                            format!("Mind: the server would refuse it: {problem}."),
                        );
                    }
                    let path = file_path(file);
                    if *overwrite {
                        ui.colored_label(RED, format!("{} is there already.", path.display()));
                    }
                    ui.horizontal(|ui| {
                        let label = if *overwrite {
                            "Write over it"
                        } else {
                            "Export"
                        };
                        let can = !file.trim().is_empty() && !name.trim().is_empty();
                        if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                            if path.exists() && !*overwrite {
                                *overwrite = true;
                            } else {
                                draft.name = name.trim().to_string();
                                match std::fs::write(&path, draft.text()) {
                                    Ok(()) => {
                                        draft.file = file.trim().to_string();
                                        draft.note = Some((
                                            format!("Written to {}.", path.display()),
                                            false,
                                        ));
                                        close = true;
                                    }
                                    Err(e) => {
                                        draft.note = Some((
                                            format!("Couldn't write {}: {e}", path.display()),
                                            true,
                                        ))
                                    }
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
        }
    }
    if close {
        *dialog = Dialog::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routes_name_makes_a_file_name() {
        assert_eq!(file_name("Baunacher Ring"), "baunacher-ring.cfg");
        assert_eq!(file_name("  Nord/Ost – Sprint! "), "nord-ost-sprint.cfg");
        assert_eq!(file_name("§"), "route.cfg");
    }
}
