//! Help for beginners: a screen listing the controls (shown at the start, H brings it
//! back; "back" closes it), and hints over the road when the car won't do what the
//! driver seems to want: what's wrong, what to press, and what that does.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, text::LayoutJob};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car, ReverseRefused, drive};
use crate::input::{Action, WheelConfig, WheelInput};
use crate::menu::kit::{self, Panel, rank};
use crate::menu::nav::{MenuInput, MenuSet};

/// The key that brings the controls screen back.
const KEY: KeyCode = KeyCode::KeyH;

/// The controls of the sandbox, keyboard first: (keys, what they do).
pub const CONTROLS: [(&str, &str); 17] = [
    ("Esc", "Menu: everything else is in it"),
    ("W or Up", "Throttle"),
    ("S or Down", "Brake"),
    ("A D or Left Right", "Steer"),
    ("E / Q", "Shift up / down (Q in neutral, standing: reverse)"),
    ("Space", "Parking brake on / off (on when you start)"),
    ("I", "Hold: starter; press while running: engine off"),
    ("R", "A helicopter carries the car back to the road"),
    ("C", "Chase or driver's view"),
    ("B (hold)", "Look back"),
    ("Left-drag, scroll", "Look around, zoom"),
    ("M", "Map"),
    ("Stop on a marker", "Shops and events (on a server)"),
    ("T", "Chat (on a server): Enter sends, Esc cancels"),
    ("H", "This screen"),
    ("2", "Telemetry"),
    ("1 / F11", "Debug gizmos / fullscreen"),
];

pub struct HelpPlugin;

impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Intro { open: true })
            .init_resource::<Hints>()
            .add_systems(Update, (toggle.in_set(MenuSet::Top), hints.after(drive)))
            .add_systems(EguiPrimaryContextPass, (intro, show_hint));
    }
}

/// Whether the controls screen is shown. The start menu waits for it.
#[derive(Resource)]
pub struct Intro {
    pub open: bool,
}

/// H opens or closes it; open, "back" or the menu button closes it.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut input: ResMut<MenuInput>, mut intro: ResMut<Intro>) {
    if keys.just_pressed(KEY) {
        intro.open ^= true;
    } else if intro.open && (input.take_back() || input.take_open()) {
        intro.open = false;
    }
}

/// The controls screen: the keys, and what else there is.
fn intro(mut contexts: EguiContexts, mut intro: ResMut<Intro>) -> Result {
    if !intro.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let close = Panel::new("intro", "Controls")
        .rank(rank::INTRO)
        .width(620.0)
        .back(Some("Close"))
        .show(ctx, |ui| {
            let drive = kit::button(ui, "Let's drive").clicked();
            for (keys, action) in CONTROLS {
                kit::info(ui, action, keys);
            }
            kit::note(
                ui,
                "You shift gears yourself. The clutch works by itself unless you bind a \
                 clutch pedal. A gamepad works the menus as it is (D-pad, A, B, Start); \
                 steering wheels, pedals, shifters and gamepads are bound in the menu \
                 (Esc), under Controls.",
            );
            drive
        });
    if close {
        intro.open = false;
    }
    Ok(())
}

/// The things the hints are about, most important first: only the first that's due is
/// shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hint {
    Blown,
    NoFuel,
    Stalled,
    Off,
    Neutral,
    ParkingBrake,
    Reverse,
    RevLimit,
    Stuck,
}

impl Hint {
    const ALL: [Hint; 9] = [
        Hint::Blown,
        Hint::NoFuel,
        Hint::Stalled,
        Hint::Off,
        Hint::Neutral,
        Hint::ParkingBrake,
        Hint::Reverse,
        Hint::RevLimit,
        Hint::Stuck,
    ];

    /// How long the situation must last before the hint shows (s).
    fn delay(self) -> f32 {
        match self {
            Hint::Reverse => 0.0,
            Hint::ParkingBrake => 0.4,
            Hint::RevLimit => 0.8,
            Hint::Stuck => 3.0,
            _ => 0.6,
        }
    }

    /// How long the hint stays up after the situation is over (s): a refused shift is
    /// over at once, the rest end when the driver does something about them.
    fn linger(self) -> f32 {
        match self {
            Hint::Reverse => 5.0,
            _ => 1.5,
        }
    }
}

/// How long each situation has lasted (s), and the hint shown with the time it stays up.
#[derive(Resource, Default)]
struct Hints {
    held: [f32; Hint::ALL.len()],
    shown: Option<(Hint, f32)>,
    /// What the hint says, worked out when it's due: (what's wrong, what to do).
    text: (String, String),
}

/// Throttle above which the driver means to go, and speed below which the car stands
/// (m/s).
const GAS: f32 = 0.2;
const STANDING: f32 = 1.0;
/// The rev limiter counts from this far below its limit (rpm): it cuts and lets the revs
/// drop a little each time.
const LIMITER_BAND: f32 = 250.0;

/// Watches the active car for situations a beginner may not understand.
fn hints(
    time: Res<Time>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    config: Res<WheelConfig>,
    wheel: Res<WheelInput>,
    mut refused: MessageReader<ReverseRefused>,
    mut hints: ResMut<Hints>,
) {
    let dt = time.delta_secs();
    let reverse = refused.read().count() > 0;
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let engine = &car.setup.engine;
    let drivetrain = &car.drivetrain;
    let c = &car.controls;
    let gas = c.throttle > GAS;
    let standing = car.forward_speed().abs() < STANDING;
    let gear = drivetrain.selected_gear();
    let blown = drivetrain.engine.blown(&engine.build.hardware);
    let no_fuel = car.fuel.supply < 0.5;
    let running = drivetrain.running;
    let (off, cranking) = (drivetrain.switched_off(), drivetrain.cranking());
    let clutch_in = c.clutch.is_some_and(|travel| travel > 0.5);
    let at_limit = drivetrain.rpm >= engine.build.ecu.rev_limit - LIMITER_BAND;
    let now = |hint: Hint| match hint {
        Hint::Blown => gas && blown,
        Hint::NoFuel => gas && no_fuel && !blown,
        Hint::Stalled => gas && !running && !off && !cranking && !blown && !no_fuel,
        Hint::Off => gas && off && !cranking && !blown,
        // Only once a gear is in: revving in neutral at the start, neutral comes first
        // (asked for by the user).
        Hint::ParkingBrake => gas && car.parking_brake && standing && gear != 0,
        Hint::Reverse => reverse,
        Hint::Neutral => gas && running && gear == 0 && standing,
        // Only in first: past it, the driver knows about shifting (asked for by the user).
        Hint::RevLimit => {
            running && c.throttle > 0.5 && at_limit && gear == 1 && gear < engine.top_gear()
        }
        Hint::Stuck => {
            gas && standing
                && running
                && gear != 0
                && !car.parking_brake
                && !clutch_in
                && c.brake < 0.5
        }
    };
    let mut due = None;
    for (i, hint) in Hint::ALL.into_iter().enumerate() {
        let held = &mut hints.held[i];
        // Falling back twice as fast keeps an on-and-off situation (the limiter) counting.
        *held = if now(hint) {
            *held + dt
        } else {
            (*held - 2.0 * dt).max(0.0)
        };
        if due.is_none() && now(hint) && *held >= hint.delay() {
            due = Some(hint);
        }
    }
    match (due, hints.shown) {
        (Some(hint), shown) => {
            if shown.is_none_or(|(old, _)| old != hint) {
                hints.text = text(hint, &config, &wheel, c.clutch.is_some());
            }
            hints.shown = Some((hint, hint.linger()));
        }
        (None, Some((hint, left))) => {
            hints.shown = (left > dt).then_some((hint, left - dt));
        }
        (None, None) => {}
    }
}

/// What a hint says: what's wrong, and what to press and what that does. Keys are in
/// [brackets], shown highlighted.
fn text(hint: Hint, config: &WheelConfig, wheel: &WheelInput, pedal: bool) -> (String, String) {
    let bound = |action| config.enabled && config.binding(action).is_some();
    // A key, or the key and the wheel's button bound to the same.
    let key = |key: &str, action| {
        if bound(action) {
            format!("[{key}] (or its button on the wheel)")
        } else {
            format!("[{key}]")
        }
    };
    let shifter = wheel.shifter.is_some();
    let clutch_first = if pedal { "press the clutch and " } else { "" };
    let (what, how) = match hint {
        Hint::Blown => (
            "The engine is wrecked".into(),
            "It was pushed past what its parts can take.".into(),
        ),
        Hint::NoFuel => (
            "The tank is empty".into(),
            "Without fuel the engine stops.".into(),
        ),
        Hint::Stalled => (
            "The engine has stalled".into(),
            format!(
                "The clutch dragged it below idle. Hold {} to start it again. To pull \
                 away, give a little gas and let the clutch out slowly; press the clutch \
                 whenever you stop.",
                key("I", Action::Ignition)
            ),
        ),
        Hint::Off => (
            "The engine is off".into(),
            format!(
                "Hold {} until it fires up: the starter turns it over.",
                key("I", Action::Ignition)
            ),
        ),
        Hint::ParkingBrake => (
            "The parking brake is on".into(),
            format!(
                "It holds the car still however much gas you give. Press {} to release \
                 it, and again to hold the car when you've stopped.",
                if wheel.handbrake.is_some() {
                    "[Space]".to_string()
                } else {
                    key("Space", Action::Handbrake)
                }
            ),
        ),
        Hint::Reverse => (
            "Reverse only goes in at a standstill".into(),
            if shifter {
                "Brake to a stop with [S] (or the brake pedal), then move the shifter into \
                 reverse."
                    .into()
            } else {
                format!(
                    "Brake to a stop with [S], then press {} in neutral to shift into \
                     reverse (R). {} shifts back up into neutral and first.",
                    key("Q", Action::ShiftDown),
                    key("E", Action::ShiftUp)
                )
            },
        ),
        Hint::Neutral => (
            "The gearbox is in neutral (N)".into(),
            if shifter {
                format!(
                    "The engine turns, but nothing drives the wheels: {clutch_first}move the \
                     shifter into first gear."
                )
            } else {
                format!(
                    "The engine turns, but nothing drives the wheels. Press {} to shift up \
                     into first gear.",
                    key("E", Action::ShiftUp)
                )
            },
        ),
        Hint::RevLimit => (
            "The engine is at its rev limit".into(),
            format!(
                "The limiter cuts the fuel to keep the engine from breaking, so the car \
                 can't go faster in this gear. {} up a gear: more speed at lower revs.",
                if shifter {
                    if pedal {
                        "Press the clutch and move the shifter".to_string()
                    } else {
                        "Move the shifter".to_string()
                    }
                } else {
                    format!("Press {} to shift", key("E", Action::ShiftUp))
                }
            ),
        ),
        Hint::Stuck => (
            "Stuck?".into(),
            format!(
                "Press {} and a helicopter carries the car back to the road.",
                key("R", Action::Reset)
            ),
        ),
    };
    (what, how)
}

const KEY_COLOR: Color32 = Color32::from_rgb(255, 196, 60);

/// The hint, in a box at the top middle, over the road.
fn show_hint(mut contexts: EguiContexts, hints: Res<Hints>, intro: Res<Intro>) -> Result {
    if hints.shown.is_none() || intro.open {
        return Ok(());
    }
    let (what, how) = &hints.text;
    egui::Area::new(egui::Id::new("hint"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 70.0))
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(10, 12, 16, 210))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.label(
                        egui::RichText::new(what)
                            .font(FontId::proportional(20.0))
                            .color(Color32::WHITE),
                    );
                    ui.add_space(4.0);
                    ui.label(highlighted(how, FontId::proportional(15.0), 440.0));
                });
        });
    Ok(())
}

/// `text` with the parts in [brackets] highlighted (without the brackets).
fn highlighted(text: &str, font: FontId, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let plain = Color32::from_rgb(220, 222, 226);
    for (i, part) in text.split(['[', ']']).enumerate() {
        let color = if i % 2 == 1 { KEY_COLOR } else { plain };
        job.append(
            part,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_highlighted_without_brackets() {
        let job = highlighted("Press [R] now", FontId::default(), 100.0);
        assert_eq!(job.text, "Press R now");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(job.sections[1].format.color, KEY_COLOR);
    }
}
