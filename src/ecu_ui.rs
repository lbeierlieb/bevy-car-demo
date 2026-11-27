//! The ECU tuning, shown in the dyno's room (`dyno`) for the car on the rollers: its fuel,
//! target λ and spark maps (and the cam maps of a programmable ECU) as grids to edit, with
//! the engine's operating point on them, live readings to tune by, the rev limiter and idle
//! settings, and saving and loading tunes.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};

use crate::car::Car;
use crate::menu::kit;
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::table::Table;
use crate::tunes;

/// Grid cell size and the width of the row labels (points).
const CELL: egui::Vec2 = egui::Vec2::new(40.0, 18.0);
const LABEL_WIDTH: f32 = 56.0;
/// Mixture this far from its target shows as off.
const LAMBDA_OFF: f32 = 0.03;

/// What the tuning shows and holds between frames.
#[derive(Default)]
pub struct EcuPanel {
    /// Whether the saved tunes were listed (when it's first shown).
    listed: bool,
    map: MapKind,
    /// Editing the second cam profile's fuel, λ and spark maps.
    high_cam: bool,
    /// Selected cells: the first clicked and the last (shift-)clicked, as (rpm, manifold
    /// pressure) indices.
    selection: Option<[(usize, usize); 2]>,
    /// Working the grid with the arrows: the cell they're on, and where a block being
    /// selected started (choose starts and ends one).
    cursor: (usize, usize),
    anchor: Option<(usize, usize)>,
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

    /// The map, on the second cam profile if `high`.
    fn table(self, ecu: &mut Ecu, high: bool) -> &mut Table {
        match self {
            Self::Fuel if high => &mut ecu.cams.ve,
            Self::Lambda if high => &mut ecu.cams.lambda,
            Self::Spark if high => &mut ecu.cams.spark,
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

/// The ECU tuning in the dyno's room, on the car on the rollers: what the engine does
/// right now against what the ECU aims for, the maps as grids to edit with the operating
/// point on them, the rev limiter and idle settings, and saving and loading tunes.
pub fn tuning(ui: &mut egui::Ui, panel: &mut EcuPanel, car: &mut Car) {
    if !panel.listed {
        panel.saved = tunes::list();
        panel.listed = true;
    }
    readings(ui, car);
    let ecu = car.setup.engine.parts.ecu_def();
    if ecu.model.locked {
        let engine = ecu.factory.as_deref().unwrap_or("engine it was made for");
        kit::note(
            ui,
            &format!(
                "The {} is locked: it runs the factory's maps for the {engine}, whatever \
                 engine it's on. A programmable ECU can be tuned.",
                ecu.name
            ),
        );
        return;
    }
    map_editor(ui, panel, car);
    settings(ui, car);
    saved_tunes(ui, panel, car);
}

/// How wide `tuning` is (points).
pub const WIDTH: f32 = LABEL_WIDTH + 19.0 * CELL.x + 8.0;

/// What the engine does right now, against what the ECU aims for.
fn readings(ui: &mut egui::Ui, car: &Car) {
    let (drivetrain, build) = (&car.drivetrain, &car.setup.engine.build);
    let engine = &drivetrain.engine;
    let (rpm, map) = (drivetrain.rpm, engine.intake_pressure);
    let cycle = engine.cycle(build, rpm);
    let target = (build.ecu.maps_on(engine.actuators.cam_profile).1).get(&[rpm, map]);
    kit::info(
        ui,
        &format!("{rpm:.0} rpm    {:.0} kPa", map / 1000.0),
        &format!("intake {:.0} °C", engine.charge_temperature - 273.15),
    );
    let lambda = if cycle.lambda.is_finite() {
        let off = if (cycle.lambda - target).abs() > LAMBDA_OFF {
            "  off"
        } else {
            ""
        };
        format!("λ {:.2} (target {target:.2}){off}", cycle.lambda)
    } else {
        "λ - (no fuel)".to_string()
    };
    kit::info(
        ui,
        &lambda,
        &format!(
            "spark {:.1}°  knock {:.1}°  wear {:.0} %",
            engine.actuators.spark,
            cycle.knock,
            engine.damage.worst(build.hardware.cylinders as usize) * 100.0,
        ),
    );
    kit::info(ui, "Lambda sensor", &lambda_sensor(car));
    if build.ecu.model.cam_control() {
        let actuators = &engine.actuators;
        let profile = if actuators.cam_profile == 0 {
            "first"
        } else {
            "second"
        };
        kit::info(
            ui,
            &format!("Cams on the {profile} profile"),
            &format!(
                "intake +{:.0}°, exhaust −{:.0}°",
                actuators.intake_phase, actuators.exhaust_phase,
            ),
        );
    }
}

/// What the car's lambda sensor tells its ECU, and the closed loop's fuel trim: a
/// narrowband's volts (rich or lean), a wideband's λ (if the ECU reads one).
pub fn lambda_sensor(car: &Car) -> String {
    let build = &car.setup.engine.build;
    let actuators = &car.drivetrain.engine.actuators;
    let reading = actuators.lambda_sensor;
    let sensor = if build.hardware.wideband && build.ecu.model.wideband {
        format!("wideband λ {reading:.2}")
    } else {
        let side = if reading > 0.45 { "rich" } else { "lean" };
        format!("narrowband {reading:.2} V ({side})")
    };
    format!("{sensor}, trim {:+.0} %", actuators.trim * 100.0)
}

/// Rev limiter, idle speed, and going back to the factory tune.
fn settings(ui: &mut egui::Ui, car: &mut Car) {
    let build = &mut car.setup.engine.build;
    let ecu = &mut build.ecu;
    kit::section(ui, "Limits and idle");
    kit::slider(
        ui,
        "Rev limit",
        &mut ecu.rev_limit,
        3000.0..=9500.0,
        50.0,
        |v| format!("{v:.0} rpm"),
    );
    kit::slider(ui, "Idle", &mut ecu.idle_rpm, 600.0..=1500.0, 10.0, |v| {
        format!("{v:.0} rpm")
    });
    let mut cut = ecu.limiter_cut * 1000.0;
    kit::slider(ui, "Limiter: cut for", &mut cut, 10.0..=300.0, 5.0, |v| {
        format!("{v:.0} ms")
    });
    ecu.limiter_cut = cut / 1000.0;
    let mut fuel = ecu.limiter_fuel * 1000.0;
    kit::slider(
        ui,
        "Limiter: then fuel for",
        &mut fuel,
        10.0..=300.0,
        5.0,
        |v| format!("{v:.0} ms"),
    );
    ecu.limiter_fuel = fuel / 1000.0;
    let mut trim = ecu.trim_limit * 100.0;
    kit::slider(
        ui,
        "Closed loop trims up to",
        &mut trim,
        0.0..=50.0,
        1.0,
        |v| format!("±{v:.0} %"),
    );
    ecu.trim_limit = trim / 100.0;
    if ecu.model.cam_control() {
        let cams = &mut ecu.cams;
        kit::slider(
            ui,
            "Second cam profile above",
            &mut cams.switch_rpm,
            1000.0..=9500.0,
            50.0,
            |v| format!("{v:.0} rpm"),
        );
        let mut map = cams.switch_map / 1000.0;
        kit::slider(ui, "… and above", &mut map, 10.0..=300.0, 5.0, |v| {
            format!("{v:.0} kPa")
        });
        cams.switch_map = map * 1000.0;
        kit::note(
            ui,
            "Only for switching cams: with fixed cams there is no second profile.",
        );
    }
    if kit::button(ui, "Factory tune (for 95 octane)").clicked() {
        *ecu = Ecu::calibrated(&build.hardware, &build.maps, 95.0, ecu.model);
    }
}

/// Saving the tune under a name, and loading or deleting saved ones.
fn saved_tunes(ui: &mut egui::Ui, panel: &mut EcuPanel, car: &mut Car) {
    let ecu_name = &car.setup.engine.parts.ecu_def().name.clone();
    let build = &mut car.setup.engine.build;
    kit::section(ui, "Tunes");
    kit::text_field(ui, &mut panel.name, "Name to save the tune under", 1);
    if kit::button(ui, "Save the tune").clicked() {
        // Without a name typed (a controller), a numbered one.
        let name = if panel.name.trim().is_empty() {
            (1..)
                .map(|n| format!("Tune {n}"))
                .find(|name| !panel.saved.contains(name))
                .expect("a free number")
        } else {
            panel.name.clone()
        };
        panel.status = match tunes::clean_name(&name) {
            None => "That name won't do as a file name.".to_string(),
            Some(name) => {
                match tunes::save(&name, &build.ecu.to_text(&build.hardware.name, ecu_name)) {
                    Ok(()) => {
                        panel.saved = tunes::list();
                        panel.chosen = Some(name.clone());
                        panel.name = name.clone();
                        format!("Saved \"{name}\".")
                    }
                    Err(err) => format!("Couldn't save: {err}"),
                }
            }
        };
    }
    if !panel.saved.is_empty() {
        let mut index = (panel.chosen.as_ref())
            .and_then(|chosen| panel.saved.iter().position(|name| name == chosen))
            .unwrap_or(0);
        kit::choice(ui, "Saved tune", &mut index, &panel.saved);
        panel.chosen = panel.saved.get(index).cloned();
    }
    if let Some(chosen) = panel.chosen.clone() {
        if kit::button(ui, &format!("Load \"{chosen}\"")).clicked() {
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
                    if let Some(made) = origin.ecu.filter(|made| made != ecu_name) {
                        status += &format!(" Made on the {made}: this one takes what it can use.");
                    }
                    status
                }
            };
        }
        let delete = if panel.deleting {
            format!("Really delete \"{chosen}\"?")
        } else {
            format!("Delete \"{chosen}\"")
        };
        if kit::button(ui, &delete).clicked() {
            if panel.deleting {
                tunes::delete(&chosen);
                panel.saved = tunes::list();
                panel.chosen = None;
                panel.status = format!("Deleted \"{chosen}\".");
            }
            panel.deleting = !panel.deleting;
        }
    }
    if !panel.status.is_empty() {
        kit::note(ui, &panel.status);
    }
}

/// Map choice, the grid, and the editing tools under it.
fn map_editor(ui: &mut egui::Ui, panel: &mut EcuPanel, car: &mut Car) {
    let model = car.setup.engine.build.ecu.model;
    if !panel.map.available(model) {
        panel.map = MapKind::Fuel;
    }
    let kinds: Vec<MapKind> = MapKind::ALL
        .into_iter()
        .filter(|kind| kind.available(model))
        .collect();
    kit::pick(ui, "Map", &mut panel.map, &kinds, |kind| {
        kind.name().to_string()
    });
    let kind = panel.map;
    // Switching cams have maps for each profile.
    let switching = model.cam_control() && car.setup.engine.build.hardware.cams.len() > 1;
    let per_cam = switching && !matches!(kind, MapKind::IntakeCam | MapKind::ExhaustCam);
    if per_cam {
        kit::pick(ui, "Cam", &mut panel.high_cam, &[false, true], |high| {
            if high {
                "Second profile"
            } else {
                "First profile"
            }
            .to_string()
        });
    }
    let high_cam = per_cam && panel.high_cam;
    let (rpm, map) = (car.drivetrain.rpm, car.drivetrain.engine.intake_pressure);
    let lambda = car
        .drivetrain
        .engine
        .cycle(&car.setup.engine.build, rpm)
        .lambda;
    let ecu = &mut car.setup.engine.build.ecu;
    let profile = car.drivetrain.engine.actuators.cam_profile;
    let target = ecu.maps_on(profile).1.get(&[rpm, map]);
    let table = kind.table(ecu, high_cam);
    let (low, high) = kind.range();

    let operating = [rpm, map];
    if let Some((cell, shift, double)) = grid(ui, panel, table, kind, operating) {
        panel.selection = match panel.selection {
            Some([anchor, _]) if shift => Some([anchor, cell]),
            _ => Some([cell, cell]),
        };
        panel.cursor = cell;
        panel.anchor = None;
        if double {
            panel.value = table.cell(&[cell.0, cell.1]) * kind.scale();
        }
    }
    kit::note(
        ui,
        "The arrows move over the cells, choose starts and ends a block; the mouse: click a \
         cell, shift-click a block, double-click picks its value. The yellow ring is where \
         the engine runs.",
    );
    let selected = panel.selection;
    let mut apply = |f: &dyn Fn(f32) -> f32| {
        if let Some(selection) = selected {
            for cell in cells(selection) {
                let value = f(table.cell(&cell) * kind.scale()) / kind.scale();
                table.set_cell(&cell, value.clamp(low, high));
            }
        }
    };
    let step = kind.step();
    ui.add_enabled_ui(selected.is_some(), |ui| {
        if kit::button(ui, &format!("Raise the selected cells by {step}")).clicked() {
            apply(&|v| v + step);
        }
        if kit::button(ui, &format!("Lower them by {step}")).clicked() {
            apply(&|v| v - step);
        }
        let (lo, hi) = (low * kind.scale(), high * kind.scale());
        kit::slider(ui, "Value to set", &mut panel.value, lo..=hi, step, |v| {
            format!("{v:.*}", kind.decimals())
        });
        let value = panel.value;
        if kit::button(ui, &format!("Set them to {value:.*}", kind.decimals())).clicked() {
            apply(&|_| value);
        }
    });
    if kind == MapKind::Fuel {
        let measured = lambda.is_finite();
        let correct = ui
            .add_enabled_ui(measured, |ui| {
                kit::button(ui, "Correct VE where the engine runs")
            })
            .inner;
        if correct.clicked() {
            table.scale_at(&[rpm, map], lambda / target);
        }
        kit::note(
            ui,
            "Scales the cells around the operating point by measured over target λ, so the \
             mixture comes out on target.",
        );
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
/// whether it was a double click). One widget for the focus: while it has it, the arrows
/// move the cursor (up from the top row and down from the bottom one leave the grid) and
/// choose starts or ends a block.
fn grid(
    ui: &mut egui::Ui,
    panel: &mut EcuPanel,
    table: &Table,
    kind: MapKind,
    operating: [f32; 2],
) -> Option<((usize, usize), bool, bool)> {
    let (rpms, maps) = (table.axis(0), table.axis(1));
    let values = || (0..rpms.len()).flat_map(|i| (0..maps.len()).map(move |j| table.cell(&[i, j])));
    let low = values().fold(f32::INFINITY, f32::min);
    let high = values().fold(f32::NEG_INFINITY, f32::max);
    let mut clicked = None;
    let size = vec2(
        LABEL_WIDTH + CELL.x * rpms.len() as f32,
        CELL.y * (maps.len() + 1) as f32,
    );
    let (rect, response) = crate::menu::kit::allocate(ui, size, Sense::click());
    let origin = rect.min + vec2(LABEL_WIDTH, CELL.y);
    // The arrows and choose, while it has the focus.
    let (columns, rows) = (rpms.len(), maps.len());
    panel.cursor = (
        panel.cursor.0.min(columns.saturating_sub(1)),
        panel.cursor.1.min(rows.saturating_sub(1)),
    );
    if kit::focused(ui, &response) {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let (up, down, left, right, enter) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space),
            )
        });
        let (mut i, mut j) = panel.cursor;
        if left {
            i = i.saturating_sub(1);
        }
        if right {
            i = (i + 1).min(columns - 1);
        }
        // Up is a higher manifold pressure (the top row).
        if up {
            if j + 1 >= rows {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Up));
            } else {
                j += 1;
            }
        }
        if down {
            if j == 0 {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Down));
            } else {
                j -= 1;
            }
        }
        if enter {
            panel.anchor = match panel.anchor {
                Some(_) => None,
                None => Some((i, j)),
            };
        }
        if (i, j) != panel.cursor || enter || panel.selection.is_none() {
            panel.cursor = (i, j);
            panel.selection = Some([panel.anchor.unwrap_or((i, j)), (i, j)]);
        }
    }
    let painter = ui.painter_at(rect);
    let font = FontId::monospace(11.0);
    let text_color = ui.visuals().text_color();
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
            if kit::focused(ui, &response) && panel.cursor == (i, j) {
                painter.rect_stroke(
                    cell,
                    2.0,
                    Stroke::new(2.0, crate::menu::ACCENT),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.clicked() || response.double_clicked())
    {
        let i = ((pointer.x - origin.x) / CELL.x).floor();
        let row = ((pointer.y - origin.y) / CELL.y).floor();
        if i >= 0.0 && row >= 0.0 && (i as usize) < columns && (row as usize) < rows {
            let j = rows - 1 - row as usize;
            let shift = ui.input(|input| input.modifiers.shift);
            clicked = Some(((i as usize, j), shift, response.double_clicked()));
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
