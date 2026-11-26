//! The central menu (Esc, a pad's Start, or a wheel's menu button): everything a driver
//! may need, also for whoever doesn't know the keys or whose controller lacks a button:
//! reset, the map, the camera, the engine and the parking brake; the driving aids, the
//! controls and bindings, force feedback, sound, display, the players online, help.
//! The game goes on while it's open.

use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use super::curve;
use super::kit::{self, Panel, rank};
use super::nav::{MenuInput, MenuSet};
use super::{CHASE_FOVS, DRIVER_FOVS, Display, SIZES};
use crate::audio::SoundSettings;
use crate::car::{ActiveCar, Away, Car, DriverAids, Spectating};
use crate::graphics::{
    Antialiasing, DAY_LENGTHS, FRAME_CAPS, Graphics, RESOLUTIONS, Shadows, TimeOfDay, ViewDistance,
};
use crate::input::{
    self, Action, Capture, Capturing, CurveKind, Gamepads, MenuPresses, Nav, Preset, Profile,
    WheelConfig, action_value,
};
use crate::net::{Connection, Status};
use crate::shops::ShopKind;
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
    Actions,
    Settings,
    Aids,
    Controls,
    Bindings,
    ForceFeedback,
    Sound,
    Display,
    Graphics,
    Online,
    Curve(CurveKind),
}

impl Screen {
    fn title(self) -> &'static str {
        match self {
            Screen::Main => "Menu",
            Screen::Actions => "In-game actions",
            Screen::Settings => "Settings",
            Screen::Aids => "Driving aids",
            Screen::Controls => "Controls",
            Screen::Bindings => "Bindings",
            Screen::ForceFeedback => "Force feedback",
            Screen::Sound => "Sound",
            Screen::Display => "Display",
            Screen::Graphics => "Graphics",
            Screen::Online => "Online",
            Screen::Curve(CurveKind::Steer) => "Steering curve",
            Screen::Curve(CurveKind::Throttle) => "Gas curve",
            Screen::Curve(CurveKind::Brake) => "Brake curve",
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
    /// "Clear every binding" was chosen once: the next time clears.
    clearing: bool,
    /// A curve's point is grabbed: "back" puts it back (`cancel_grab`) instead.
    grabbed: bool,
    cancel_grab: bool,
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
        } else if menu.grabbed {
            menu.cancel_grab = true;
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
    spectating: Option<Res<'w, Spectating>>,
    away: Res<'w, Away>,
    presses: ResMut<'w, MenuPresses>,
    shops: Option<Res<'w, crate::shops::Shops>>,
}

#[derive(SystemParam)]
struct Settings<'w> {
    aids: ResMut<'w, DriverAids>,
    sound: ResMut<'w, SoundSettings>,
    display: ResMut<'w, Display>,
    graphics: ResMut<'w, Graphics>,
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
    event: Option<ResMut<'w, crate::events::lobby::EventState>>,
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
        Screen::Controls | Screen::Curve(_) => 520.0,
        _ => 460.0,
    };
    let focused = ctx.memory(|m| m.focused());
    let mut go = None;
    menu.grabbed = false;
    Panel::new("central menu", screen.title())
        .rank(rank::CENTRAL)
        .width(width)
        .back(Some(back))
        .show(ctx, |ui| match screen {
            Screen::Main => go = main(ui, &mut menu, &mut online, &mut commands, &mut exit),
            Screen::Actions => actions(ui, &mut menu, &mut driving, &wheel.config, &mut online),
            Screen::Settings => {
                for screen in [
                    Screen::Aids,
                    Screen::Controls,
                    Screen::Sound,
                    Screen::Display,
                    Screen::Graphics,
                ] {
                    if kit::submenu(ui, screen.title()).clicked() {
                        go = Some(screen);
                    }
                }
            }
            Screen::Aids => aids(ui, &mut settings),
            Screen::Controls => go = controls(ui, &mut wheel.config),
            Screen::Bindings => go = bindings(ui, &mut menu, &mut wheel),
            Screen::ForceFeedback => force_feedback(ui, &mut wheel),
            Screen::Sound => sound(ui, &mut settings),
            Screen::Display => display(ui, &mut settings, &mut windows),
            Screen::Graphics => graphics(ui, &mut settings.graphics),
            Screen::Online => {
                if online_screen(ui, &mut online, &mut commands) {
                    menu.close();
                }
            }
            Screen::Curve(kind) => curve_screen(ui, &mut menu, &mut wheel, kind),
        });
    if let Some(next) = go {
        menu.push(next, focused);
        // The new screen starts at its first row (rows on different screens can share
        // egui's ids, which would keep the focus where it was).
        if let Some(id) = focused {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }
    Ok(())
}

/// The main screen: the in-game actions and settings, an event's round, online, help,
/// and back to driving at the bottom.
fn main(
    ui: &mut egui::Ui,
    menu: &mut CentralMenu,
    online: &mut Online,
    commands: &mut Commands,
    exit: &mut MessageWriter<AppExit>,
) -> Option<Screen> {
    let mut go = None;
    for screen in [Screen::Actions, Screen::Settings] {
        if kit::submenu(ui, screen.title()).clicked() {
            go = Some(screen);
        }
    }
    // In an event's round under way: starting a time trial again, or leaving.
    if let (Some(connection), Some(state)) = (&mut online.connection, &mut online.event)
        && let Some(round) = connection.round.clone().filter(|round| round.running())
    {
        use crate::events::round::{Action as RoundAction, Game};
        kit::section(ui, round.game.title());
        let trial = matches!(round.game, Game::Trial(_));
        if trial && kit::button(ui, "Start the time trial again").clicked() {
            connection.act(RoundAction::Restart);
            menu.close();
        }
        let leave = match round.game {
            Game::Trial(_) => "Stop the time trial",
            Game::Race(_) => "Leave the race",
            Game::Hunt(_) => "Leave the hunt",
        };
        if kit::button(ui, leave).clicked() {
            crate::events::lobby::leave(connection, state);
            menu.close();
        }
    }
    let joined = online
        .connection
        .as_ref()
        .is_some_and(|c| matches!(c.status, Status::Joined { .. }));
    // Alone is on the game's own server: no players to list, and joining a real one
    // leaves it.
    let alone = online.start.as_ref().is_some_and(|start| start.alone);
    if joined && !alone {
        kit::section(ui, "Online");
        let count = online
            .connection
            .as_ref()
            .map_or(0, |c| c.players.len() + 1);
        let label = format!("Players ({count})");
        if kit::submenu(ui, &label).clicked() {
            go = Some(Screen::Online);
        }
    } else if (online.connection.is_none() || alone)
        && !cfg!(target_arch = "wasm32")
        && let Some(start) = &mut online.start
    {
        kit::section(ui, "Online");
        if kit::button(ui, "Join a server").clicked() {
            if let Some(connection) = &mut online.connection {
                connection.close();
                commands.remove_resource::<Connection>();
            }
            start.alone = false;
            start.open = true;
            menu.close();
        }
    }
    kit::section(ui, "Help");
    if let Some(intro) = &mut online.intro
        && kit::button_with(ui, "Quick start", "H").clicked()
    {
        intro.open = true;
        menu.close();
    }
    kit::note(
        ui,
        "Map: Gerach to Rattelsdorf and Baunach, Bavaria. Map data © OpenStreetMap \
         contributors (ODbL). Elevation: DGM1 © Bayerische Vermessungsverwaltung (CC BY 4.0).",
    );
    ui.add_space(8.0);
    if !cfg!(target_arch = "wasm32") && kit::button(ui, "Quit").clicked() {
        exit.write(AppExit::Success);
    }
    go
}

/// What the profile in use has bound to an action, for a row's right side.
fn bound(config: &WheelConfig, action: Action) -> String {
    config.profile().labels(action).join(", ")
}

/// Things to do while driving, also for a controller without the buttons: reset, the
/// map, the camera.
fn actions(
    ui: &mut egui::Ui,
    menu: &mut CentralMenu,
    driving: &mut Driving,
    config: &WheelConfig,
    online: &mut Online,
) {
    let car = driving
        .cars
        .iter()
        .find(|car| car.index == driving.active.0);
    // (Whether R does anything here is the server's call: the event's rules.)
    let can_reset = car.is_some()
        && !driving.away.0
        && (driving.spectating.as_deref()).is_none_or(|s| s.0.is_none());
    let reset = ui
        .add_enabled_ui(can_reset, |ui| {
            kit::button_with(
                ui,
                "Reset the car (helicopter)",
                &bound(config, Action::Reset),
            )
        })
        .inner;
    if reset.clicked() {
        driving.presses.press(Action::Reset);
        menu.close();
    }
    if kit::button_with(ui, "Map", &bound(config, Action::Map)).clicked() {
        driving.presses.press(Action::Map);
        menu.close();
    }
    if kit::button_with(ui, "Chase or driver's view", &bound(config, Action::View)).clicked() {
        driving.presses.press(Action::View);
        menu.close();
    }
    // The special rescue: a car that can't drive on, to the nearest gas station or car
    // workshop (whether it goes is the server's call too).
    let Some(connection) = online.connection.as_deref_mut() else {
        return;
    };
    let Some(car) = car else {
        return;
    };
    kit::section(ui, "Special rescue");
    let stranded = can_reset && car.stranded();
    let p = car.body.position.xz();
    for kind in [ShopKind::Gas, ShopKind::Car] {
        let list = driving
            .shops
            .as_ref()
            .map_or(&[][..], |shops| &shops.list[..]);
        let nearest = (list.iter().enumerate())
            .filter(|(_, placed)| placed.shop.kind == kind)
            .min_by(|(_, a), (_, b)| a.marker.distance(p).total_cmp(&b.marker.distance(p)));
        let Some((i, placed)) = nearest else {
            continue;
        };
        let label = format!("To {}", placed.shop.name);
        let far = format!("{:.1} km", placed.marker.distance(p) / 1000.0);
        let go = ui
            .add_enabled_ui(stranded, |ui| kit::button_with(ui, &label, &far))
            .inner;
        if go.clicked() {
            connection.say(&crate::net::protocol::ToServer::Rescue { to: i as u16 });
            menu.close();
        }
    }
    kit::note(
        ui,
        "For a car that can't drive on (its engine wrecked, its tank empty): the rescue \
         helicopter takes it to the nearest gas station or parts shop.",
    );
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
}

/// The profile in use (and its name), its bindings, force feedback, and the keys that
/// are always the same.
fn controls(ui: &mut egui::Ui, config: &mut WheelConfig) -> Option<Screen> {
    let mut go = None;
    let names: Vec<String> = config.profiles.iter().map(|p| p.name.clone()).collect();
    let mut active = config.active;
    if kit::choice(ui, "Profile in use", &mut active, &names).changed() {
        config.active = active;
    }
    let label = format!("Bindings of \"{}\"", config.profile().name);
    if kit::submenu(ui, &label).clicked() {
        go = Some(Screen::Bindings);
    }
    if kit::submenu(ui, Screen::ForceFeedback.title()).clicked() {
        go = Some(Screen::ForceFeedback);
    }
    kit::note(
        ui,
        "Four profiles to switch between: the first is the keyboard's, the others start \
         empty, say one for a gamepad and one for a wheel. Only the one in use drives.",
    );
    kit::section(ui, "The profile's name");
    let mut name = config.profile().name.clone();
    if kit::text_field(ui, &mut name, "Name", 1).changed() && !name.trim().is_empty() {
        config.profile_mut().name = name.replace(['\n', '\r'], " ");
    }
    kit::section(ui, "Always");
    for (keys, what) in [
        ("Esc", "Menu"),
        ("H", "Quick start"),
        ("T", "Chat (on a server)"),
        ("1 / 2", "Gizmos / telemetry"),
        ("F11", "Fullscreen"),
        ("Left-drag, scroll", "Look around, zoom"),
    ] {
        kit::info(ui, what, keys);
    }
    kit::note(
        ui,
        "A gamepad works the menus as it is: the D-pad moves, A chooses, B goes back, \
         Start opens this menu.",
    );
    go
}

fn bindings(ui: &mut egui::Ui, menu: &mut CentralMenu, wheel: &mut Wheel) -> Option<Screen> {
    let mut go = None;
    let Wheel {
        config,
        capture,
        gamepads,
        base,
        ..
    } = wheel;
    let names = gamepads.names();
    if names.is_empty() {
        kit::note(ui, "No wheel or gamepad connected.");
    } else {
        kit::note(ui, &format!("Connected: {}", names.join(", ")));
    }
    kit::note(
        ui,
        "Choose a row, then press the key or move the control (up to two each). Left clears \
         a row; back cancels.",
    );
    if kit::button(ui, "Set up the menu buttons, one by one").clicked() {
        capture.0 = None;
        menu.guide = Some(0);
    }
    let listening = capture.0.as_ref().map(|c| c.action);
    let mut section = "";
    for action in Action::ALL {
        let group = match action {
            Action::Steer
            | Action::SteerLeft
            | Action::SteerRight
            | Action::Throttle
            | Action::Brake
            | Action::Clutch => "Driving",
            Action::Handbrake | Action::ShiftUp | Action::ShiftDown | Action::Gate(_) => {
                "Brake and gears"
            }
            Action::Menu(_) => "Menus (the keyboard's arrows, Enter and Esc always work)",
            _ => "Other",
        };
        if group != section {
            section = group;
            kit::section(ui, group);
        }
        let detail = if listening == Some(action) {
            match action {
                Action::Steer => "turn the wheel left…".to_string(),
                Action::LookSide => "push the stick left…".to_string(),
                Action::LookUp => "push the stick up…".to_string(),
                Action::Gate(_) => "move the lever into the gate and out…".to_string(),
                _ if action.analog() => "press it fully, then release…".to_string(),
                _ => "press and release it…".to_string(),
            }
        } else {
            let names = bound(config, action);
            if names.is_empty() {
                "—".to_string()
            } else {
                let shown = match action_value(config, gamepads, action) {
                    Some(v) if action.whole_axis() => format!("  {:+.0} %", v * 100.0),
                    Some(v) if action.analog() && v > 0.0 => format!("  {:.0} %", v * 100.0),
                    Some(v) if v > 0.5 => "  ●".to_string(),
                    _ => String::new(),
                };
                format!("{names}{shown}")
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
    kit::section(ui, "The whole profile");
    if kit::button(ui, "Fill it with the keyboard's defaults").clicked() {
        config.profile_mut().bindings = Profile::keyboard().bindings;
    }
    let clear = if menu.clearing {
        "Really clear every binding?"
    } else {
        "Clear every binding"
    };
    if kit::button(ui, clear).clicked() {
        if menu.clearing {
            config.profile_mut().bindings.clear();
        }
        menu.clearing ^= true;
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
    kit::section(ui, "Curves");
    for kind in CurveKind::ALL {
        let shown = Preset::of(config.profile().curves.get(kind)).map_or("Your own", Preset::name);
        if kit::submenu_with(ui, Screen::Curve(kind).title(), shown).clicked() {
            go = Some(Screen::Curve(kind));
        }
    }
    kit::note(
        ui,
        "For the profile in use: how far a stick, trigger or pedal is pushed against what \
         it asks for. Linear for a wheel and pedals; a force feedback wheel always steers \
         linearly.",
    );
    go
}

/// One of the profile's curves, drawn and edited.
fn curve_screen(ui: &mut egui::Ui, menu: &mut CentralMenu, wheel: &mut Wheel, kind: CurveKind) {
    let live = input::push_value(&wheel.config, &wheel.gamepads, kind.action());
    // Edited on a copy, so the settings only count as changed (and get saved) when they are.
    let mut curve = wheel.config.profile().curves.get(kind).clone();
    let cancel = std::mem::take(&mut menu.cancel_grab);
    let edited = curve::editor(ui, kind.key(), &mut curve, live, cancel);
    menu.grabbed = edited.grabbed;
    if edited.changed {
        *wheel.config.profile_mut().curves.get_mut(kind) = curve;
    }
    if kit::button_with(ui, "Back to the default", kind.default_preset().name()).clicked() {
        *wheel.config.profile_mut().curves.get_mut(kind) = kind.default_preset().curve();
    }
    kit::note(
        ui,
        match kind {
            CurveKind::Steer => {
                "Across: how far the steering is pushed; up: how far the front wheels \
                 turn. Lower in the middle is finer around straight ahead. Not for a force \
                 feedback wheel, which always steers 1:1."
            }
            CurveKind::Throttle => {
                "Across: how far the gas is pushed; up: how far it opens. Lower early is \
                 finer at part throttle."
            }
            CurveKind::Brake => {
                "Across: how far the brake is pushed; up: how hard it brakes. Lower early is \
                 easier to trail off."
            }
        },
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
    let degrees = |v: f32| format!("{v:.0}°");
    let mut fov = settings.display.driver_fov;
    if kit::slider(
        ui,
        "Field of view (driver)",
        &mut fov,
        DRIVER_FOVS,
        1.0,
        degrees,
    )
    .changed()
    {
        settings.display.driver_fov = fov;
    }
    let mut fov = settings.display.chase_fov;
    if kit::slider(
        ui,
        "Field of view (chase)",
        &mut fov,
        CHASE_FOVS,
        1.0,
        degrees,
    )
    .changed()
    {
        settings.display.chase_fov = fov;
    }
    kit::note(
        ui,
        "The view's height in degrees (vertical), as field of view calculators give it.",
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

fn graphics(ui: &mut egui::Ui, graphics: &mut ResMut<Graphics>) {
    let mut new = **graphics;
    kit::pick(
        ui,
        "Shadows",
        &mut new.shadows,
        &[Shadows::Off, Shadows::Low, Shadows::High],
        |s| {
            match s {
                Shadows::Off => "Off",
                Shadows::Low => "Low",
                Shadows::High => "High",
            }
            .into()
        },
    );
    kit::pick(
        ui,
        "Antialiasing",
        &mut new.antialiasing,
        &[Antialiasing::Off, Antialiasing::Fxaa, Antialiasing::Msaa],
        |a| {
            match a {
                Antialiasing::Off => "Off",
                Antialiasing::Fxaa => "FXAA",
                Antialiasing::Msaa => "MSAA 4×",
            }
            .into()
        },
    );
    kit::pick(
        ui,
        "View distance",
        &mut new.view,
        &[ViewDistance::Near, ViewDistance::Far],
        |v| {
            match v {
                ViewDistance::Near => "Near",
                ViewDistance::Far => "Far",
            }
            .into()
        },
    );
    let mut resolution = new.resolution.unwrap_or(100);
    if kit::pick(
        ui,
        "Render resolution",
        &mut resolution,
        &RESOLUTIONS,
        |p| format!("{p} %"),
    ) {
        new.resolution = Some(resolution);
    }
    if !cfg!(target_arch = "wasm32") {
        kit::toggle(ui, "Vsync", &mut new.vsync);
        kit::pick(
            ui,
            "Frame rate limit",
            &mut new.frame_cap,
            &FRAME_CAPS,
            |cap| cap.map_or("Off".into(), |fps| format!("{fps} fps")),
        );
    }
    kit::toggle(ui, "Post-processing", &mut new.post);
    if new.post {
        kit::toggle(ui, "Day and night", &mut new.day_night);
    }
    if new.post && new.day_night {
        kit::pick(
            ui,
            "Time of day",
            &mut new.time_of_day,
            &TimeOfDay::ALL,
            |t| t.name().into(),
        );
        if new.time_of_day == TimeOfDay::Cycle {
            kit::pick(
                ui,
                "Length of a day",
                &mut new.day_minutes,
                &DAY_LENGTHS,
                |m| format!("{m} min"),
            );
        }
    }
    kit::note(
        ui,
        "Shadows, multisampling, a far view and a high resolution cost the most. FXAA \
         smooths edges cheaply. Post-processing (the time of day's colours and light) \
         is one cheap pass; switch it off if the browser struggles (it's then always \
         day).",
    );
    if new != **graphics {
        **graphics = new;
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
