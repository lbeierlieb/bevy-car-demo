//! The chat on a server that has it on: the latest lines in the bottom left corner,
//! fading a while after they came; T opens a line to write in (Enter sends it, Esc or a
//! click elsewhere drops it), with all the recent lines shown while it's open. While
//! writing, the game gets no keys (`ui::ShieldPlugin`).

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, text::LayoutJob};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::net::protocol::CHAT_LENGTH;
use crate::net::{Connection, Status};

/// How long a line shows, and how long it takes to fade at the end of that (s).
const SHOWN: f64 = 20.0;
const FADE: f64 = 2.0;
/// The most lines shown, and how wide they are at most (points).
const LINES: usize = 8;
const WIDTH: f32 = 440.0;
/// Where a player's name is drawn, and the server's lines.
const NAME: Color32 = Color32::from_rgb(255, 214, 120);
const SERVER: Color32 = Color32::from_rgb(170, 190, 210);

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Writing>()
            .add_systems(Update, open)
            .add_systems(EguiPrimaryContextPass, show);
    }
}

/// The line being written, while it's open.
#[derive(Resource, Default)]
struct Writing {
    open: bool,
    text: String,
    /// Just opened: the field takes the keyboard.
    focus: bool,
}

/// Whether there's a chat: on a server that has it on.
fn chatting(connection: Option<&Connection>) -> bool {
    connection.is_some_and(|c| c.chat_on && matches!(c.status, Status::Joined { .. }))
}

/// T opens it, also while a menu has the focus (a lobby is where people chat), though
/// not while some other text field is being typed in. The window's key events, since a
/// menu with the focus keeps the keys from the game.
fn open(
    mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>,
    focus: Option<Res<crate::menu::nav::MenuFocus>>,
    connection: Option<Res<Connection>>,
    mut writing: ResMut<Writing>,
) {
    let typing = focus.is_some_and(|focus| focus.text);
    let t = keys
        .read()
        .any(|key| key.key_code == KeyCode::KeyT && key.state.is_pressed() && !key.repeat);
    if t && !typing && chatting(connection.as_deref()) && !writing.open {
        writing.open = true;
        writing.focus = true;
    }
}

fn show(
    mut contexts: EguiContexts,
    connection: Option<ResMut<Connection>>,
    mut writing: ResMut<Writing>,
    time: Res<Time<Real>>,
) -> Result {
    let Some(mut connection) = connection.filter(|c| chatting(Some(c))) else {
        *writing = Writing::default();
        return Ok(());
    };
    let now = time.elapsed_secs_f64();
    let open = writing.open;
    // The latest lines, as visible as they still are (all of them while writing).
    let lines: Vec<_> = (connection.chat.iter().rev())
        .map(|line| {
            let left = SHOWN - (now - line.at);
            let alpha = if open {
                1.0
            } else {
                (left / FADE).clamp(0.0, 1.0) as f32
            };
            (line.clone(), alpha)
        })
        .take_while(|&(_, alpha)| alpha > 0.0)
        .take(LINES)
        .collect();
    if lines.is_empty() && !open {
        return Ok(());
    }
    let font = FontId::proportional(15.0);
    egui::Area::new(egui::Id::new("chat"))
        .anchor(Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
        .order(egui::Order::Foreground)
        .interactable(open)
        .show(contexts.ctx_mut()?, |ui| {
            ui.set_max_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (line, alpha) in lines.into_iter().rev() {
                let fade = |c: Color32| c.gamma_multiply(alpha);
                let mut job = LayoutJob::default();
                job.wrap.max_width = WIDTH - 12.0;
                let format = |color: Color32| egui::TextFormat {
                    font_id: font.clone(),
                    color: fade(color),
                    ..default()
                };
                match &line.from {
                    Some(name) => {
                        job.append(&format!("{name}: "), 0.0, format(NAME));
                        job.append(&line.text, 0.0, format(Color32::WHITE));
                    }
                    None => job.append(&line.text, 0.0, format(SERVER)),
                }
                egui::Frame::new()
                    .fill(fade(Color32::from_black_alpha(150)))
                    .corner_radius(3.0)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| ui.label(job));
            }
            if !open {
                return;
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut writing.text)
                    .char_limit(CHAT_LENGTH)
                    .desired_width(WIDTH)
                    .font(font.clone())
                    .hint_text("Enter: send    Esc: cancel"),
            );
            // After the field took this frame's input, so the T that opened it isn't
            // typed into it.
            if std::mem::take(&mut writing.focus) {
                response.request_focus();
            }
            if response.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    connection.write(&writing.text);
                }
                writing.open = false;
                writing.text.clear();
            }
        });
    Ok(())
}
