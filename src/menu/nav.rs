//! Working the menus with six buttons (up, down, left, right, choose, back) and a menu
//! button, from any device: the keyboard (arrows, Enter, Esc), any gamepad (D-pad, South,
//! East, Start; no setup) and wheel buttons bound to the menu actions. No axes: pedals
//! report as sticks too, and a clutch held halfway scrolled the menus.
//!
//! The directions and "choose" go to egui as arrow keys and Enter, so egui's own focus
//! moves between the widgets (sliders take left and right) and the focused one is
//! clicked. A menu claims the focus with `claim` (the highest rank shown wins), the
//! focused widget gets a ring and is scrolled into view, and while a menu holds the
//! focus no key or wheel button reaches the driving. "Back" and the menu button are
//! `MenuInput`'s, taken by whichever menu is on top (see `MenuSet`).

use bevy::input::InputSystems;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseButton};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiInput, EguiPreUpdateSet, PrimaryEguiContext, egui};

use bevy::input::gamepad::GamepadInput;

use crate::input::{Action, Capture, Input, JustBound, Nav, WheelConfig, WheelInput};

/// A held direction repeats after this long, then this often (s).
const REPEAT_DELAY: f32 = 0.4;
const REPEAT_EVERY: f32 = 0.08;

/// The pad buttons a menu uses unless the driving has them bound.
const PAD_CONFIRM: GamepadButton = GamepadButton::South;
const PAD_BACK: GamepadButton = GamepadButton::East;
const PAD_MENU: GamepadButton = GamepadButton::Start;

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuInput>()
            .init_resource::<MenuFocus>()
            .configure_sets(
                Update,
                (MenuSet::Top, MenuSet::Windows, MenuSet::Open).chain(),
            )
            .add_systems(Update, install)
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
    /// The directions held, ±1 (x right, y down).
    pub held: Vec2,
    /// The same from pads and the wheel's menu buttons only (the keyboard's arrows steer
    /// the car where nothing has the focus: the map).
    pub pad_held: Vec2,
    /// "Choose" pressed this frame.
    pub confirm: bool,
    /// "Back" pressed this frame and not yet taken (see `take_back`).
    back: bool,
    /// A pad's or the wheel's "back" while a text field is typed in: Esc for it.
    escape: bool,
    /// The menu button pressed and not yet taken.
    open: bool,
    /// Zooming (the map): the pad's shoulder buttons and triggers (as buttons), -1 out to
    /// 1 in.
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
    /// The focus moved since the last pass: scroll to it.
    scroll: bool,
    /// The device last used: no ring while it's the mouse.
    device: Device,
}

/// The device the menus were last worked with.
pub fn device(ctx: &egui::Context) -> Device {
    ctx.data_mut(|d| d.get_temp::<Scopes>(scopes_id()).unwrap_or_default().device)
}

fn scopes_id() -> egui::Id {
    egui::Id::new("menu scopes")
}

/// Changes what the callbacks keep.
fn scopes_mut(ctx: &egui::Context) -> ScopesGuard<'_> {
    let scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
    ScopesGuard { ctx, scopes }
}

struct ScopesGuard<'a> {
    ctx: &'a egui::Context,
    scopes: Scopes,
}

impl std::ops::Deref for ScopesGuard<'_> {
    type Target = Scopes;
    fn deref(&self) -> &Scopes {
        &self.scopes
    }
}

impl std::ops::DerefMut for ScopesGuard<'_> {
    fn deref_mut(&mut self) -> &mut Scopes {
        &mut self.scopes
    }
}

impl Drop for ScopesGuard<'_> {
    fn drop(&mut self) {
        let scopes = std::mem::take(&mut self.scopes);
        self.ctx.data_mut(|d| d.insert_temp(scopes_id(), scopes));
    }
}

/// Hooks into egui's passes: each pass starts by scrolling a newly focused widget into
/// view and ends by drawing the ring round the focused one.
fn install(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    *done = true;
    hook(ctx);
    Ok(())
}

/// Adds the hooks to egui's passes.
pub(crate) fn hook(ctx: &egui::Context) {
    ctx.on_begin_pass(
        "menu focus",
        std::sync::Arc::new(|ui: &mut egui::Ui| {
            let ctx = ui.ctx();
            let focused = ctx.memory(|m| m.focused());
            let mut scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
            scopes.last = std::mem::take(&mut scopes.this);
            // A newly focused widget is scrolled into view by its menu (`claim`).
            scopes.scroll = focused != scopes.focused;
            scopes.focused = focused;
            ctx.data_mut(|d| d.insert_temp(scopes_id(), scopes));
        }),
    );
    ctx.on_end_pass(
        "menu focus",
        std::sync::Arc::new(|ui: &mut egui::Ui| {
            let ctx = ui.ctx();
            let scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
            if scopes.device == Device::Mouse {
                return;
            }
            let Some(response) = ctx
                .memory(|m| m.focused())
                .and_then(|id| ctx.read_response(id))
            else {
                return;
            };
            if !scopes
                .this
                .iter()
                .any(|(_, layer)| *layer == response.layer_id)
            {
                return;
            }
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("focus ring"),
            ));
            // Only what's in view of it (a row half scrolled out of its list).
            if !response.interact_rect.is_positive() {
                return;
            }
            let painter = painter.with_clip_rect(response.interact_rect.expand(6.0));
            painter.rect_stroke(
                response.rect.expand(2.5),
                7.0,
                egui::Stroke::new(2.5, super::ACCENT),
                egui::StrokeKind::Outside,
            );
        }),
    );
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
    let response = focused.and_then(|id| ctx.read_response(id));
    let here = response.as_ref().is_some_and(|r| r.layer_id == layer);
    // A widget not drawn yet (asked for the focus, as coming back to a screen asks for
    // the row gone into) keeps it: if it doesn't turn up, egui drops the focus at the
    // end of the pass and the next claim hands it on.
    let unknown = focused.is_some() && response.is_none();
    if here || unknown || (focused.is_some() && ctx.text_edit_focused()) {
        return;
    }
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
        m.move_focus(egui::FocusDirection::Next);
    });
}

/// Called at the start of a menu's scrolling list: a widget in it that got the focus since
/// the last pass is scrolled into view (a scroll area only takes what's asked inside it).
pub fn follow_focus(ui: &egui::Ui) {
    let ctx = ui.ctx();
    let layer = ui.layer_id();
    let Some(response) = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
    else {
        return;
    };
    if response.layer_id == layer && std::mem::take(&mut scopes_mut(ctx).scroll) {
        response.scroll_to_me(None);
    }
}

/// Whether a menu holds the focus and whether it's a text field, from the last pass.
fn focus_state(mut contexts: EguiContexts, mut state: ResMut<MenuFocus>) -> Result {
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    let scopes: Scopes = ctx.data_mut(|d| d.get_temp(scopes_id()).unwrap_or_default());
    let focused = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id));
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
    let mut pad_held = [false; 4];
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
    // Pads, while nothing is being bound: their standard buttons, in a menu always, else
    // unless the profile in use drives with them (B mustn't open the menu while it's the
    // handbrake).
    let capturing = capture.0.is_some();
    let profile = config.profile();
    for (name, pad) in pads.iter().filter(|_| !capturing) {
        let bound = |button: GamepadButton| {
            !focus.active
                && profile.bindings.iter().any(|(action, b)| {
                    !matches!(action, Action::Menu(_))
                        && b.is_on(name)
                        && b.input == Input::Pad(GamepadInput::Button(button))
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
            pad_held[i] |= down(button);
        }
        confirm |= pressed(PAD_CONFIRM);
        back |= pressed(PAD_BACK);
        open |= pressed(PAD_MENU);
        let zoom_in = down(GamepadButton::RightTrigger) || down(GamepadButton::RightTrigger2);
        let zoom_out = down(GamepadButton::LeftTrigger) || down(GamepadButton::LeftTrigger2);
        zoom += f32::from(u8::from(zoom_in)) - f32::from(u8::from(zoom_out));
        if pad.digital().get_just_pressed().next().is_some() {
            device = Some(Device::Pad);
        }
    }
    // The wheel's buttons bound to the menu actions.
    if !capturing {
        let dirs = [Nav::Up, Nav::Down, Nav::Left, Nav::Right];
        for (i, nav) in dirs.into_iter().enumerate() {
            held[i] |= wheel.nav_held(nav);
            pad_held[i] |= wheel.nav_held(nav);
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
    let digital = |held: [bool; 4]| {
        Vec2::new(
            f32::from(u8::from(held[3])) - f32::from(u8::from(held[2])),
            f32::from(u8::from(held[1])) - f32::from(u8::from(held[0])),
        )
    };
    input.pressed = pressed;
    input.held = digital(held);
    input.pad_held = digital(pad_held);
    input.confirm = confirm;
    // Typing, "back" is the field's (the keyboard's Esc goes to it anyway).
    input.escape = back && focus.text;
    input.back = back && !focus.text;
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
    (capture, just): (Res<Capture>, Res<JustBound>),
    mut contexts: EguiContexts,
    mut egui_input: Query<&mut EguiInput, With<PrimaryEguiContext>>,
) -> Result {
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    ctx.data_mut(|d| {
        let scopes = d.get_temp_mut_or_default::<Scopes>(scopes_id());
        scopes.device = input.device;
    });
    let Ok(mut egui_input) = egui_input.single_mut() else {
        return Ok(());
    };
    // A key being bound is the binding's, not the menu's (Enter would choose the row and
    // start binding again).
    if capture.0.is_some() || just.0 {
        egui_input
            .events
            .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
        return Ok(());
    }
    // Only Enter chooses: egui would click the focused widget on Space too (typing a
    // space comes as text, which stays).
    egui_input.events.retain(|e| {
        !matches!(
            e,
            egui::Event::Key {
                key: egui::Key::Space,
                ..
            }
        )
    });
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
    if input.escape {
        keys_out.push(egui::Key::Escape);
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
