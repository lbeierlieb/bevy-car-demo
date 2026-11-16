//! The ECU tuning window (F3) for the active car: its fuel, target λ and spark maps as
//! grids to edit, with the engine's operating point on them, live readings to tune by,
//! the rev limiter and idle settings, and saving and loading tunes.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::table::Table;
use crate::tunes;

/// Grid cell size and the width of the row labels (points).
const CELL: egui::Vec2 = egui::Vec2::new(40.0, 18.0);
const LABEL_WIDTH: f32 = 56.0;
/// Mixture this far from its target shows as off.
const LAMBDA_OFF: f32 = 0.03;

pub struct EcuUiPlugin;

impl Plugin for EcuUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EcuPanel>()
            .add_systems(Update, toggle)
            .add_systems(
                EguiPrimaryContextPass,
                window.run_if(|panel: Res<EcuPanel>| panel.open),
            );
    }
}

#[derive(Resource, Default)]
struct EcuPanel {
    open: bool,
    map: MapKind,
    /// Selected cells: the first clicked and the last (shift-)clicked, as (rpm, manifold
    /// pressure) indices.
    selection: Option<[(usize, usize); 2]>,
    /// Value for "Set", as shown.
    value: f32,
    /// Name to save the tune under, the saved tunes, the one picked from them, whether
    /// "Delete" was clicked once already, and what the last save or load did.
    name: String,
    saved: Vec<String>,
    chosen: Option<String>,
    deleting: bool,
    status: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum MapKind {
    #[default]
    Fuel,
    Lambda,
    Spark,
    IntakeCam,
    ExhaustCam,
}

impl MapKind {
    const ALL: [Self; 5] = [
        Self::Fuel,
        Self::Lambda,
        Self::Spark,
        Self::IntakeCam,
        Self::ExhaustCam,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Fuel => "Fuel (VE %)",
            Self::Lambda => "Target λ",
            Self::Spark => "Spark °",
            Self::IntakeCam => "Intake cam °",
            Self::ExhaustCam => "Exhaust cam °",
        }
    }

    /// The ECU has this map: the cam maps need cam control.
    fn available(self, model: EcuModel) -> bool {
        !matches!(self, Self::IntakeCam | Self::ExhaustCam) || model.cam_control()
    }

    fn table(self, ecu: &mut Ecu) -> &mut Table {
        match self {
            Self::Fuel => &mut ecu.ve,
            Self::Lambda => &mut ecu.lambda,
            Self::Spark => &mut ecu.spark,
            Self::IntakeCam => &mut ecu.cams.intake_phase,
            Self::ExhaustCam => &mut ecu.cams.exhaust_phase,
        }
    }

    /// Shown value per stored value: VE shows in percent.
    fn scale(self) -> f32 {
        if self == Self::Fuel { 100.0 } else { 1.0 }
    }

    /// One step of the +/- buttons, as shown.
    fn step(self) -> f32 {
        match self {
            Self::Fuel => 1.0,
            Self::Lambda => 0.01,
            Self::Spark => 0.5,
            Self::IntakeCam | Self::ExhaustCam => 1.0,
        }
    }

    fn decimals(self) -> usize {
        match self {
            Self::Fuel => 0,
            Self::Lambda => 2,
            Self::Spark => 1,
            Self::IntakeCam | Self::ExhaustCam => 0,
        }
    }

    /// Sensible range of stored values.
    fn range(self) -> (f32, f32) {
        match self {
            Self::Fuel => (0.05, 2.0),
            Self::Lambda => (0.6, 1.5),
            Self::Spark => (-10.0, 60.0),
            Self::IntakeCam | Self::ExhaustCam => (0.0, 60.0),
        }
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut panel: ResMut<EcuPanel>) {
    if keys.just_pressed(KeyCode::F3) {
        panel.open = !panel.open;
        if panel.open {
            panel.saved = tunes::list();
        }
    }
}

fn window(
    mut contexts: EguiContexts,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut panel: ResMut<EcuPanel>,
) -> Result {
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let mut open = panel.open;
    egui::Window::new("ECU tune")
        .open(&mut open)
        .default_pos([360.0, 10.0])
        .default_width(LABEL_WIDTH + 19.0 * CELL.x + 20.0)
        .show(contexts.ctx_mut()?, |ui| {
            readings(ui, &car);
            ui.separator();
            settings(ui, &mut car);
            saved_tunes(ui, &mut panel, &mut car);
            ui.separator();
            map_editor(ui, &mut panel, &mut car);
        });
    panel.open = open;
    Ok(())
}

/// What the engine does right now, against what the ECU aims for.
fn readings(ui: &mut egui::Ui, car: &Car) {
    let (drivetrain, build) = (&car.drivetrain, &car.setup.engine.build);
    let engine = &drivetrain.engine;
    let (rpm, map) = (drivetrain.rpm, engine.intake_pressure);
    let cycle = engine.cycle(build, rpm);
    let target = build.ecu.lambda.get(&[rpm, map]);
    ui.label(format!(
        "{rpm:.0} rpm    {:.0} kPa    intake {:.0} °C",
        map / 1000.0,
        engine.charge_temperature - 273.15,
    ));
    ui.horizontal(|ui| {
        if cycle.lambda.is_finite() {
            let off = (cycle.lambda - target).abs() > LAMBDA_OFF;
            let text = format!("λ {:.2} (target {target:.2})", cycle.lambda);
            let color = if off {
                Color32::from_rgb(255, 170, 60)
            } else {
                ui.visuals().text_color()
            };
            ui.colored_label(color, text);
        } else {
            ui.label("λ - (no fuel)");
        }
        ui.label(format!(
            "    spark {:.1}°    knock {:.1}°    wear {:.0} %",
            engine.actuators.spark,
            cycle.knock,
            engine.damage.worst(build.hardware.cylinders as usize) * 100.0,
        ));
    });
    if build.ecu.model.cam_control() {
        let actuators = &engine.actuators;
        let profile = if actuators.cam_profile == 0 {
            "first"
        } else {
            "second"
        };
        ui.label(format!(
            "cams on the {profile} profile    intake advanced {:.0}°    exhaust retarded {:.0}°",
            actuators.intake_phase, actuators.exhaust_phase,
        ));
    }
}

/// Rev limiter, idle speed, and going back to the factory tune.
fn settings(ui: &mut egui::Ui, car: &mut Car) {
    let build = &mut car.setup.engine.build;
    let ecu = &mut build.ecu;
    ui.horizontal(|ui| {
        ui.label("Rev limit");
        ui.add(
            egui::DragValue::new(&mut ecu.rev_limit)
                .range(3000.0..=9500.0)
                .speed(10.0)
                .suffix(" rpm"),
        );
        ui.label("Idle");
        ui.add(
            egui::DragValue::new(&mut ecu.idle_rpm)
                .range(600.0..=1500.0)
                .speed(5.0)
                .suffix(" rpm"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Limiter: cut");
        let mut cut = ecu.limiter_cut * 1000.0;
        ui.add(
            egui::DragValue::new(&mut cut)
                .range(10.0..=300.0)
                .suffix(" ms"),
        );
        ecu.limiter_cut = cut / 1000.0;
        ui.label("then fuel");
        let mut fuel = ecu.limiter_fuel * 1000.0;
        ui.add(
            egui::DragValue::new(&mut fuel)
                .range(10.0..=300.0)
                .suffix(" ms"),
        );
        ecu.limiter_fuel = fuel / 1000.0;
        if ui
            .button("Factory tune")
            .on_hover_text("Back to the tune calibrated for this engine on 95 octane")
            .clicked()
        {
            *ecu = Ecu::calibrated(&build.hardware, &build.maps, 95.0, ecu.model);
        }
    });
    if ecu.model.cam_control() {
        let cams = &mut ecu.cams;
        ui.horizontal(|ui| {
            ui.label("Second cam profile above");
            ui.add(
                egui::DragValue::new(&mut cams.switch_rpm)
                    .range(1000.0..=9500.0)
                    .speed(10.0)
                    .suffix(" rpm"),
            );
            ui.label("and");
            let mut map = cams.switch_map / 1000.0;
            ui.add(
                egui::DragValue::new(&mut map)
                    .range(10.0..=300.0)
                    .suffix(" kPa"),
            );
            cams.switch_map = map * 1000.0;
        })
        .response
        .on_hover_text("Only for switching cams: with fixed cams there is no second profile");
    }
}

/// Saving the tune under a name, and loading or deleting saved ones.
fn saved_tunes(ui: &mut egui::Ui, panel: &mut EcuPanel, car: &mut Car) {
    let build = &mut car.setup.engine.build;
    ui.horizontal(|ui| {
        ui.label("Tune");
        let name = egui::TextEdit::singleline(&mut panel.name)
            .hint_text("name")
            .desired_width(140.0);
        ui.add(name);
        if ui.button("Save").clicked() {
            panel.status = match tunes::clean_name(&panel.name) {
                None => "Give the tune a name first.".to_string(),
                Some(name) => match tunes::save(&name, &build.ecu.to_text(&build.hardware.name)) {
                    Ok(()) => {
                        panel.saved = tunes::list();
                        panel.chosen = Some(name.clone());
                        panel.name = name.clone();
                        format!("Saved \"{name}\".")
                    }
                    Err(err) => format!("Couldn't save: {err}"),
                },
            };
        }
        ui.separator();
        let shown = panel.chosen.as_deref().unwrap_or("Saved tunes");
        egui::ComboBox::from_id_salt("saved tunes")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for name in &panel.saved {
                    ui.selectable_value(&mut panel.chosen, Some(name.clone()), name);
                }
            });
        let Some(chosen) = panel.chosen.clone() else {
            return;
        };
        if ui.button("Load").clicked() {
            panel.deleting = false;
            panel.status = match tunes::load(&chosen) {
                None => format!("Couldn't read \"{chosen}\"."),
                Some(text) => {
                    // Over this engine's factory tune: anything the file lacks comes from it.
                    let model = build.ecu.model;
                    let mut ecu = Ecu::calibrated(&build.hardware, &build.maps, 95.0, model);
                    let origin = ecu.read_text(&text);
                    build.ecu = ecu;
                    panel.name = chosen.clone();
                    let mut status = format!("Loaded \"{chosen}\".");
                    if let Some(engine) = origin.engine.filter(|e| *e != build.hardware.name) {
                        status += &format!(" Made for the {engine}: watch λ and knock.");
                    }
                    if let Some(made) = origin.ecu.filter(|&m| m != model) {
                        status += &format!(
                            " Made for the {} ECU: this one takes what it can use.",
                            made.name().to_lowercase()
                        );
                    }
                    status
                }
            };
        }
        let delete = if panel.deleting {
            "Really delete?"
        } else {
            "Delete"
        };
        if ui.button(delete).clicked() {
            if panel.deleting {
                tunes::delete(&chosen);
                panel.saved = tunes::list();
                panel.chosen = None;
                panel.status = format!("Deleted \"{chosen}\".");
            }
            panel.deleting = !panel.deleting;
        }
    });
    if !panel.status.is_empty() {
        ui.label(&panel.status);
    }
}

/// Map choice, editing tools, and the grid.
fn map_editor(ui: &mut egui::Ui, panel: &mut EcuPanel, car: &mut Car) {
    let model = car.setup.engine.build.ecu.model;
    if !panel.map.available(model) {
        panel.map = MapKind::Fuel;
    }
    ui.horizontal(|ui| {
        for kind in MapKind::ALL
            .into_iter()
            .filter(|kind| kind.available(model))
        {
            ui.selectable_value(&mut panel.map, kind, kind.name());
        }
    });
    let kind = panel.map;
    let (rpm, map) = (car.drivetrain.rpm, car.drivetrain.engine.intake_pressure);
    let lambda = car
        .drivetrain
        .engine
        .cycle(&car.setup.engine.build, rpm)
        .lambda;
    let ecu = &mut car.setup.engine.build.ecu;
    let target = ecu.lambda.get(&[rpm, map]);
    let table = kind.table(ecu);
    let (low, high) = kind.range();

    ui.horizontal(|ui| {
        let selected = panel.selection;
        let mut apply = |f: &dyn Fn(f32) -> f32| {
            if let Some(selection) = selected {
                for cell in cells(selection) {
                    let value = f(table.cell(&cell) * kind.scale()) / kind.scale();
                    table.set_cell(&cell, value.clamp(low, high));
                }
            }
        };
        ui.add_enabled_ui(selected.is_some(), |ui| {
            let step = kind.step();
            if ui.button(format!("−{step}")).clicked() {
                apply(&|v| v - step);
            }
            if ui.button(format!("+{step}")).clicked() {
                apply(&|v| v + step);
            }
            ui.add(
                egui::DragValue::new(&mut panel.value)
                    .speed(step)
                    .max_decimals(kind.decimals()),
            );
            let value = panel.value;
            if ui.button("Set").clicked() {
                apply(&|_| value);
            }
        });
        if kind == MapKind::Fuel {
            let measured = lambda.is_finite();
            let button = ui
                .add_enabled(measured, egui::Button::new("Correct VE here"))
                .on_hover_text(
                    "Scales the cells around the operating point by measured over target λ, \
                     so the mixture comes out on target",
                );
            if button.clicked() {
                table.scale_at(&[rpm, map], lambda / target);
            }
        }
    });
    ui.label(
        "Click a cell to select it, shift-click to select a block; double-click picks its value.",
    );
    let operating = [rpm, map];
    if let Some((cell, shift, double)) = grid(ui, panel, table, kind, operating) {
        panel.selection = match panel.selection {
            Some([anchor, _]) if shift => Some([anchor, cell]),
            _ => Some([cell, cell]),
        };
        if double {
            panel.value = table.cell(&[cell.0, cell.1]) * kind.scale();
        }
    }
}

/// The cells of a selection, as [rpm, manifold pressure] indices.
fn cells([a, b]: [(usize, usize); 2]) -> impl Iterator<Item = [usize; 2]> {
    let rows = a.1.min(b.1)..=a.1.max(b.1);
    (a.0.min(b.0)..=a.0.max(b.0)).flat_map(move |i| rows.clone().map(move |j| [i, j]))
}

fn selected(selection: Option<[(usize, usize); 2]>, cell: [usize; 2]) -> bool {
    selection.is_some_and(|[a, b]| {
        (a.0.min(b.0)..=a.0.max(b.0)).contains(&cell[0])
            && (a.1.min(b.1)..=a.1.max(b.1)).contains(&cell[1])
    })
}

/// Draws the table (rpm across, manifold pressure up) coloured by value, with the selection
/// and the operating point, and returns a clicked cell (with whether shift was held, and
/// whether it was a double click).
fn grid(
    ui: &mut egui::Ui,
    panel: &EcuPanel,
    table: &Table,
    kind: MapKind,
    operating: [f32; 2],
) -> Option<((usize, usize), bool, bool)> {
    let (rpms, maps) = (table.axis(0), table.axis(1));
    let values = || (0..rpms.len()).flat_map(|i| (0..maps.len()).map(move |j| table.cell(&[i, j])));
    let low = values().fold(f32::INFINITY, f32::min);
    let high = values().fold(f32::NEG_INFINITY, f32::max);
    let mut clicked = None;
    egui::ScrollArea::both().max_height(560.0).show(ui, |ui| {
        let size = vec2(
            LABEL_WIDTH + CELL.x * rpms.len() as f32,
            CELL.y * (maps.len() + 1) as f32,
        );
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);
        let font = FontId::monospace(11.0);
        let text_color = ui.visuals().text_color();
        let origin = rect.min + vec2(LABEL_WIDTH, CELL.y);
        for (i, rpm) in rpms.iter().enumerate() {
            let at = origin + vec2((i as f32 + 0.5) * CELL.x, -CELL.y / 2.0);
            painter.text(
                at,
                Align2::CENTER_CENTER,
                format!("{:.1}k", rpm / 1000.0),
                font.clone(),
                text_color,
            );
        }
        for (row, j) in (0..maps.len()).rev().enumerate() {
            let y = origin.y + row as f32 * CELL.y;
            let label = pos2(rect.min.x + LABEL_WIDTH - 6.0, y + CELL.y / 2.0);
            painter.text(
                label,
                Align2::RIGHT_CENTER,
                format!("{:.0} kPa", maps[j] / 1000.0),
                font.clone(),
                text_color,
            );
            for i in 0..rpms.len() {
                let cell = Rect::from_min_size(pos2(origin.x + i as f32 * CELL.x, y), CELL);
                let value = table.cell(&[i, j]);
                let t = if high > low {
                    (value - low) / (high - low)
                } else {
                    0.5
                };
                painter.rect_filled(cell.shrink(0.5), 2.0, heat(t));
                let shown = format!("{:.*}", kind.decimals(), value * kind.scale());
                painter.text(
                    cell.center(),
                    Align2::CENTER_CENTER,
                    shown,
                    font.clone(),
                    Color32::WHITE,
                );
                if selected(panel.selection, [i, j]) {
                    painter.rect_stroke(
                        cell.shrink(1.0),
                        2.0,
                        Stroke::new(2.0, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                let response = ui.interact(cell, ui.id().with(("ecu cell", i, j)), Sense::click());
                if response.clicked() || response.double_clicked() {
                    let shift = ui.input(|input| input.modifiers.shift);
                    clicked = Some(((i, j), shift, response.double_clicked()));
                }
            }
        }
        // The operating point, where the lookup blends the cells.
        let x = origin.x + (fraction(rpms, operating[0]) + 0.5) * CELL.x;
        let y = origin.y + (maps.len() as f32 - 0.5 - fraction(maps, operating[1])) * CELL.y;
        painter.circle(
            pos2(x, y),
            5.0,
            Color32::from_black_alpha(80),
            Stroke::new(2.0, Color32::YELLOW),
        );
    });
    clicked
}

/// Position of `x` along an axis in grid steps (0 at the first point), clamped to it.
fn fraction(axis: &[f32], x: f32) -> f32 {
    let i = axis
        .partition_point(|&a| a <= x)
        .clamp(1, axis.len().max(2) - 1)
        - 1;
    match axis.get(i + 1) {
        Some(&next) => i as f32 + ((x - axis[i]) / (next - axis[i])).clamp(0.0, 1.0),
        None => 0.0,
    }
}

/// Cell colour from low (blue) through green to high (red).
fn heat(t: f32) -> Color32 {
    let stops = [
        (40.0, 70.0, 150.0),
        (40.0, 130.0, 70.0),
        (190.0, 70.0, 40.0),
    ];
    let t = t.clamp(0.0, 1.0) * 2.0;
    let (a, b, t) = if t < 1.0 {
        (stops[0], stops[1], t)
    } else {
        (stops[1], stops[2], t - 1.0)
    };
    let mix = |a: f32, b: f32| (a + (b - a) * t) as u8;
    Color32::from_rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_cover_their_block() {
        let block: Vec<_> = cells([(3, 1), (2, 2)]).collect();
        assert_eq!(block, vec![[2, 1], [2, 2], [3, 1], [3, 2]]);
        assert!(selected(Some([(3, 1), (2, 2)]), [2, 2]));
        assert!(!selected(Some([(3, 1), (2, 2)]), [1, 2]));
    }

    #[test]
    fn operating_point_lies_between_cells() {
        let axis = [0.0, 500.0, 1000.0];
        assert_eq!(fraction(&axis, 750.0), 1.5);
        assert_eq!(fraction(&axis, -10.0), 0.0);
        assert_eq!(fraction(&axis, 5000.0), 2.0);
    }
}
