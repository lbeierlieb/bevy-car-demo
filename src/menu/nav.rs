//! Working the menus with six buttons (up, down, left, right, choose, back) and a menu
//! button, from any device: the keyboard (arrows, Enter, Esc), any gamepad (D-pad or left
//! stick, South, East, Start; no setup) and wheel buttons bound to the menu actions.
//!
//! The directions and "choose" go to egui as arrow keys and Enter, so egui's own focus
//! moves between the widgets (sliders take left and right) and the focused one is
//! clicked. A menu claims the focus with `claim` (the highest rank shown wins), the
//! focused widget gets a ring and is scrolled into view, and while a menu holds the
//! focus no key or wheel button reaches the driving. "Back" and the menu button are
//! `MenuInput`'s, taken by whichever menu is on top (see `MenuSet`).

use bevy::input::InputSystems;
use bevy::input::gamepad::GamepadAxis;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseButton};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiInput, EguiPreUpdateSet, PrimaryEguiContext, egui};

use crate::input::{Action, Capture, Nav, WheelConfig, WheelInput};

/// A held direction repeats after this long, then this often (s).
const REPEAT_DELAY: f32 = 0.4;
const REPEAT_EVERY: f32 = 0.08;
/// How far a stick goes before it counts as a direction.
const STICK: f32 = 0.55;

/// The pad buttons a menu uses unless the driving has them bound.
const PAD_CONFIRM: GamepadButton = GamepadButton::South;
const PAD_BACK: GamepadButton = GamepadButton::East;
const PAD_MENU: GamepadButton = GamepadButton::Start;

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuInput>()
            .init_resource::<MenuFocus>()
            .configure_sets(Update, (MenuSet::Top, MenuSet::Windows, MenuSet::Open).chain())
            .add_systems(Startup, install)
            .add_systems(
                PreUpdate,
                (
                    focus_state.before(crate::input::ReadWheel),
                    read.after(crate::input::ReadWheel)
                        .after(InputSystems)
                        .before(crate::ui::Shield),
                    hand_to_egui
                        .after(read)
                        .after(EguiPreUpdateSet::ProcessInput)
                        .before(EguiPreUpdateSet::BeginPass),
                ),
            );
    }
}

/// Who handles "back" and the menu button first: the menu on top (the central menu, the
/// controls screen), then the windows (shops, lobbies, the map), then what opens the
/// central menu with a "back" nobody took.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MenuSet {
    Top,
    Windows,
    Open,
}

/// The device last used, for the prompts and the focus ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    #[default]
    Keyboard,
    Mouse,
    Pad,
    Wheel,
}

/// This frame's menu buttons, from every device.
#[derive(Resource, Default, Debug)]
pub struct MenuInput {
    /// Directions pressed this frame, held ones repeating: up, down, left, right.
    pub pressed: [bool; 4],
    /// The directions held, as an analog stick or ±1 (x right, y down).
    pub held: Vec2,
    /// "Choose" pressed this frame.
    pub confirm: bool,
    /// "Back" pressed this frame and not yet taken (see `take_back`).
    back: bool,
    /// The menu button pressed and not yet taken.
    open: bool,
    /// Zooming (the map): the pad's triggers or right stick, -1 out to 1 in.
    pub zoom: f32,
    pub device: Device,
    /// How long each direction has been held, and when it last fired.
    hold: [(f32, f32); 4],
    /// The wheel's choose, back and menu buttons last frame.
    wheel_last: [bool; 3],
}

impl MenuInput {
    /// "Back", if pressed and nobody took it yet.
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    /// The menu button, if pressed and nobody took it yet.
    pub fn take_open(&mut self) -> bool {
        std::mem::take(&mut self.open)
    }

    pub fn up(&self) -> bool {
        self.pressed[0]
    }

    pub fn down(&self) -> bool {
        self.pressed[1]
    }

    pub fn left(&self) -> bool {
        self.pressed[2]
    }

    pub fn right(&self) -> bool {
        self.pressed[3]
    }
}

/// Whether a menu holds the focus (then keys and wheel buttons don't drive), and whether
/// a text field has it (then Esc is the field's, not "back"). From the last egui pass.
#[derive(Resource, Default, Debug)]
pub struct MenuFocus {
    pub active: bool,
    pub text: bool,
}

/// What the egui callbacks keep between passes (in the context's memory).
#[derive(Clone, Default)]
struct Scopes {
    /// The menus shown in the last pass and in this one: (rank, layer).
    last: Vec<(u8, egui::LayerId)>,
    this: Vec<(u8, egui::LayerId)>,
    /// The widget that had the focus in the last pass.
    focused: Option<egui::Id>,
    /// Draw the ring (not while the mouse is in use).
    ring: bool,
}

fn scopes_id() -> egui::Id {
    egui::Id::new("menu scopes")
}

/// Hooks into egui's passes: each pass starts by scrolling a newly focused widget into
/// view and ends by drawing the ring round the focused one.
fn install(mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?;
    ctx.on_begin_pass(
        "menu focus",
        std::sync::Arc::new(|ui: &mut egui::Ui| {
            let ctx = ui.ctx();
            let focused = ctx.memory(|m| m.focused());
            let mut scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
            scopes.last = std::mem::take(&mut scopes.this);
            if focused != scopes.focused
                && let Some(response) = focused.and_then(|id| ctx.read_response(id))
            {
                response.scroll_to_me(None);
            }
            scopes.focused = focused;
            ctx.data_mut(|d| d.insert_temp(scopes_id(), scopes));
        }),
    );
    ctx.on_end_pass(
        "menu focus",
        std::sync::Arc::new(|ui: &mut egui::Ui| {
            let ctx = ui.ctx();
            let scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
            if !scopes.ring {
                return;
            }
            let Some(response) = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id))
            else {
                return;
            };
            if !scopes.this.iter().any(|(_, layer)| *layer == response.layer_id) {
                return;
            }
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("focus ring"),
            ));
            painter.rect_stroke(
                response.rect.expand(2.5),
                7.0,
                egui::Stroke::new(2.5, super::ACCENT),
                egui::StrokeKind::Outside,
            );
        }),
    );
    Ok(())
}

/// Called by a menu at the start of its contents: it's a menu of this rank (the central
/// menu outranks a shop's window), and if it ranks highest of those shown, the focus is
/// kept in it (given to its first widget when it's elsewhere). A text field being typed
/// in elsewhere (the chat) keeps the focus.
pub fn claim(ui: &egui::Ui, rank: u8) {
    let ctx = ui.ctx();
    let layer = ui.layer_id();
    let mut scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
    scopes.this.push((rank, layer));
    let top = scopes.last.iter().max_by_key(|(rank, _)| *rank).copied();
    ctx.data_mut(|d| d.insert_temp(scopes_id(), scopes));
    let on_top = top.is_none_or(|(top_rank, top_layer)| top_layer == layer || rank > top_rank);
    if !on_top {
        return;
    }
    let focused = ctx.memory(|m| m.focused());
    let here = focused
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| r.layer_id == layer);
    if here || (focused.is_some() && ctx.text_edit_focused()) {
        return;
    }
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
        m.move_focus(egui::FocusDirection::Next);
    });
}

/// Whether a menu holds the focus and whether it's a text field, from the last pass.
fn focus_state(mut contexts: EguiContexts, mut state: ResMut<MenuFocus>) -> Result {
    let ctx = contexts.ctx_mut()?;
    let scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
    let focused = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id));
    let active = focused.is_some_and(|r| scopes.this.iter().any(|(_, l)| *l == r.layer_id));
    let text = ctx.text_edit_focused();
    if (state.active, state.text) != (active, text) {
        *state = MenuFocus { active, text };
    }
    Ok(())
}

/// The menu buttons of every device into `MenuInput`.
#[allow(clippy::too_many_arguments)]
fn read(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    pads: Query<(&Name, &Gamepad)>,
    wheel: Res<WheelInput>,
    config: Res<WheelConfig>,
    capture: Res<Capture>,
    focus: Res<MenuFocus>,
    mut input: ResMut<MenuInput>,
) {
    let mut held = [false; 4];
    let mut stick = Vec2::ZERO;
    let (mut confirm, mut back, mut open, mut zoom) = (false, false, false, 0.0);
    let mut device = None;
    // The keyboard: egui reads the arrows and Enter itself; Esc is "back" (unless it's a
    // text field's).
    let arrows = [
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
    ];
    for (i, key) in arrows.into_iter().enumerate() {
        held[i] |= keys.pressed(key);
    }
    if keys.just_pressed(KeyCode::Escape) && !focus.text {
        back = true;
    }
    if keys.get_just_pressed().next().is_some() {
        device = Some(Device::Keyboard);
    }
    if motion.delta.length_squared() > 4.0 || mouse.get_just_pressed().next().is_some() {
        device = Some(Device::Mouse);
    }
    // Pads, while nothing is being bound: their standard buttons, unless the driving has
    // them.
    let capturing = capture.0.is_some();
    for (name, pad) in pads.iter().filter(|_| !capturing) {
        let bound = |button: GamepadButton| {
            config.enabled
                && config.bindings.iter().any(|(action, b)| {
                    !matches!(action, Action::Menu(_))
                        && b.is_on(name)
                        && b.input == bevy::input::gamepad::GamepadInput::Button(button)
                })
        };
        let pressed = |button| pad.digital().just_pressed(button) && !bound(button);
        let down = |button| pad.digital().pressed(button) && !bound(button);
        let dpad = [
            GamepadButton::DPadUp,
            GamepadButton::DPadDown,
            GamepadButton::DPadLeft,
            GamepadButton::DPadRight,
        ];
        for (i, button) in dpad.into_iter().enumerate() {
            held[i] |= down(button);
        }
        let left = Vec2::new(
            pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0),
            -pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
        );
        let sticks_bound = config.enabled
            && config.bindings.iter().any(|(_, b)| {
                b.is_on(name)
                    && matches!(
                        b.input,
                        bevy::input::gamepad::GamepadInput::Axis(
                            GamepadAxis::LeftStickX | GamepadAxis::LeftStickY
                        )
                    )
            });
        if !sticks_bound && left.length() > 0.2 {
            stick += left;
            held[0] |= left.y < -STICK;
            held[1] |= left.y > STICK;
            held[2] |= left.x < -STICK;
            held[3] |= left.x > STICK;
        }
        confirm |= pressed(PAD_CONFIRM);
        back |= pressed(PAD_BACK);
        open |= pressed(PAD_MENU);
        let triggers = pad.get(GamepadButton::RightTrigger2).unwrap_or(0.0)
            - pad.get(GamepadButton::LeftTrigger2).unwrap_or(0.0);
        let shoulders = f32::from(u8::from(down(GamepadButton::RightTrigger)))
            - f32::from(u8::from(down(GamepadButton::LeftTrigger)));
        zoom += triggers + shoulders + pad.get(GamepadAxis::RightStickY).unwrap_or(0.0);
        if pad.digital().get_just_pressed().next().is_some() || left.length() > STICK {
            device = Some(Device::Pad);
        }
    }
    // The wheel's buttons bound to the menu actions.
    if !capturing {
        let dirs = [Nav::Up, Nav::Down, Nav::Left, Nav::Right];
        for (i, nav) in dirs.into_iter().enumerate() {
            held[i] |= wheel.nav_held(nav);
        }
        let wheel_pressed = |nav, last: &mut bool| {
            let now = wheel.nav_held(nav);
            let pressed = now && !*last;
            *last = now;
            pressed
        };
        let mut last = input.wheel_last;
        confirm |= wheel_pressed(Nav::Confirm, &mut last[0]);
        back |= wheel_pressed(Nav::Back, &mut last[1]);
        open |= wheel_pressed(Nav::Open, &mut last[2]);
        let any = dirs.iter().any(|&nav| wheel.nav_held(nav)) || last.iter().any(|&b| b);
        if any && last != input.wheel_last {
            device = Some(Device::Wheel);
        }
        input.wheel_last = last;
    }
    // Pressed this frame, or held long enough to repeat.
    let dt = time.delta_secs();
    let mut pressed = [false; 4];
    for i in 0..4 {
        let (since, fired) = &mut input.hold[i];
        if !held[i] {
            *since = 0.0;
            continue;
        }
        if *since == 0.0 {
            pressed[i] = true;
            *fired = 0.0;
        } else if *since >= REPEAT_DELAY && *since - *fired >= REPEAT_EVERY {
            pressed[i] = true;
            *fired = *since;
        } else if *since < REPEAT_DELAY {
            *fired = *since;
        }
        *since += dt.max(1e-4);
    }
    let digital = Vec2::new(
        f32::from(u8::from(held[3])) - f32::from(u8::from(held[2])),
        f32::from(u8::from(held[1])) - f32::from(u8::from(held[0])),
    );
    input.pressed = pressed;
    input.held = if stick.length() > digital.length() {
        stick.clamp_length_max(1.0)
    } else {
        digital
    };
    input.confirm = confirm;
    input.back = back;
    input.open = open;
    input.zoom = zoom.clamp(-1.0, 1.0);
    if let Some(device) = device {
        input.device = device;
    }
}

/// The pads' and wheel's directions and "choose" go to egui as arrow keys and Enter (the
/// keyboard's own go there anyway); whether to ring the focus follows the device.
fn hand_to_egui(
    input: Res<MenuInput>,
    mut contexts: EguiContexts,
    mut egui_input: Query<&mut EguiInput, With<PrimaryEguiContext>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    ctx.data_mut(|d| {
        let scopes = d.get_temp_mut_or_default::<Scopes>(scopes_id());
        scopes.ring = input.device != Device::Mouse;
    });
    let Ok(mut egui_input) = egui_input.single_mut() else {
        return Ok(());
    };
    let arrows = [
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
    ];
    let mut keys_out = Vec::new();
    // The keyboard's arrows are egui's already (repeating as the system repeats them).
    if input.device != Device::Keyboard {
        for (i, key) in arrows.into_iter().enumerate() {
            if input.pressed[i] {
                keys_out.push(key);
            }
        }
    }
    if input.confirm {
        keys_out.push(egui::Key::Enter);
    }
    for key in keys_out {
        for pressed in [true, false] {
            egui_input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    Ok(())
}
