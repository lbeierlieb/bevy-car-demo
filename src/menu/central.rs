//! The central menu (Esc, a pad's Start, or a wheel's menu button): everything a driver
//! may need, also for whoever doesn't know the keys or whose controller lacks a button:
//! reset, the map, the camera, the engine and the parking brake; the driving aids, the
//! controls and bindings, force feedback, sound, display, the players online, help.
//! The game goes on while it's open.

use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::GamepadInput;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use super::kit::{self, Panel, rank};
use super::nav::{MenuInput, MenuSet};
use super::{Display, SIZES};
use crate::audio::SoundSettings;
use crate::car::{ActiveCar, Away, Car, DriverAids, ResetRule};
use crate::heli::rescue::Rescue;
use crate::input::{
    Action, Capture, Capturing, Gamepads, MenuPresses, Nav, WheelConfig, action_value,
};
use crate::net::{Connection, Status};
use crate::traffic::{MOST_CARS, Source, Traffic};
use crate::wheel_base::{ForceFeedback, WheelBase};

pub struct CentralMenuPlugin;

impl Plugin for CentralMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CentralMenu>()
            .add_systems(
                Update,
                (
                    (open_and_back, follow_guide).chain().in_set(MenuSet::Top),
                    open_on_back.in_set(MenuSet::Open),
                ),
            )
            .add_systems(EguiPrimaryContextPass, show);
    }
}

/// The screens of the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Main,
    Aids,
    Controls,
    Bindings,
    ForceFeedback,
    Sound,
    Display,
    Online,
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::Main => "Menu",
            Screen::Aids => "Driving aids",
            Screen::Controls => "Controls",
            Screen::Bindings => "Wheel, pedals and gamepad",
            Screen::ForceFeedback => "Force feedback",
            Screen::Sound => "Sound",
            Screen::Display => "Display",
            Screen::Online => "Online",
        }
    }
}

#[derive(Resource, Default)]
pub struct CentralMenu {
    /// Open, with the screens gone into (the last shown), each with the row to focus
    /// when coming back to it.
    stack: Vec<(Screen, Option<egui::Id>)>,
    /// Focus this row next pass (coming back to a screen).
    refocus: Option<egui::Id>,
    /// Going through the menu buttons one by one to bind them: which is next.
    guide: Option<usize>,
}

impl CentralMenu {
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn open(&mut self) {
        self.stack = vec![(Screen::Main, None)];
    }

    pub fn close(&mut self) {
        self.stack.clear();
        self.guide = None;
    }

    fn push(&mut self, screen: Screen, from: Option<egui::Id>) {
        if let Some(top) = self.stack.last_mut() {
            top.1 = from;
        }
        self.stack.push((screen, None));
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.refocus = self.stack.last().and_then(|(_, id)| *id);
    }
}

/// Whether another menu owns the screen (the start menu, the controls screen at the
/// start): then the menu button and "back" aren't the central menu's.
fn others_open(world_menus: &OtherMenus) -> bool {
    world_menus.start.as_ref().is_some_and(|m| m.open)
        || world_menus.intro.as_ref().is_some_and(|i| i.open)
}

#[derive(SystemParam)]
struct OtherMenus<'w> {
    start: Option<Res<'w, crate::online::Menu>>,
    intro: Option<Res<'w, crate::help::Intro>>,
}

/// The menu button opens or closes it; "back" goes back a screen (or cancels a
/// binding).
fn open_and_back(
    mut menu: ResMut<CentralMenu>,
    mut input: ResMut<MenuInput>,
    mut capture: ResMut<Capture>,
    others: OtherMenus,
) {
    if others_open(&others) {
        return;
    }
    if input.take_open() {
        if menu.is_open() {
            menu.close();
            capture.0 = None;
        } else {
            menu.open();
        }
        return;
    }
    if menu.is_open() && input.take_back() {
        if capture.0.is_some() {
            capture.0 = None;
            menu.guide = None;
        } else if menu.stack.len() > 1 {
            menu.pop();
        } else {
            menu.close();
        }
    }
}

/// "Back" that no window took opens the menu.
fn open_on_back(mut menu: ResMut<CentralMenu>, mut input: ResMut<MenuInput>, others: OtherMenus) {
    if !menu.is_open() && !others_open(&others) && input.take_back() {
        menu.open();
    }
}

/// Binding the menu buttons one after the other: once one is bound, the next.
fn follow_guide(mut menu: ResMut<CentralMenu>, mut capture: ResMut<Capture>) {
    let Some(next) = menu.guide else {
        return;
    };
    if capture.0.is_some() {
        return;
    }
    if next < Nav::ALL.len() {
        capture.0 = Some(Capturing::new(Action::Menu(Nav::ALL[next])));
        menu.guide = Some(next + 1);
    } else {
        menu.guide = None;
    }
}

/// The driving's state the main screen shows and acts on.
#[derive(SystemParam)]
struct Driving<'w, 's> {
    active: Res<'w, ActiveCar>,
    cars: Query<'w, 's, &'static Car>,
    rule: Option<Res<'w, ResetRule>>,
    away: Res<'w, Away>,
    rescue: Option<Res<'w, Rescue>>,
    presses: ResMut<'w, MenuPresses>,
}

#[derive(SystemParam)]
struct Settings<'w> {
    aids: ResMut<'w, DriverAids>,
    traffic: ResMut<'w, Traffic>,
    source: Res<'w, Source>,
    sound: ResMut<'w, SoundSettings>,
    display: ResMut<'w, Display>,
}

#[derive(SystemParam)]
struct Wheel<'w, 's> {
    config: ResMut<'w, WheelConfig>,
    capture: ResMut<'w, Capture>,
    gamepads: Gamepads<'w, 's>,
    base: Res<'w, WheelBase>,
    ffb: ResMut<'w, ForceFeedback>,
}

#[derive(SystemParam)]
struct Online<'w> {
    connection: Option<ResMut<'w, Connection>>,
    start: Option<ResMut<'w, crate::online::Menu>>,
    intro: Option<ResMut<'w, crate::help::Intro>>,
}

#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut menu: ResMut<CentralMenu>,
    mut driving: Driving,
    mut settings: Settings,
    mut wheel: Wheel,
    mut online: Online,
    mut windows: Query<&mut Window>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let Some(&(screen, _)) = menu.stack.last() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    if let Some(id) = menu.refocus.take() {
        ctx.memory_mut(|m| m.request_focus(id));
    }
    let back = if menu.stack.len() > 1 {
        "Back"
    } else {
        "Close"
    };
    let width = match screen {
        Screen::Bindings => 560.0,
        Screen::Controls => 520.0,
        _ => 460.0,
    };
    let focused = ctx.memory(|m| m.focused());
    let mut go = None;
    Panel::new("central menu", screen.title())
        .rank(rank::CENTRAL)
        .width(width)
        .back(Some(back))
        .show(ctx, |ui| match screen {
            Screen::Main => {
                go = main(
                    ui,
                    &mut menu,
                    &mut driving,
                    &mut online,
                    &mut windows,
                    &mut exit,
                );
            }
            Screen::Aids => aids(ui, &mut settings),
            Screen::Controls => go = controls(ui),
            Screen::Bindings => bindings(ui, &mut menu, &mut wheel),
            Screen::ForceFeedback => force_feedback(ui, &mut wheel),
            Screen::Sound => sound(ui, &mut settings),
            Screen::Display => display(ui, &mut settings, &mut windows),
            Screen::Online => {
                if online_screen(ui, &mut online, &mut commands) {
                    menu.close();
                }
            }
        });
    if let Some(next) = go {
        menu.push(next, focused);
    }
    Ok(())
}

/// The main screen: actions first, then the screens.
fn main(
    ui: &mut egui::Ui,
    menu: &mut CentralMenu,
    driving: &mut Driving,
    online: &mut Online,
    windows: &mut Query<&mut Window>,
    exit: &mut MessageWriter<AppExit>,
) -> Option<Screen> {
    let car = driving
        .cars
        .iter()
        .find(|car| car.index == driving.active.0);
    if kit::button(ui, "Back to driving").clicked() {
        menu.close();
    }
    let can_reset = car.is_some()
        && !driving.away.0
        && driving.rescue.as_ref().is_none_or(|r| !r.busy())
        && driving
            .rule
            .as_deref()
            .is_some_and(|rule| *rule != ResetRule::Never);
    let reset = ui
        .add_enabled_ui(can_reset, |ui| {
            kit::button_with(ui, "Reset the car (helicopter)", "R")
        })
        .inner;
    if reset.clicked() {
        driving.presses.press(Action::Reset);
        menu.close();
    }
    if kit::button_with(ui, "Map", "M").clicked() {
        driving.presses.press(Action::Map);
        menu.close();
    }
    if kit::button_with(ui, "Chase or driver's view", "C").clicked() {
        driving.presses.press(Action::View);
        menu.close();
    }
    if let Some(car) = car {
        let running = car.drivetrain.running;
        let (label, key) = if running {
            ("Switch the engine off", "I")
        } else {
            ("Start the engine", "hold I")
        };
        if kit::button_with(ui, label, key).clicked() {
            if running {
                driving.presses.press(Action::Ignition);
            } else {
                driving.presses.crank(2.0);
            }
            menu.close();
        }
        let label = if car.parking_brake {
            "Release the parking brake"
        } else {
            "Pull the parking brake"
        };
        if kit::button_with(ui, label, "Space").clicked() {
            driving.presses.press(Action::Handbrake);
            menu.close();
        }
    }
    let mut go = None;
    kit::section(ui, "Settings");
    for screen in [
        Screen::Aids,
        Screen::Controls,
        Screen::Sound,
        Screen::Display,
    ] {
        if kit::submenu(ui, screen.title()).clicked() {
            go = Some(screen);
        }
    }
    let joined = online
        .connection
        .as_ref()
        .is_some_and(|c| matches!(c.status, Status::Joined { .. }));
    if joined {
        kit::section(ui, "Online");
        let count = online
            .connection
            .as_ref()
            .map_or(0, |c| c.players.len() + 1);
        let label = format!("Players ({count})");
        if kit::submenu(ui, &label).clicked() {
            go = Some(Screen::Online);
        }
    } else if online.connection.is_none()
        && !cfg!(target_arch = "wasm32")
        && let Some(start) = &mut online.start
    {
        kit::section(ui, "Online");
        if kit::button(ui, "Join a server").clicked() {
            start.open = true;
            menu.close();
        }
    }
    kit::section(ui, "Help");
    if let Some(intro) = &mut online.intro
        && kit::button_with(ui, "The controls", "H").clicked()
    {
        intro.open = true;
        menu.close();
    }
    if !cfg!(target_arch = "wasm32") {
        if kit::button_with(ui, "Fullscreen", "F11").clicked() {
            toggle_fullscreen(windows);
        }
        if kit::button(ui, "Quit").clicked() {
            exit.write(AppExit::Success);
        }
    }
    go
}

fn toggle_fullscreen(windows: &mut Query<&mut Window>) {
    for mut window in windows.iter_mut() {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}

fn aids(ui: &mut egui::Ui, settings: &mut Settings) {
    let aids = &mut settings.aids;
    let mut tc = aids.traction_control;
    kit::toggle(ui, "Traction control", &mut tc);
    kit::note(
        ui,
        "Eases the throttle when the driven wheels spin: full throttle on a key would \
         just spin the wheels of anything strong.",
    );
    let mut abs = aids.abs;
    kit::toggle(ui, "ABS", &mut abs);
    kit::note(
        ui,
        "Eases the brakes before a wheel locks, so the car still steers while braking.",
    );
    if (tc, abs) != (aids.traction_control, aids.abs) {
        aids.traction_control = tc;
        aids.abs = abs;
        aids.save();
    }
    // Alone: the traffic around (on a server, the server's).
    if *settings.source == Source::Own {
        kit::section(ui, "Traffic");
        let mut cars = settings.traffic.cars as f32;
        if kit::slider(
            ui,
            "Cars around",
            &mut cars,
            0.0..=MOST_CARS as f32,
            1.0,
            |v| format!("{v:.0}"),
        )
        .changed()
        {
            settings.traffic.cars = cars as usize;
        }
    }
}

/// The keys, and the screens for the wheel and force feedback.
fn controls(ui: &mut egui::Ui) -> Option<Screen> {
    let mut go = None;
    if kit::submenu(ui, Screen::Bindings.title()).clicked() {
        go = Some(Screen::Bindings);
    }
    if kit::submenu(ui, Screen::ForceFeedback.title()).clicked() {
        go = Some(Screen::ForceFeedback);
    }
    kit::section(ui, "Keyboard");
    for (keys, action) in crate::help::CONTROLS {
        kit::info(ui, action, keys);
    }
    kit::section(ui, "Gamepad");
    kit::note(
        ui,
        "In the menus: the D-pad or left stick moves, A chooses, B goes back, Start opens \
         this menu. To drive with a gamepad, bind its sticks and buttons under \"Wheel, \
         pedals and gamepad\".",
    );
    go
}

/// A gamepad input's name for people.
fn input_label(input: GamepadInput) -> String {
    use bevy::input::gamepad::{GamepadAxis, GamepadButton};
    match input {
        GamepadInput::Button(GamepadButton::Other(n)) => format!("button {n}"),
        GamepadInput::Axis(GamepadAxis::Other(n)) => format!("axis {n}"),
        GamepadInput::Button(button) => format!("{button:?}"),
        GamepadInput::Axis(axis) => format!("{axis:?}"),
    }
}

fn bindings(ui: &mut egui::Ui, menu: &mut CentralMenu, wheel: &mut Wheel) {
    let Wheel {
        config,
        capture,
        gamepads,
        base,
        ..
    } = wheel;
    let mut enabled = config.enabled;
    if kit::toggle(ui, "Use wheel, pedals and gamepad", &mut enabled).changed() {
        config.enabled = enabled;
    }
    let names = gamepads.names();
    if names.is_empty() {
        kit::note(ui, "Nothing connected.");
    } else {
        kit::note(ui, &format!("Connected: {}", names.join(", ")));
    }
    if kit::button(ui, "Set up the menu buttons, one by one").clicked() {
        capture.0 = None;
        menu.guide = Some(0);
    }
    kit::note(
        ui,
        "Choose a row, then move the control to bind it. Left clears a row; back cancels.",
    );
    let listening = capture.0.as_ref().map(|c| c.action);
    let mut section = "";
    for action in Action::ALL {
        let group = match action {
            Action::Steer | Action::Throttle | Action::Brake | Action::Clutch => "Driving",
            Action::Handbrake | Action::ShiftUp | Action::ShiftDown | Action::Gate(_) => {
                "Brake and gears"
            }
            Action::Menu(_) => "Menus",
            _ => "Other",
        };
        if group != section {
            section = group;
            kit::section(ui, group);
        }
        let detail = if listening == Some(action) {
            match action {
                Action::Steer => "turn the wheel left…".to_string(),
                Action::Gate(_) => "move the lever into the gate…".to_string(),
                _ if action.analog() => "press fully, then release…".to_string(),
                _ => "press the button…".to_string(),
            }
        } else {
            match config.binding(action) {
                Some(binding) => {
                    let value = action_value(config, gamepads, action);
                    let shown = match value {
                        Some(v) if action == Action::Steer => format!("  {:+.0} %", v * 100.0),
                        Some(v) if action.analog() => format!("  {:.0} %", v * 100.0),
                        Some(v) if v > 0.5 => "  ●".to_string(),
                        _ => String::new(),
                    };
                    format!("{}{shown}", input_label(binding.input))
                }
                None => "—".to_string(),
            }
        };
        let response = kit::button_with(ui, &action.label(), &detail);
        if response.clicked() {
            capture.0 = Some(Capturing::new(action));
            menu.guide = None;
        }
        if kit::focused(ui, &response)
            && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft))
            && listening.is_none()
        {
            config.unbind(action);
        }
    }
    kit::section(ui, "Wheel");
    kit::slider(
        ui,
        "Rotation range",
        &mut config.range,
        180.0..=1080.0,
        10.0,
        |v| format!("{v:.0}°"),
    );
    match (base.applied, base.actual) {
        (Some(range), _) => kit::note(
            ui,
            &format!("Wheel base set to {range:.0}° (restored on exit)"),
        ),
        (None, Some(actual)) => kit::note(
            ui,
            &format!("Wheel base is at {actual:.0}° and can't be changed: soft lock at the range"),
        ),
        (None, None) => {
            let mut other = config.base_range.is_some();
            kit::toggle(ui, "Wheel base set to another range", &mut other);
            config.base_range = other.then(|| {
                let mut range = config.base_range.unwrap_or(900.0);
                kit::slider(
                    ui,
                    "Wheel base range",
                    &mut range,
                    180.0..=2880.0,
                    10.0,
                    |v| format!("{v:.0}°"),
                );
                range
            });
        }
    }
    kit::slider(
        ui,
        "Pedal dead zone",
        &mut config.deadzone,
        0.0..=0.15,
        0.01,
        |v| format!("{:.0} %", v * 100.0),
    );
}

fn force_feedback(ui: &mut egui::Ui, wheel: &mut Wheel) {
    let Wheel { config, ffb, .. } = wheel;
    let mut on = config.ffb_enabled;
    if kit::toggle(ui, "Force feedback", &mut on).changed() {
        config.ffb_enabled = on;
    }
    let Some(device) = ffb.device.clone() else {
        kit::note(
            ui,
            &match &ffb.problem {
                Some(problem) => format!("No force feedback: {problem}"),
                None => "No force feedback device: bind the steering to a wheel.".into(),
            },
        );
        return;
    };
    kit::note(ui, &format!("On {device}"));
    kit::slider(ui, "Strength", &mut config.ffb_gain, 0.0..=1.0, 0.05, |v| {
        format!("{:.0} %", v * 100.0)
    });
    kit::slider(ui, "Road feel", &mut config.ffb_road, 0.0..=5.0, 0.1, |v| {
        format!("{v:.1}")
    });
    kit::note(
        ui,
        "How strongly bumps and the driven front wheels' tugs come through, against the \
         steady pull of the tires: 1 is as they make it.",
    );
    let side = if ffb.level >= 0.0 { "left" } else { "right" };
    let clipping = if ffb.level.abs() > 0.99 {
        "  clipping: lower the strength"
    } else {
        ""
    };
    kit::info(
        ui,
        "Force now",
        &format!("{:.0} % {side}{clipping}", ffb.level.abs() * 100.0),
    );
    if kit::button(ui, "Test: push left").clicked() {
        ffb.test_push();
    }
    let mut invert = config.ffb_invert;
    if kit::toggle(ui, "Reverse direction", &mut invert).changed() {
        config.ffb_invert = invert;
    }
    kit::note(
        ui,
        "Turn this on if the test pushes the wheel to the right.",
    );
}

fn sound(ui: &mut egui::Ui, settings: &mut Settings) {
    let sound = &mut settings.sound;
    let mut on = sound.on;
    if kit::toggle(ui, "Sound", &mut on).changed() {
        sound.on = on;
    }
    let mut volume = sound.volume;
    if kit::slider(ui, "Volume", &mut volume, 0.0..=1.0, 0.05, |v| {
        format!("{:.0} %", v * 100.0)
    })
    .changed()
    {
        sound.volume = volume;
    }
}

fn display(ui: &mut egui::Ui, settings: &mut Settings, windows: &mut Query<&mut Window>) {
    let mut size = settings.display.ui_size;
    if kit::slider(ui, "Size of menus and text", &mut size, SIZES, 0.05, |v| {
        format!("{:.0} %", v * 100.0)
    })
    .changed()
    {
        settings.display.ui_size = size;
    }
    kit::note(
        ui,
        "Everything grows with the window; this scales it on top (the window's height \
         sets 100 %).",
    );
    if !cfg!(target_arch = "wasm32") {
        let mut full = windows
            .iter()
            .any(|window| !matches!(window.mode, WindowMode::Windowed));
        if kit::toggle(ui, "Fullscreen", &mut full).changed() {
            toggle_fullscreen(windows);
        }
    }
}

/// Who's on the server with their pings, and leaving it. Returns whether we left.
fn online_screen(ui: &mut egui::Ui, online: &mut Online, commands: &mut Commands) -> bool {
    let Some(connection) = &mut online.connection else {
        return false;
    };
    let Status::Joined { you } = connection.status else {
        return false;
    };
    let ping = |id| (connection.pings.get(&id)).map_or(String::new(), |ms| format!("{ms} ms"));
    let event = |id| {
        (connection.busy.get(&id))
            .and_then(|&e| connection.events.get(usize::from(e)))
            .map_or(String::new(), |event| format!("{}   ", event.name))
    };
    let name = online.start.as_ref().map_or("", |m| m.name());
    kit::info(
        ui,
        &format!("You: {name}"),
        &format!("{}{}", event(you), ping(you)),
    );
    for (&id, name) in &connection.players {
        kit::info(ui, name, &format!("{}{}", event(id), ping(id)));
    }
    ui.add_space(8.0);
    if kit::button(ui, "Leave the server").clicked() {
        connection.close();
        commands.remove_resource::<Connection>();
        if let Some(start) = &mut online.start {
            start.open = true;
        }
        return true;
    }
    false
}
