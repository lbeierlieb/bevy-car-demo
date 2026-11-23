//! The driver's controls: keys, and steering wheel, pedals, shifter and gamepads through
//! Bevy's gamepad input. Bindings are kept in four profiles to switch between (the first
//! is the keyboard's defaults, the others start empty: say one for a gamepad, one for a
//! wheel); each control takes up to two inputs (a key, or an input on a device by name).
//! The profiles and the wheel's settings are saved to a small text file, so they only
//! have to be set up once.

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
            .init_resource::<JustBound>()
            .init_resource::<MenuPresses>()
            .init_resource::<Reported>()
            .add_systems(
                PreUpdate,
                (
                    unfilter_gamepads,
                    track_reported,
                    capture,
                    (read_wheel, press_from_menus).chain().in_set(ReadWheel),
                )
                    .chain()
                    .after(bevy::input::InputSystems)
                    // Before a menu with the focus takes the keys away.
                    .before(crate::ui::Shield),
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
    /// Steering by an axis (a wheel, a stick): straight to the front wheels.
    Steer,
    /// Steering by keys or buttons: the front wheels turn towards full lock at a rate.
    SteerLeft,
    SteerRight,
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
    pub const ALL: [Action; 28] = [
        Action::Steer,
        Action::SteerLeft,
        Action::SteerRight,
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
            Action::SteerLeft => "steer_left".into(),
            Action::SteerRight => "steer_right".into(),
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
            Action::Steer => "Steering (wheel or stick)".into(),
            Action::SteerLeft => "Steer left".into(),
            Action::SteerRight => "Steer right".into(),
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

/// What a binding reads: an input of a gamepad (a wheel, pedals, a shifter, a pad), or a
/// key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Pad(GamepadInput),
    Key(KeyCode),
}

impl Input {
    /// An axis (steering, a pedal, a lever), rather than a button or key.
    pub fn is_axis(self) -> bool {
        matches!(self, Input::Pad(GamepadInput::Axis(_)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The gamepad's name, as reported by the system (without padding spaces); empty for a
    /// key.
    pub device: String,
    pub input: Input,
    /// Raw value at rest and fully applied. Pedals map linearly from one to the other,
    /// whichever way round they are; for steering, `full` is the value turned left (only its
    /// direction counts: the whole axis is the base's rotation range).
    pub rest: f32,
    pub full: f32,
}

impl Binding {
    pub fn key(code: KeyCode) -> Self {
        Self {
            device: String::new(),
            input: Input::Key(code),
            rest: 0.0,
            full: 1.0,
        }
    }

    /// Its name for people: "W", "Up", "button 3", "LeftStickX".
    pub fn label(&self) -> String {
        input_label(self.input)
    }

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
    /// The profiles (`PROFILES` of them), and which is in use.
    pub profiles: Vec<Profile>,
    pub active: usize,
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
            profiles: Profile::defaults(),
            active: 0,
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

/// How many profiles there are, and how many inputs a control takes in one.
pub const PROFILES: usize = 4;
pub const PER_ACTION: usize = 2;

/// A set of bindings with a name.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Profile {
    pub name: String,
    pub bindings: Vec<(Action, Binding)>,
}

impl Profile {
    /// The keyboard's defaults.
    pub fn keyboard() -> Self {
        use KeyCode::*;
        let keys = [
            (Action::Throttle, KeyW),
            (Action::Throttle, ArrowUp),
            (Action::Brake, KeyS),
            (Action::Brake, ArrowDown),
            (Action::SteerLeft, KeyA),
            (Action::SteerLeft, ArrowLeft),
            (Action::SteerRight, KeyD),
            (Action::SteerRight, ArrowRight),
            (Action::Handbrake, Space),
            (Action::ShiftUp, KeyE),
            (Action::ShiftDown, KeyQ),
            (Action::Ignition, KeyI),
            (Action::Reset, KeyR),
            (Action::View, KeyC),
            (Action::LookBack, KeyB),
            (Action::Map, KeyM),
        ];
        Self {
            name: "Keyboard".into(),
            bindings: keys
                .into_iter()
                .map(|(action, key)| (action, Binding::key(key)))
                .collect(),
        }
    }

    /// The keyboard's, then empty ones.
    pub fn defaults() -> Vec<Self> {
        let mut profiles = vec![Self::keyboard()];
        profiles.extend((2..=PROFILES).map(|n| Self {
            name: format!("Profile {n}"),
            bindings: Vec::new(),
        }));
        profiles
    }

    /// The inputs bound to an action, in the order they were bound.
    pub fn bindings_of(&self, action: Action) -> impl Iterator<Item = &Binding> {
        self.bindings
            .iter()
            .filter(move |(a, _)| *a == action)
            .map(|(_, binding)| binding)
    }

    /// Adds an input to an action, taking it away from any other action; past
    /// `PER_ACTION` inputs, the oldest goes.
    pub fn bind(&mut self, action: Action, binding: Binding) {
        self.bindings
            .retain(|(_, b)| (b.device.trim(), b.input) != (binding.device.trim(), binding.input));
        if self.bindings_of(action).count() >= PER_ACTION
            && let Some(oldest) = self.bindings.iter().position(|(a, _)| *a == action)
        {
            self.bindings.remove(oldest);
        }
        self.bindings.push((action, binding));
    }

    pub fn unbind(&mut self, action: Action) {
        self.bindings.retain(|(a, _)| *a != action);
    }
}

impl WheelConfig {
    /// The profile in use.
    pub fn profile(&self) -> &Profile {
        &self.profiles[self.active.min(self.profiles.len() - 1)]
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        let active = self.active.min(self.profiles.len() - 1);
        &mut self.profiles[active]
    }

    /// The first input bound to an action in the profile in use.
    pub fn binding(&self, action: Action) -> Option<&Binding> {
        self.profile().bindings_of(action).next()
    }

    pub fn bind(&mut self, action: Action, binding: Binding) {
        self.profile_mut().bind(action, binding);
    }

    pub fn unbind(&mut self, action: Action) {
        self.profile_mut().unbind(action);
    }

    /// Half the rotation range: the steering wheel angle at full lock (radians).
    pub fn steering_wheel_lock(&self) -> f32 {
        (self.range / 2.0).to_radians()
    }

    fn to_text(&self) -> String {
        let mut text = format!(
            "# car-sim wheel settings\nrange {}\ndeadzone {}\nffb_gain {}\nffb_road {}\nffb_enabled {}\nffb_invert {}\nactive {}\n",
            self.range,
            self.deadzone,
            self.ffb_gain,
            self.ffb_road,
            self.ffb_enabled,
            self.ffb_invert,
            self.active,
        );
        if let Some(base_range) = self.base_range {
            text += &format!("base_range {base_range}\n");
        }
        // Each profile, then its bindings: action, input, rest and full values, and the
        // device name (may contain spaces, so it's last).
        for (i, profile) in self.profiles.iter().enumerate() {
            text += &format!("profile {i} {}\n", profile.name);
            for (action, b) in &profile.bindings {
                text += &format!(
                    "bind {} {} {} {} {}\n",
                    action.key(),
                    input_name(b.input),
                    b.rest,
                    b.full,
                    b.device
                );
            }
        }
        text
    }

    /// Reads settings written by `to_text`; unknown or broken lines are skipped. A file
    /// from before the profiles has its bindings put in the second profile ("Wheel"),
    /// which is then in use; the first gets the keyboard's defaults.
    fn from_text(text: &str) -> Self {
        let mut config = Self::default();
        let mut profiles: Option<Vec<Profile>> = None;
        let mut current = 0;
        let mut old = Profile {
            name: "Wheel".into(),
            bindings: Vec::new(),
        };
        let mut active = None;
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(value)) = (words.next(), words.next()) else {
                continue;
            };
            match key {
                "range" => config.range = value.parse().unwrap_or(config.range),
                "base_range" => config.base_range = value.parse().ok().or(config.base_range),
                "deadzone" => config.deadzone = value.parse().unwrap_or(config.deadzone),
                "ffb_gain" => config.ffb_gain = value.parse().unwrap_or(config.ffb_gain),
                "ffb_road" => config.ffb_road = value.parse().unwrap_or(config.ffb_road),
                "ffb_enabled" => config.ffb_enabled = value.parse().unwrap_or(config.ffb_enabled),
                "ffb_invert" => config.ffb_invert = value.parse().unwrap_or(config.ffb_invert),
                "active" => active = value.parse::<usize>().ok().filter(|&a| a < PROFILES),
                "profile" => {
                    let Some(i) = value.parse::<usize>().ok().filter(|&i| i < PROFILES) else {
                        continue;
                    };
                    let list = profiles.get_or_insert_with(|| {
                        (1..=PROFILES)
                            .map(|n| Profile {
                                name: format!("Profile {n}"),
                                bindings: Vec::new(),
                            })
                            .collect()
                    });
                    let name = words.collect::<Vec<_>>().join(" ");
                    if !name.is_empty() {
                        list[i].name = name;
                    }
                    current = i;
                }
                "bind" => {
                    let action = Action::ALL.into_iter().find(|a| a.key() == value);
                    let input = words.next().and_then(parse_input);
                    let rest = words.next().and_then(|w| w.parse().ok());
                    let full = words.next().and_then(|w| w.parse().ok());
                    let device = words.collect::<Vec<_>>().join(" ");
                    if let (Some(action), Some(input), Some(rest), Some(full)) =
                        (action, input, rest, full)
                    {
                        let binding = Binding {
                            device,
                            input,
                            rest,
                            full,
                        };
                        match &mut profiles {
                            Some(list) => list[current].bind(action, binding),
                            None => old.bind(action, binding),
                        }
                    }
                }
                _ => {}
            }
        }
        match profiles {
            Some(list) => {
                config.profiles = list;
                config.active = active.unwrap_or(0);
            }
            None if !old.bindings.is_empty() => {
                config.profiles[1] = old;
                config.active = 1;
            }
            None => {}
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

/// The keys a control can be bound to (Escape is the menu's).
/// Not H, T, 1 and 2: they're the help's, the chat's, the gizmos' and the telemetry's.
const KEYS: [KeyCode; 78] = {
    use KeyCode::*;
    [
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        Space,
        Enter,
        Tab,
        Backspace,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F12,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadAdd,
        NumpadSubtract,
        NumpadEnter,
        Comma,
        Period,
        Slash,
        Semicolon,
        Minus,
        Equal,
        Home,
        End,
    ]
};

/// "key:KeyW", "axis:LeftStickX", "button:Other(300)" and so on.
pub fn input_name(input: Input) -> String {
    match input {
        Input::Key(key) => format!("key:{key:?}"),
        Input::Pad(GamepadInput::Axis(axis)) => format!("axis:{axis:?}"),
        Input::Pad(GamepadInput::Button(button)) => format!("button:{button:?}"),
    }
}

/// An input's name for people: "W", "Up", "button 3", "LeftStickX".
pub fn input_label(input: Input) -> String {
    match input {
        Input::Key(key) => {
            let name = format!("{key:?}");
            let name = name
                .strip_prefix("Key")
                .or_else(|| name.strip_prefix("Digit"))
                .or_else(|| name.strip_prefix("Arrow"))
                .unwrap_or(&name);
            name.to_string()
        }
        Input::Pad(GamepadInput::Button(GamepadButton::Other(n))) => format!("button {n}"),
        Input::Pad(GamepadInput::Axis(GamepadAxis::Other(n))) => format!("axis {n}"),
        Input::Pad(GamepadInput::Button(button)) => format!("{button:?}"),
        Input::Pad(GamepadInput::Axis(axis)) => format!("{axis:?}"),
    }
}

fn parse_input(name: &str) -> Option<Input> {
    let (kind, which) = name.split_once(':')?;
    let other = which
        .strip_prefix("Other(")
        .and_then(|w| w.strip_suffix(')'))
        .and_then(|code| code.parse().ok());
    match kind {
        "key" => KEYS
            .into_iter()
            .find(|k| format!("{k:?}") == which)
            .map(Input::Key),
        "axis" => other
            .map(GamepadAxis::Other)
            .or_else(|| AXES.into_iter().find(|a| format!("{a:?}") == which))
            .map(|axis| Input::Pad(GamepadInput::Axis(axis))),
        "button" => other
            .map(GamepadButton::Other)
            .or_else(|| BUTTONS.into_iter().find(|b| format!("{b:?}") == which))
            .map(|button| Input::Pad(GamepadInput::Button(button))),
        _ => None,
    }
}

/// What the driver's controls ask for this frame, from the profile in use. Unbound or
/// disconnected axes are `None`.
#[derive(Resource, Default, Debug)]
pub struct WheelInput {
    /// Steering by an axis from -1 (right) to 1 (left) over the rotation range.
    pub steer: Option<f32>,
    /// Steering by keys or buttons: -1 (right), 0 or 1 (left), to turn towards.
    pub steer_keys: f32,
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
    keys: Res<'w, ButtonInput<KeyCode>>,
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

    /// Whether a device with the binding's name is connected (the keyboard always is).
    fn connected(&self, binding: &Binding) -> bool {
        match binding.input {
            Input::Key(_) => true,
            Input::Pad(_) => self.gamepads.iter().any(|(_, name, _)| binding.is_on(name)),
        }
    }

    /// Raw value of a bound input: from the first device with that name that has reported
    /// it (some wheel bases register a second, silent device under the same name).
    fn raw(&self, binding: &Binding) -> Option<f32> {
        let input = match binding.input {
            Input::Key(key) => return Some(f32::from(u8::from(self.keys.pressed(key)))),
            Input::Pad(input) => input,
        };
        self.gamepads
            .iter()
            .filter(|(entity, name, _)| {
                binding.is_on(name) && self.reported.0.contains(&(*entity, input))
            })
            .find_map(|(_, _, gamepad)| gamepad.get_unclamped(input))
    }
}

/// A binding's value: steering from -1 to 1, pedals 0 to 1, buttons and keys 0 or 1.
fn binding_value(
    config: &WheelConfig,
    gamepads: &Gamepads,
    action: Action,
    binding: &Binding,
) -> Option<f32> {
    let raw = gamepads.raw(binding)?;
    Some(match action {
        Action::Steer => binding.steer(raw),
        _ if action.analog() => binding.travel(raw, config.deadzone),
        _ => binding.travel(raw, 0.0),
    })
}

/// An action's value in the profile in use, the most of its inputs that pass `which`.
fn value_of(
    config: &WheelConfig,
    gamepads: &Gamepads,
    action: Action,
    which: impl Fn(&Binding) -> bool,
) -> Option<f32> {
    config
        .profile()
        .bindings_of(action)
        .filter(|b| which(b))
        .filter_map(|b| binding_value(config, gamepads, action, b))
        .reduce(|a, b| if b.abs() > a.abs() { b } else { a })
}

/// An action's value: steering from -1 to 1, pedals 0 to 1, buttons and keys 0 or 1.
pub fn action_value(config: &WheelConfig, gamepads: &Gamepads, action: Action) -> Option<f32> {
    value_of(config, gamepads, action, |_| true)
}

fn read_wheel(
    config: Res<WheelConfig>,
    base: Res<WheelBase>,
    gamepads: Gamepads,
    menu: Option<Res<crate::menu::nav::MenuFocus>>,
    mut wheel: ResMut<WheelInput>,
) {
    let value = |action| action_value(&config, &gamepads, action);
    let axis = |action| value_of(&config, &gamepads, action, |b| b.input.is_axis());
    // The axis covers the base's range, which may be wider than the game's.
    let angle = axis(Action::Steer).map(|axis| axis * base.range(&config) / 2.0);
    // A handbrake bound to a button or key toggles the parking brake; to an axis, it's a
    // lever.
    let button =
        |a: Action, b: &Binding| !a.analog() || (a == Action::Handbrake && !b.input.is_axis());
    let held: Vec<Action> = Action::ALL
        .into_iter()
        .filter(|&a| value_of(&config, &gamepads, a, |b| button(a, b)).is_some_and(|v| v > 0.5))
        .collect();
    // While a menu has the focus, the buttons only work the menu (the pedals, the
    // steering and the shifter's gates go on: a held brake must stay held).
    let in_menu = menu.is_some_and(|menu| menu.active);
    let pressed = |action| !in_menu && held.contains(&action) && !wheel.held.contains(&action);
    let down = |action| !in_menu && held.contains(&action);
    // Each gate is a button held while the lever is in it: the gear is whichever is held.
    // A shifter that isn't connected doesn't count, so the shift buttons work instead.
    let gates = (1..=6).chain([-1]);
    let shifter_bound = gates.clone().any(|gear| {
        (config.profile().bindings_of(Action::Gate(gear)))
            .any(|binding| gamepads.connected(binding))
    });
    let shifter = shifter_bound.then(|| {
        gates
            .clone()
            .find(|&gear| held.contains(&Action::Gate(gear)))
            .unwrap_or(0)
    });
    let steer_keys = f32::from(u8::from(down(Action::SteerLeft)))
        - f32::from(u8::from(down(Action::SteerRight)));
    *wheel = WheelInput {
        steer: angle.map(|angle| (angle / (config.range / 2.0)).clamp(-1.0, 1.0)),
        steer_keys,
        angle,
        throttle: value(Action::Throttle),
        brake: value(Action::Brake),
        clutch: value(Action::Clutch),
        handbrake: axis(Action::Handbrake),
        handbrake_toggle: pressed(Action::Handbrake),
        shift_up: pressed(Action::ShiftUp),
        shift_down: pressed(Action::ShiftDown),
        ignition: pressed(Action::Ignition),
        reset: pressed(Action::Reset),
        view: pressed(Action::View),
        look_back: down(Action::LookBack),
        ignition_held: down(Action::Ignition),
        map: pressed(Action::Map),
        shifter,
        held,
    };
}

/// Buttons a menu presses for the driver (reset, the camera, the map, the parking brake,
/// the ignition), taken as the wheel's next frame: so a controller without those buttons
/// still has them, and nothing else needs to know about the menu.
#[derive(Resource, Default)]
pub struct MenuPresses {
    press: Vec<Action>,
    /// The ignition held for this long (s): the starter turns until the engine catches.
    crank: f32,
}

impl MenuPresses {
    pub fn press(&mut self, action: Action) {
        self.press.push(action);
    }

    /// Runs the starter for `seconds`.
    pub fn crank(&mut self, seconds: f32) {
        self.crank = seconds;
    }
}

fn press_from_menus(
    time: Res<Time>,
    mut presses: ResMut<MenuPresses>,
    mut wheel: ResMut<WheelInput>,
) {
    for action in std::mem::take(&mut presses.press) {
        match action {
            Action::Handbrake => wheel.handbrake_toggle = true,
            Action::ShiftUp => wheel.shift_up = true,
            Action::ShiftDown => wheel.shift_down = true,
            Action::Ignition => wheel.ignition = true,
            Action::Reset => wheel.reset = true,
            Action::View => wheel.view = true,
            Action::Map => wheel.map = true,
            _ => {}
        }
    }
    if presses.crank > 0.0 {
        presses.crank -= time.delta_secs();
        wheel.ignition_held = true;
    }
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

/// Something was bound this frame (the key that did it isn't also the menu's).
#[derive(Resource, Default)]
pub struct JustBound(pub bool);

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
fn capture(
    mut capture: ResMut<Capture>,
    mut config: ResMut<WheelConfig>,
    gamepads: Gamepads,
    mut just: ResMut<JustBound>,
) {
    just.0 = false;
    let Some(capturing) = &mut capture.0 else {
        return;
    };
    let action = capturing.action;
    let binding = |name: &Name, input, rest, full| Binding {
        device: name.trim().to_string(),
        input: Input::Pad(input),
        rest,
        full,
    };
    let mut found = None;
    // Any control but steering by an axis takes a key.
    if action != Action::Steer
        && let Some(&key) = gamepads.keys.get_just_pressed().find(|k| KEYS.contains(k))
    {
        found = Some(Binding::key(key));
    }
    // A handbrake can be a lever (an axis) or a button.
    if found.is_none() && (!action.analog() || action == Action::Handbrake) {
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
        just.0 = true;
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

    fn pad(input: GamepadInput, rest: f32, full: f32) -> Binding {
        Binding {
            device: "FANATEC CSL DD".into(),
            input: Input::Pad(input),
            rest,
            full,
        }
    }

    #[test]
    fn config_round_trips_through_text() {
        let mut config = WheelConfig {
            range: 720.0,
            base_range: Some(2520.0),
            ffb_gain: 0.35,
            ffb_road: 4.0,
            ffb_invert: true,
            active: 2,
            ..default()
        };
        config.profiles[2].name = "Wheel and pedals".into();
        config.bind(
            Action::Steer,
            pad(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.0, -0.4),
        );
        config.bind(
            Action::Gate(-1),
            pad(GamepadInput::Button(GamepadButton::Other(300)), 0.0, 1.0),
        );
        config.bind(Action::Reset, Binding::key(KeyCode::F5));
        assert_eq!(WheelConfig::from_text(&config.to_text()), config);
    }

    #[test]
    fn a_file_from_before_the_profiles_becomes_the_second() {
        let old = "enabled true\nrange 900\nbind steer axis:LeftStickX 0 -0.4 Fanatec Wheel\n";
        let config = WheelConfig::from_text(old);
        assert_eq!(config.range, 900.0);
        assert_eq!(config.active, 1);
        assert_eq!(config.profiles[0], Profile::keyboard());
        assert_eq!(config.profiles[1].name, "Wheel");
        assert!(
            config
                .binding(Action::Steer)
                .is_some_and(|b| b.is_on("Fanatec Wheel"))
        );
    }

    #[test]
    fn a_control_takes_two_inputs_and_an_input_one_control() {
        let mut profile = Profile::default();
        let keys = [KeyCode::KeyW, KeyCode::ArrowUp, KeyCode::KeyK];
        for key in keys {
            profile.bind(Action::Throttle, Binding::key(key));
        }
        let bound: Vec<_> = profile
            .bindings_of(Action::Throttle)
            .map(|b| b.input)
            .collect();
        assert_eq!(
            bound,
            [Input::Key(KeyCode::ArrowUp), Input::Key(KeyCode::KeyK)]
        );
        profile.bind(Action::Brake, Binding::key(KeyCode::KeyK));
        assert_eq!(profile.bindings_of(Action::Throttle).count(), 1);
    }

    #[test]
    fn the_keyboard_profile_drives_and_an_empty_one_doesnt() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(WheelConfig::default());
        world.init_resource::<WheelBase>();
        world.init_resource::<Reported>();
        world.init_resource::<WheelInput>();
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyW);
        keys.press(KeyCode::KeyA);
        world.insert_resource(keys);
        world.run_system_once(read_wheel).unwrap();
        let wheel = world.resource::<WheelInput>();
        assert_eq!((wheel.throttle, wheel.steer_keys), (Some(1.0), 1.0));
        assert_eq!(wheel.brake, Some(0.0));
        world.resource_mut::<WheelConfig>().active = 3;
        world.run_system_once(read_wheel).unwrap();
        let wheel = world.resource::<WheelInput>();
        assert_eq!((wheel.throttle, wheel.steer_keys), (None, 0.0));
    }

    #[test]
    fn pedals_map_either_way_round() {
        let pedal = |rest, full| pad(GamepadInput::Axis(GamepadAxis::Other(2)), rest, full);
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
            pad(GamepadInput::Button(GamepadButton::Other(300)), 0.0, 1.0),
        );
        let mut world = World::new();
        world.insert_resource(config);
        world.init_resource::<WheelBase>();
        world.init_resource::<Reported>();
        world.init_resource::<WheelInput>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.run_system_once(read_wheel).unwrap();
        assert_eq!(world.resource::<WheelInput>().shifter, None);
    }

    #[test]
    fn device_names_match_without_padding() {
        let mut config = WheelConfig::default();
        let mut binding = pad(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.0, 1.0);
        binding.device = "Fanatec FANATEC Wheel    ".into();
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
