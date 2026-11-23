//! Steering wheel, pedals and shifter through Bevy's gamepad input. Every control is bound
//! to one input on one device (by name); the bindings and wheel settings are saved to a
//! small text file, so they only have to be set up once.

use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::{
    AxisSettings, ButtonAxisSettings, GamepadInput, GamepadSettings, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent,
};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use crate::wheel_base::WheelBase;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WheelConfig::load())
            .init_resource::<WheelInput>()
            .init_resource::<Capture>()
            .init_resource::<Reported>()
            .add_systems(
                PreUpdate,
                (unfilter_gamepads, track_reported, capture, read_wheel.in_set(ReadWheel))
                    .chain()
                    .after(bevy::input::InputSystems),
            )
            .add_systems(Last, save_changes);
    }
}

/// Where the wheel's input is read (`WheelInput`), in `PreUpdate`.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ReadWheel;

/// Something the driver does, which a wheel, pedal or shifter input can be bound to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Steer,
    Throttle,
    Brake,
    Clutch,
    Handbrake,
    ShiftUp,
    ShiftDown,
    /// An H-shifter gate: -1 for reverse, 1..=6 forward. Held while the lever is in it.
    Gate(i8),
    Ignition,
    Reset,
    View,
    /// Held: the camera looks backwards.
    LookBack,
    Map,
    /// A menu button: opening the central menu, or moving and choosing in menus.
    Menu(Nav),
}

/// The menu buttons: open the central menu, move, choose, go back.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Nav {
    Open,
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
}

impl Nav {
    pub const ALL: [Nav; 7] = [
        Nav::Open,
        Nav::Up,
        Nav::Down,
        Nav::Left,
        Nav::Right,
        Nav::Confirm,
        Nav::Back,
    ];

    fn key(self) -> &'static str {
        match self {
            Nav::Open => "menu",
            Nav::Up => "menu_up",
            Nav::Down => "menu_down",
            Nav::Left => "menu_left",
            Nav::Right => "menu_right",
            Nav::Confirm => "menu_confirm",
            Nav::Back => "menu_back",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Nav::Open => "Menu",
            Nav::Up => "Menu: up",
            Nav::Down => "Menu: down",
            Nav::Left => "Menu: left",
            Nav::Right => "Menu: right",
            Nav::Confirm => "Menu: choose",
            Nav::Back => "Menu: back",
        }
    }
}

impl Action {
    pub const ALL: [Action; 26] = [
        Action::Steer,
        Action::Throttle,
        Action::Brake,
        Action::Clutch,
        Action::Handbrake,
        Action::ShiftUp,
        Action::ShiftDown,
        Action::Gate(-1),
        Action::Gate(1),
        Action::Gate(2),
        Action::Gate(3),
        Action::Gate(4),
        Action::Gate(5),
        Action::Gate(6),
        Action::Ignition,
        Action::Reset,
        Action::View,
        Action::LookBack,
        Action::Map,
        Action::Menu(Nav::Open),
        Action::Menu(Nav::Up),
        Action::Menu(Nav::Down),
        Action::Menu(Nav::Left),
        Action::Menu(Nav::Right),
        Action::Menu(Nav::Confirm),
        Action::Menu(Nav::Back),
    ];

    /// Name in the config file.
    fn key(self) -> String {
        match self {
            Action::Steer => "steer".into(),
            Action::Throttle => "throttle".into(),
            Action::Brake => "brake".into(),
            Action::Clutch => "clutch".into(),
            Action::Handbrake => "handbrake".into(),
            Action::ShiftUp => "shift_up".into(),
            Action::ShiftDown => "shift_down".into(),
            Action::Gate(-1) => "gate_r".into(),
            Action::Gate(gear) => format!("gate_{gear}"),
            Action::Ignition => "ignition".into(),
            Action::Reset => "reset".into(),
            Action::View => "view".into(),
            Action::LookBack => "look_back".into(),
            Action::Map => "map".into(),
            Action::Menu(nav) => nav.key().into(),
        }
    }

    pub fn label(self) -> String {
        match self {
            Action::Steer => "Steering".into(),
            Action::Throttle => "Throttle".into(),
            Action::Brake => "Brake".into(),
            Action::Clutch => "Clutch".into(),
            Action::Handbrake => "Handbrake".into(),
            Action::ShiftUp => "Shift up".into(),
            Action::ShiftDown => "Shift down".into(),
            Action::Gate(-1) => "Shifter R".into(),
            Action::Gate(gear) => format!("Shifter {gear}"),
            Action::Ignition => "Ignition".into(),
            Action::Reset => "Reset car".into(),
            Action::View => "Camera view".into(),
            Action::LookBack => "Look back".into(),
            Action::Map => "Map".into(),
            Action::Menu(nav) => nav.label().into(),
        }
    }

    /// Steering and the pedals are read as analog values, the rest as buttons.
    pub fn analog(self) -> bool {
        matches!(
            self,
            Action::Steer | Action::Throttle | Action::Brake | Action::Clutch | Action::Handbrake
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The gamepad's name, as reported by the system (without padding spaces).
    pub device: String,
    pub input: GamepadInput,
    /// Raw value at rest and fully applied. Pedals map linearly from one to the other,
    /// whichever way round they are; for steering, `full` is the value turned left (only its
    /// direction counts: the whole axis is the base's rotation range).
    pub rest: f32,
    pub full: f32,
}

impl Binding {
    /// Whether this binding is for the device with this name. Some devices pad their names
    /// with spaces, which the config file doesn't keep.
    pub fn is_on(&self, device: &str) -> bool {
        device.trim() == self.device.trim()
    }
    /// Pedal travel from 0 (`rest`) to 1 (`full`), with a dead zone at both ends.
    fn travel(&self, raw: f32, deadzone: f32) -> f32 {
        let span = self.full - self.rest;
        if span.abs() < 1e-6 {
            return 0.0;
        }
        let travel = (raw - self.rest) / span;
        ((travel - deadzone) / (1.0 - 2.0 * deadzone)).clamp(0.0, 1.0)
    }

    /// Steering from -1 (right) to 1 (left).
    fn steer(&self, raw: f32) -> f32 {
        let left = if self.full >= self.rest { 1.0 } else { -1.0 };
        ((raw - self.rest) * left).clamp(-1.0, 1.0)
    }
}

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "car-sim-wheel";

/// Wheel bindings and settings.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct WheelConfig {
    /// Whether the bound controls are read at all (off: keyboard only).
    pub enabled: bool,
    pub bindings: Vec<(Action, Binding)>,
    /// Steering wheel rotation lock to lock (degrees). Full steering lock of the car's front
    /// wheels is at either end. The wheel base is set to this range where the game can;
    /// otherwise force feedback holds the wheel at either end (soft lock).
    pub range: f32,
    /// What the wheel base is set to (degrees), if not `range` and the game can't find out
    /// itself.
    pub base_range: Option<f32>,
    /// Dead zone at either end of pedal travel (0..0.5).
    pub deadzone: f32,
    /// Force feedback strength and on/off, and whether its direction is flipped.
    pub ffb_gain: f32,
    /// How much stronger the quick part of the force feedback (bumps, the driven front
    /// wheels' tugs) comes through than the steady pull: 1 is as the tires make it.
    pub ffb_road: f32,
    pub ffb_enabled: bool,
    pub ffb_invert: bool,
}

impl Default for WheelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bindings: Vec::new(),
            range: 540.0,
            base_range: None,
            deadzone: 0.02,
            ffb_gain: 0.5,
            ffb_road: 2.5,
            ffb_enabled: true,
            ffb_invert: false,
        }
    }
}

impl WheelConfig {
    pub fn binding(&self, action: Action) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, binding)| binding)
    }

    /// Binds an input to an action, taking it away from any other action.
    pub fn bind(&mut self, action: Action, binding: Binding) {
        self.unbind(action);
        self.bindings.retain(|(_, b)| {
            (b.device.as_str(), b.input) != (binding.device.as_str(), binding.input)
        });
        self.bindings.push((action, binding));
    }

    pub fn unbind(&mut self, action: Action) {
        self.bindings.retain(|(a, _)| *a != action);
    }

    /// Half the rotation range: the steering wheel angle at full lock (radians).
    pub fn steering_wheel_lock(&self) -> f32 {
        (self.range / 2.0).to_radians()
    }

    fn to_text(&self) -> String {
        let mut text = format!(
            "# car-sim wheel settings\nenabled {}\nrange {}\ndeadzone {}\nffb_gain {}\nffb_road {}\nffb_enabled {}\nffb_invert {}\n",
            self.enabled,
            self.range,
            self.deadzone,
            self.ffb_gain,
            self.ffb_road,
            self.ffb_enabled,
            self.ffb_invert
        );
        if let Some(base_range) = self.base_range {
            text += &format!("base_range {base_range}\n");
        }
        // Bindings: action, input, rest and full values, and the device name (may contain
        // spaces, so it's last).
        for (action, b) in &self.bindings {
            text += &format!(
                "bind {} {} {} {} {}\n",
                action.key(),
                input_name(b.input),
                b.rest,
                b.full,
                b.device
            );
        }
        text
    }

    /// Reads settings written by `to_text`; unknown or broken lines are skipped.
    fn from_text(text: &str) -> Self {
        let mut config = Self::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(value)) = (words.next(), words.next()) else {
                continue;
            };
            match key {
                "enabled" => config.enabled = value.parse().unwrap_or(config.enabled),
                "range" => config.range = value.parse().unwrap_or(config.range),
                "base_range" => config.base_range = value.parse().ok().or(config.base_range),
                "deadzone" => config.deadzone = value.parse().unwrap_or(config.deadzone),
                "ffb_gain" => config.ffb_gain = value.parse().unwrap_or(config.ffb_gain),
                "ffb_road" => config.ffb_road = value.parse().unwrap_or(config.ffb_road),
                "ffb_enabled" => config.ffb_enabled = value.parse().unwrap_or(config.ffb_enabled),
                "ffb_invert" => config.ffb_invert = value.parse().unwrap_or(config.ffb_invert),
                "bind" => {
                    let action = Action::ALL.into_iter().find(|a| a.key() == value);
                    let input = words.next().and_then(parse_input);
                    let rest = words.next().and_then(|w| w.parse().ok());
                    let full = words.next().and_then(|w| w.parse().ok());
                    let device = words.collect::<Vec<_>>().join(" ");
                    if let (Some(action), Some(input), Some(rest), Some(full)) =
                        (action, input, rest, full)
                    {
                        config.bind(
                            action,
                            Binding {
                                device,
                                input,
                                rest,
                                full,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        config
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn path() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
            })?;
        Some(base.join("car-sim").join("wheel.cfg"))
    }

    /// The saved settings, or the defaults if there are none.
    pub fn load() -> Self {
        Self::read().map_or_else(Self::default, |text| Self::from_text(&text))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read() -> Option<String> {
        std::fs::read_to_string(Self::path()?).ok()
    }

    /// Writes the settings to the config file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, self.to_text()));
        if let Err(err) = written {
            warn!("Couldn't save wheel settings to {}: {err}", path.display());
        }
    }

    /// On the web, the settings live in the browser's local storage.
    #[cfg(target_arch = "wasm32")]
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    #[cfg(target_arch = "wasm32")]
    fn read() -> Option<String> {
        Self::storage()?.get_item(STORAGE_KEY).ok()?
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save(&self) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(STORAGE_KEY, &self.to_text());
        }
    }
}

const AXES: [GamepadAxis; 6] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::LeftZ,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
    GamepadAxis::RightZ,
];

const BUTTONS: [GamepadButton; 19] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::North,
    GamepadButton::West,
    GamepadButton::C,
    GamepadButton::Z,
    GamepadButton::LeftTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger,
    GamepadButton::RightTrigger2,
    GamepadButton::Select,
    GamepadButton::Start,
    GamepadButton::Mode,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
];

/// "axis:LeftStickX", "button:Other(300)" and so on.
pub fn input_name(input: GamepadInput) -> String {
    match input {
        GamepadInput::Axis(axis) => format!("axis:{axis:?}"),
        GamepadInput::Button(button) => format!("button:{button:?}"),
    }
}

fn parse_input(name: &str) -> Option<GamepadInput> {
    let (kind, which) = name.split_once(':')?;
    let other = which
        .strip_prefix("Other(")
        .and_then(|w| w.strip_suffix(')'))
        .and_then(|code| code.parse().ok());
    match kind {
        "axis" => other
            .map(GamepadAxis::Other)
            .or_else(|| AXES.into_iter().find(|a| format!("{a:?}") == which))
            .map(GamepadInput::Axis),
        "button" => other
            .map(GamepadButton::Other)
            .or_else(|| BUTTONS.into_iter().find(|b| format!("{b:?}") == which))
            .map(GamepadInput::Button),
        _ => None,
    }
}

/// What the wheel, pedals and shifter ask for this frame. Unbound or disconnected controls
/// are `None`, so the keyboard stays in charge of them.
#[derive(Resource, Default, Debug)]
pub struct WheelInput {
    /// Steering from -1 (right) to 1 (left) over the rotation range.
    pub steer: Option<f32>,
    /// Steering wheel angle from the centre (degrees, positive left). Past half the
    /// rotation range when the base turns farther than that.
    pub angle: Option<f32>,
    pub throttle: Option<f32>,
    pub brake: Option<f32>,
    /// Clutch pedal travel: 0 released, 1 pressed.
    pub clutch: Option<f32>,
    /// A handbrake lever's travel (`None` when the handbrake is a button).
    pub handbrake: Option<f32>,
    /// A handbrake button was pressed this frame: the parking brake goes on or off.
    pub handbrake_toggle: bool,
    /// Buttons pressed this frame.
    pub shift_up: bool,
    pub shift_down: bool,
    pub ignition: bool,
    pub reset: bool,
    pub view: bool,
    /// The look-back button is held.
    pub look_back: bool,
    /// The ignition button is held (the starter runs while it is).
    pub ignition_held: bool,
    pub map: bool,
    /// With an H-shifter bound: the gate the lever is in, or neutral (0) between gates.
    pub shifter: Option<i8>,
    /// Buttons held last frame, to find presses.
    held: Vec<Action>,
}

/// A wheel's axis is one 16-bit value over the whole rotation range: Bevy's default dead
/// zone and change threshold would throw away the small movements around the centre.
fn unfilter_gamepads(mut gamepads: Query<&mut GamepadSettings, Added<GamepadSettings>>) {
    for mut settings in &mut gamepads {
        settings.default_axis_settings =
            AxisSettings::new(-1.0, 0.0, 0.0, 1.0, 0.0).expect("valid axis settings");
        settings.default_button_axis_settings = ButtonAxisSettings {
            high: 1.0,
            low: 0.0,
            threshold: 0.0,
        };
    }
}

/// Inputs each gamepad has sent a value for since it connected. Bevy starts every axis at 0,
/// which for a pedal resting at -1 reads as half pressed, so an input only counts once it
/// has reported a value of its own.
#[derive(Resource, Default)]
pub struct Reported(HashSet<(Entity, GamepadInput)>);

fn track_reported(
    mut reported: ResMut<Reported>,
    mut axes: MessageReader<RawGamepadAxisChangedEvent>,
    mut buttons: MessageReader<RawGamepadButtonChangedEvent>,
) {
    for event in axes.read() {
        reported
            .0
            .insert((event.gamepad, GamepadInput::Axis(event.axis)));
    }
    for event in buttons.read() {
        reported
            .0
            .insert((event.gamepad, GamepadInput::Button(event.button)));
    }
}

/// The connected gamepads, with the inputs that have reported a value.
#[derive(SystemParam)]
pub struct Gamepads<'w, 's> {
    gamepads: Query<'w, 's, (Entity, &'static Name, &'static Gamepad)>,
    reported: Res<'w, Reported>,
}

impl Gamepads<'_, '_> {
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .gamepads
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Each gamepad's reported inputs and their values.
    fn values(&self) -> impl Iterator<Item = (Entity, &Name, GamepadInput, f32)> {
        self.gamepads
            .iter()
            .flat_map(move |(entity, name, gamepad)| {
                gamepad
                    .analog()
                    .all_axes_and_values()
                    .filter(move |(input, _)| self.reported.0.contains(&(entity, **input)))
                    .map(move |(input, value)| (entity, name, *input, value))
            })
    }

    /// Whether a device with the binding's name is connected.
    fn connected(&self, binding: &Binding) -> bool {
        self.gamepads.iter().any(|(_, name, _)| binding.is_on(name))
    }

    /// Raw value of a bound input: from the first device with that name that has reported
    /// it (some wheel bases register a second, silent device under the same name).
    fn raw(&self, binding: &Binding) -> Option<f32> {
        self.gamepads
            .iter()
            .filter(|(entity, name, _)| {
                binding.is_on(name) && self.reported.0.contains(&(*entity, binding.input))
            })
            .find_map(|(_, _, gamepad)| gamepad.get_unclamped(binding.input))
    }
}

/// An action's value: steering from -1 to 1, pedals 0 to 1, buttons 0 or 1.
pub fn action_value(config: &WheelConfig, gamepads: &Gamepads, action: Action) -> Option<f32> {
    let binding = config.binding(action)?;
    let raw = gamepads.raw(binding)?;
    Some(match action {
        Action::Steer => binding.steer(raw),
        _ if action.analog() => binding.travel(raw, config.deadzone),
        _ => binding.travel(raw, 0.0),
    })
}

fn read_wheel(
    config: Res<WheelConfig>,
    base: Res<WheelBase>,
    gamepads: Gamepads,
    menu: Option<Res<crate::menu::nav::MenuFocus>>,
    mut wheel: ResMut<WheelInput>,
) {
    if !config.enabled {
        *wheel = WheelInput::default();
        return;
    }
    let value = |action| action_value(&config, &gamepads, action);
    // The axis covers the base's range, which may be wider than the game's.
    let angle = value(Action::Steer).map(|axis| axis * base.range(&config) / 2.0);
    // A handbrake bound to a button toggles the parking brake, so it counts as a button.
    let handbrake_button = config
        .binding(Action::Handbrake)
        .is_some_and(|binding| matches!(binding.input, GamepadInput::Button(_)));
    let button = |a: Action| !a.analog() || (a == Action::Handbrake && handbrake_button);
    let held: Vec<Action> = Action::ALL
        .into_iter()
        .filter(|a| button(*a) && value(*a).is_some_and(|v| v > 0.5))
        .collect();
    // While a menu has the focus, the buttons only work the menu (the pedals, the
    // steering and the shifter's gates go on: a held brake must stay held).
    let in_menu = menu.is_some_and(|menu| menu.active);
    let pressed = |action| !in_menu && held.contains(&action) && !wheel.held.contains(&action);
    // Each gate is a button held while the lever is in it: the gear is whichever is held.
    // A shifter that isn't connected doesn't count, so Q/E and the paddles shift instead.
    let gates = (1..=6).chain([-1]);
    let shifter_bound = gates.clone().any(|gear| {
        config
            .binding(Action::Gate(gear))
            .is_some_and(|binding| gamepads.connected(binding))
    });
    let shifter = shifter_bound.then(|| {
        gates
            .clone()
            .find(|&gear| held.contains(&Action::Gate(gear)))
            .unwrap_or(0)
    });
    *wheel = WheelInput {
        steer: angle.map(|angle| (angle / (config.range / 2.0)).clamp(-1.0, 1.0)),
        angle,
        throttle: value(Action::Throttle),
        brake: value(Action::Brake),
        clutch: value(Action::Clutch),
        handbrake: value(Action::Handbrake).filter(|_| !handbrake_button),
        handbrake_toggle: pressed(Action::Handbrake),
        shift_up: pressed(Action::ShiftUp),
        shift_down: pressed(Action::ShiftDown),
        ignition: pressed(Action::Ignition),
        reset: pressed(Action::Reset),
        view: pressed(Action::View),
        look_back: !in_menu && held.contains(&Action::LookBack),
        ignition_held: !in_menu && held.contains(&Action::Ignition),
        map: pressed(Action::Map),
        shifter,
        held,
    };
}

impl WheelInput {
    /// Whether the wheel's button bound to this menu action is held.
    pub fn nav_held(&self, nav: Nav) -> bool {
        self.held.contains(&Action::Menu(nav))
    }
}

/// Waiting for the driver to move the control to bind to an action.
#[derive(Resource, Default)]
pub struct Capture(pub Option<Capturing>);

pub struct Capturing {
    pub action: Action,
    /// Each input's value when it was first reported during the capture: where it rests.
    start: Vec<(Entity, GamepadInput, f32)>,
    /// The pedal found so far, and its value farthest from rest.
    pedal: Option<(Entity, GamepadInput, f32, f32)>,
}

impl Capturing {
    pub fn new(action: Action) -> Self {
        Self {
            action,
            start: Vec::new(),
            pedal: None,
        }
    }
}

/// Steering binds to the first axis turned well away from its starting value, buttons and
/// shifter gates to the first button pressed, and pedals to the first axis pushed through
/// most of its travel, once it's released again.
fn capture(mut capture: ResMut<Capture>, mut config: ResMut<WheelConfig>, gamepads: Gamepads) {
    let Some(capturing) = &mut capture.0 else {
        return;
    };
    let action = capturing.action;
    let binding = |name: &Name, input, rest, full| Binding {
        device: name.trim().to_string(),
        input,
        rest,
        full,
    };
    let mut found = None;
    // A handbrake can be a lever (an axis) or a button.
    if !action.analog() || action == Action::Handbrake {
        for (_, name, gamepad) in &gamepads.gamepads {
            if let Some(&button) = gamepad.digital().get_just_pressed().next() {
                found = Some(binding(name, GamepadInput::Button(button), 0.0, 1.0));
            }
        }
    }
    if action.analog() && found.is_none() {
        for (entity, name, input, value) in gamepads.values() {
            let binding = |input, rest, full| binding(name, input, rest, full);
            let start = match capturing
                .start
                .iter()
                .find(|(e, i, _)| (*e, *i) == (entity, input))
            {
                Some(&(_, _, start)) => start,
                None => {
                    capturing.start.push((entity, input, value));
                    value
                }
            };
            match (action, &mut capturing.pedal) {
                // The axis centre is straight ahead; turning left gives the direction.
                (Action::Steer, _) if (value - start).abs() > 0.15 => {
                    found = Some(binding(input, 0.0, value));
                }
                (Action::Steer, _) => {}
                (_, None) if (value - start).abs() > 0.5 => {
                    capturing.pedal = Some((entity, input, start, value));
                }
                (_, Some((e, i, rest, full))) if (*e, *i) == (entity, input) => {
                    if (value - *rest).abs() > (*full - *rest).abs() {
                        *full = value;
                    }
                    if (value - *rest).abs() < 0.15 * (*full - *rest).abs() {
                        found = Some(binding(input, *rest, *full));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(binding) = found {
        config.bind(action, binding);
        capture.0 = None;
    }
}

/// Saves the settings a second after the last change (sliders change them every frame).
fn save_changes(config: Res<WheelConfig>, time: Res<Time>, mut pending: Local<Option<f32>>) {
    if config.is_changed() && !config.is_added() {
        *pending = Some(1.0);
    }
    if let Some(left) = &mut *pending {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            config.save();
            *pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_text() {
        let mut config = WheelConfig {
            enabled: false,
            range: 720.0,
            base_range: Some(2520.0),
            ffb_gain: 0.35,
            ffb_road: 4.0,
            ffb_invert: true,
            ..default()
        };
        config.bind(
            Action::Steer,
            Binding {
                device: "FANATEC CSL DD".into(),
                input: GamepadInput::Axis(GamepadAxis::LeftStickX),
                rest: 0.0,
                full: -0.4,
            },
        );
        config.bind(
            Action::Gate(-1),
            Binding {
                device: "FANATEC CSL DD".into(),
                input: GamepadInput::Button(GamepadButton::Other(300)),
                rest: 0.0,
                full: 1.0,
            },
        );
        assert_eq!(WheelConfig::from_text(&config.to_text()), config);
    }

    #[test]
    fn pedals_map_either_way_round() {
        let pedal = |rest, full| Binding {
            device: String::new(),
            input: GamepadInput::Axis(GamepadAxis::Other(2)),
            rest,
            full,
        };
        // Resting at -1 and pressed at 1, or inverted: half way is half travel either way.
        assert!((pedal(-1.0, 1.0).travel(0.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((pedal(1.0, -1.0).travel(0.0, 0.0) - 0.5).abs() < 1e-6);
        // The dead zone swallows a little noise at rest and saturates at the end.
        assert_eq!(pedal(-1.0, 1.0).travel(-0.97, 0.02), 0.0);
        assert_eq!(pedal(-1.0, 1.0).travel(0.97, 0.02), 1.0);
        // Steering uses the whole axis; `full` only gives the direction.
        assert_eq!(pedal(0.0, -0.3).steer(-0.5), 0.5);
    }

    #[test]
    fn disconnected_shifter_leaves_shifting_to_the_keys() {
        use bevy::ecs::system::RunSystemOnce;
        let mut config = WheelConfig::default();
        config.bind(
            Action::Gate(1),
            Binding {
                device: "Fanatec FANATEC Wheel".into(),
                input: GamepadInput::Button(GamepadButton::Other(300)),
                rest: 0.0,
                full: 1.0,
            },
        );
        let mut world = World::new();
        world.insert_resource(config);
        world.init_resource::<WheelBase>();
        world.init_resource::<Reported>();
        world.init_resource::<WheelInput>();
        world.run_system_once(read_wheel).unwrap();
        assert_eq!(world.resource::<WheelInput>().shifter, None);
    }

    #[test]
    fn device_names_match_without_padding() {
        let mut config = WheelConfig::default();
        let binding = Binding {
            device: "Fanatec FANATEC Wheel    ".into(),
            input: GamepadInput::Axis(GamepadAxis::LeftStickX),
            rest: 0.0,
            full: 1.0,
        };
        config.bind(Action::Steer, binding);
        let loaded = WheelConfig::from_text(&config.to_text());
        assert!(
            loaded
                .binding(Action::Steer)
                .unwrap()
                .is_on("Fanatec FANATEC Wheel    ")
        );
    }
}
