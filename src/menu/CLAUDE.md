# Notes: Menus

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/menu/`; keep it up to date.

## Menus (agreed with the user, 2026-11-23; built the same day)

The egui windows weren't it. Every menu must be fully usable with six buttons (up, down,
left, right, choose, back) plus a menu button, so a gamepad or a wheel alone does
everything (except typing: chat, name, server address, a run's name); the mouse keeps
working. Sizes don't follow the display scaling (a 4K screen showed everything tiny).
One central menu (Esc, a pad's Start, or a wheel button bound to "Menu") holds what a
driver needs, so whoever doesn't know the keys, or whose controller lacks a button,
finds everything there. Decisions: **no pause and no parking brake** while it's open
(not even alone); the 0 (mute) key went (sound is in the menu), H and M stay; shops, the
dyno (ECU grids too) and events got the same treatment. The font was my choice: Ubuntu
Regular and Medium (`assets/fonts`, Ubuntu Font Licence; egui's own Light is too thin).

As built (`src/menu/`):
- `mod.rs`: `LookPlugin` (in `ui::UiPlugin`): egui's zoom so a point is a pixel in a
  1280×720 window and grows with the window (`pixels_per_point`: the smaller of
  height/720 and width/1280, times `Display::ui_size`, saved in `display.cfg`), whatever
  the display's scaling; fonts and style; selectable labels off (one took the focus and
  dropped it). `to_points` turns `Camera::world_to_viewport` positions into points (the
  scenes' egui-painted screens and the shops' floating labels need it).
- `nav.rs`: `MenuInput` from every device (keyboard arrows/Enter/Esc; any pad's D-pad,
  South, East, Start, unless the driving has them bound (no axes, not even the left
  stick, since 2026-11-26: pedals report as sticks, and the user's clutch scrolled the
  menus; the map zooms with shoulders and triggers as buttons); the wheel's buttons
  bound to `Action::Menu(Nav::…)`), held directions repeating (0.4 s, then 0.08 s). Pad
  and wheel directions and choose go into egui as arrow keys and Enter (`hand_to_egui`),
  so **egui's own focus navigation** moves between widgets. A menu calls `claim(ui,
  rank)` (the kit's `Panel` does): the highest rank shown keeps the focus in it (its first
  widget gets it); a text field being typed in elsewhere keeps it. Lists scroll to a newly
  focused widget from inside their scroll area (`follow_focus`: a scroll area ignores
  targets set before it); the ring is drawn at the end of the pass, clipped to what's in
  view, not while the mouse is in use. While a menu has the focus, `ShieldPlugin` keeps
  keys from the game (egui wants the keyboard) and `read_wheel` (which runs before the
  shield) takes nothing from keys or pad/wheel buttons, pedal actions included (a key
  bound to throttle revved the engine in the menu, 2026-11-23); axes and the shifter's
  gates go on. Text fields are `kit::text_field` (asked for by the user, 2026-11-23):
  a row the arrows pass over; choose starts typing, Enter keeps it, Esc (the field takes
  Esc itself: egui would drop the focus and `claim` give it to the first row) throws it
  away, the focus back on the row; a pad's or wheel's back while typing is Esc. "Back" and the menu button are taken in
  order (`MenuSet`: Top = central menu and quick start, Windows = shops, scenes,
  lobbies, the map, Open = a back nobody took opens the central menu).
- `kit.rs`: `Panel` (title, scrolling contents, prompt line naming the device's buttons;
  `Place::Center` or `Right`), rows: `button`, `button_with`, `submenu`, `submenu_with`
  (showing its value), `toggle`, `choice`, `pick` (any values), `slider` (left/right
  step), `swatches` (one widget the arrows pick in, for a picker screen of its own: the
  arrows stay in it), `progress`, `section`, `note`, `info`, `warning`. Left/right on a
  toggle, choice or slider lock the horizontal arrows (`set_focus_lock_filter`). The ECU
  tables are one focusable widget with a cursor, leaving at their top and bottom edges
  (`move_focus`). Ranks: window 10, start menu 20, central 30, intro 40. Only Enter
  chooses (`hand_to_egui` drops egui's Space key events; typing a space is text).
  Focusable widgets go through `kit::allocate`: a disabled one gets another id, since
  egui keeps an id among the arrows' targets while it's drawn, and a row disabled under
  the menu (dyno buttons while holding, the garage's build) took the focus from the
  arrows and dropped it, `claim` giving it to the first row: stuck there (2026-11-23).
- `central.rs` (restructured at the user's wish, 2026-11-23): Main = In-game actions
  (reset, map, camera: pressed for the driver through `input::MenuPresses` as the
  wheel's next frame), Settings (Driving aids, Controls, Sound
  (`audio::SoundSettings`: on/off, volume, `sound.cfg`), Display (UI size, the driver's
  and chase view's vertical field of view (`Display::driver_fov`/`chase_fov`, degrees,
  `camera::apply_fov` puts them on the lens unless a shop's scene has it; asked for by
  the user, 2026-11-26), fullscreen),
  Graphics (see "Graphics")),
  an event round's restart/leave, online players/leave or join, help and credits, quit
  ("Back to driving" removed at the user's wish, 2026-11-23: the prompt line says how). No engine or parking brake rows, no fullscreen on
  Main (the user: not wanted there). Controls: the profile in use (a choice), its name,
  its bindings (with a guided setup of the menu buttons, fill with the keyboard's
  defaults, clear), force feedback, the fixed keys. Going into a screen starts at its
  first row; back focuses the row gone into (rows on different screens share egui's
  automatic ids: the push clears the focus, and `claim` keeps a focus asked for before
  its widget is drawn).
- **Control profiles** (asked for by the user, 2026-11-23; `input.rs`): keys are
  bindings like pad inputs (`Input::Key`/`Input::Pad`); `WheelConfig::profiles` (4) and
  `active`; the first is `Profile::keyboard()` (WASD (no arrows since 2026-11-23, at the
  user's wish: they're the menus'; a `wheel.cfg` without `keys 2` has them taken out of
  the first profile), Space, Q/E, I, R, C, B, M, left Shift full power), the others start empty ("set one up for a gamepad, one for a wheel"); each action
  up to `PER_ACTION` (2) inputs, binding a third drops the oldest, an input belongs to one
  action per profile, except that an axis's two halves can go to two pedals (a stick
  up throttle, down brake; asked for by the user, 2026-11-24: `Binding::other_half`, same
  rest, opposite `full`; `Profile::labels` adds "up"/"down" then). Only the profile in use drives (`read_wheel` turns all of it into
  `WheelInput`; `drive`, the camera, reset and the map read only that). Steering by keys
  or buttons is `Action::SteerLeft`/`SteerRight` (rate-limited as the keys were), by an
  axis `Action::Steer`. Looking around by a stick (asked for by the user, 2026-11-24):
  `Action::LookSide`/`LookUp`, whole axes like steering (`Action::whole_axis`; bound by
  pushing left/up, no keys), `WheelInput::look` (dead zone `LOOK_DEADZONE`); the camera
  adds the smoothed push to the mouse's look (driver: head turn up to 2.2 rad; chase:
  swings round the car up to π), back to the front when let go. Fixed, never bindable: Esc (menu), H, T, 1, 2, F11, and the
  menus' arrows/Enter. `wheel.cfg`: `active N`, `profile N name`, then its `bind` lines
  (`key:KeyW`, `axis:…`, `button:…`); a file from before has its bindings put in profile
  2 ("Wheel"), in use. Keys and buttons bind when let go, and only one pressed
  during the binding (the Enter that began it isn't bound); bound on the press, it
  counted at once as what it was bound to ("Menu back" left the bindings; the user,
  2026-11-26). While a key is bound (and the frame after), egui gets no key events. A pad's standard buttons work the menus while one has the focus even if the
  profile drives with them, outside only when it doesn't.
- The map (M) moves with the arrow keys (asked for by the user): `map::take_arrows`
  takes them before `read_wheel`, so they don't steer while it's open.
- Scenes: `shops::SceneBack` hands "back" to the scene's window (back to the car);
  `shops::shop_window` is a right panel. The race bar while driving never claims (it
  would stop the driving); its buttons are for the mouse, the menu has the same.
- Flicker (reported 2026-11-23): natively the egui panels and the scenes' egui-painted
  screens flickered for a few seconds in shops. Cause: egui's font atlas (glyphs per
  exact size; the screens' perspective text made new sizes as the camera moved, filling
  the atlas until egui rebuilt it and re-sent the whole font texture). Fixed by
  `menu::scene_text` (a ladder of sizes), confirmed by the user: **any text whose size
  follows the camera must go through it**. Under lavapipe the rebuilds showed no gap, so
  shots can't catch it. Still open: in Firefox (wasm) black rectangles flickered over
  the lower half of the screen, not following geometry (not investigated).
- Gotchas: under Xvfb the window never has focus, and egui's `Response::has_focus` asks
  for it: the kit uses `kit::focused` (egui's memory), so shots show the focus; egui's own
  sliders don't take arrows in shots. Shots press keys for egui with `TAPS=
  "ArrowDown*3@100;Enter@110;Escape@120"` (window key events; the keys argument only
  presses `ButtonInput`). Not done: driving with a gamepad needs its sticks bound (no
  default driving bindings for pads); typing on a controller.
