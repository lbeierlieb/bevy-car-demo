# Notes for Claude Code

Instructions and background for working on this repo. The sandbox VM only keeps the
repo, so everything worth knowing between sessions belongs here.

## Working conventions

- Atomic commits: one logical change each, committed as soon as it works, with clippy
  and tests passing for every commit (CI runs clippy with `--deny warnings` for native
  and wasm). Don't leave uncommitted files around.
- Git author is set repo-locally (Lukas Beierlieb). Commit messages end with a
  `Co-Authored-By:` line.
- The user builds and runs the game outside the sandbox at
  `/home/lu/git-projects/bevy-car-demo` (NixOS, Fanatec CSL DD wheel base).
- Respect "stop"/"wait" messages immediately.
- Keep this file up to date when something worth knowing next session comes up.

## Apps (agreed with the user)

The game is a library (`src/lib.rs`) and apps, each the list of plugins it adds: the
**sandbox** (`src/main.rs`: the shared `CorePlugin` (`WorldPlugin` (terrain, obstacles,
simulation, traffic: all the server has too), scenery, input, driving, visuals, camera,
HUD, map, sound, egui), help (quick start, hints), the car the server gives (its catalogue's start: the Vireo hot hatch, B16),
gizmos, 2 telemetry, the central menu (Esc), shops (online), F11 fullscreen in all apps). The career was removed for the multiplayer work (see
"Multiplayer"). Keep it that way: no mode flags; something only one app has is a plugin
of its own (e.g. `ui::PanelPlugin`), and
shared pieces are public functions (e.g. `workshop`, `garage::start`). Two apps, the
map editor (`src/bin/editor.rs`) and the race editor (`src/bin/races.rs`; both native
only), have no `CorePlugin` (no cars): `editor::EditorPlugin` and
`race_editor::RaceEditorPlugin` add the terrain, scenery, sky and map plugins
themselves (see "Race editor").

- Window keys are on the number row (moved from F1–F6 at the user's wish, 2026-11-19):
  1 gizmos (with the car's map x, z, height and heading top right, for placing things in
  the server's lists; the map (M) shows the pointer's x, z at the bottom), 2 telemetry (3–6 were tool windows until the shops, 2026-11-22; the settings moved into the central menu, Esc, 2026-11-23). The F
  keys belong to the browser on the web
  (`web/index.html` keeps them from winit's canvas listeners); natively F11 toggles
  fullscreen (`FullscreenPlugin` in `default_plugins`). The editor keeps F5 (bake).
- Input while typing (agreed with the user, 2026-11-21: generic, no per-key checks):
  `ui::ShieldPlugin` clears all keys while egui wants the keyboard, and in the driving
  apps the mouse's buttons, motion and wheel while egui uses the pointer, so game
  systems just read `ButtonInput` and never ask `EguiWantsInput`. The wheel and pedals
  pass. The editor shields only the keyboard (its systems check the pointer themselves).
- rustfmt (asked for by the user): the whole repo is rustfmt-clean since 2026-11-17.
  Always run `nix develop -c cargo fmt --all` before every commit (and `cargo fmt` in
  `tools/bake` and `tools/dev/fakewheel`, `rustfmt --edition 2024 tools/dev/shot.rs`
  if touched). Mind that a format change in `cycle.rs` or `maps.rs` needs a re-bake.

## Beginner help (agreed with the user, 2026-11-20)

Made to be easy to pick up: `help.rs` (`HelpPlugin`, sandbox). At every start a
quick start (a `kit::Panel`, rank 40; Esc or "Let's drive" closes it, H reopens it; the
start menu waits for it; made a quick start at the user's wish, 2026-11-23) has only the
essentials: how to drive off (neutral, parking brake on: E, W), W/S, A/D, E/Q,
Space, R and Esc (`QUICK_START`); the rest is in the menu; the telemetry panel (2) is hidden by default. Hints over the road say what's wrong, what to press and what
that does: reverse asked for while rolling
(`car::ReverseRefused`, a message from `drive`), on the rev limiter in first gear (only
there, at the user's wish, 2026-11-22), and
gas without moving for 3 s (R). (The parking brake hint went when gas started releasing it, 2026-11-24.) Added alone: in neutral, stalled, wrecked engine, empty
tank (stating the problem only, since 2026-11-22). R (a press in the step's input, for the server) puts the car on the nearest road, as it is, (`Terrain::drive_spot_along`: the
right-hand lane on roads with traffic lanes; 8–25 m along the road from the nearest point,
either way at random (`rescue::ALONG_ROAD`, hashed from the step and the car's place), the other way
if the road ends first, so a car stuck on a road isn't put back into the trap; 2026-11-26), carried there by the rescue helicopter (see
"Helicopters"). The map teleport was removed at the user's
wish (2026-11-21). Keys (the keyboard profile's): M map, B (held) looks back; Esc the central menu (the 0
mute key went into it, 2026-11-23).

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
  the first profile), Space, Q/E, I, R, C, B, M), the others start empty ("set one up for a gamepad, one for a wheel"); each action
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

## Sandbox workflow

- Build inside the dev shell with a separate target dir (the shared `target/` broke):
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo clippy --all-targets -j 4 -- -D warnings'`.
  Touch `src/main.rs` first to make clippy actually re-check. Wasm:
  `cargo clippy --target wasm32-unknown-unknown --all-targets -j 4 -- -D warnings` (as the flake's `clippy-wasm` check).
- Tests: `cargo test --profile release-dev` (no LTO, incremental; the physics tests need
  the optimisation). All tests are in the library: the apps have `test = false` and the
  library `doctest = false`, so the crate is built once, with the tests (34 → 17 s for a
  full rebuild of it; a small change about 10 s).
- `cargo run` in the dev shell is a wrapper for `--profile release-dev`; build that
  profile, not `dev`, before screenshots.
- No python/perl installed: `nix shell nixpkgs#python3 -c python3 -I`.
- 4 cores, 7.8 GB RAM, no swap: a full `nix flake check` got OOM-killed once.
- Anything that copies the repo flake into the Nix store (`nix build .#…`,
  `builtins.getFlake`) fails with "No space left on device": `target/` is over 20 GB.
  `nix build 'git+file:///repo#…'` works (only tracked files; it built
  `checks.x86_64-linux.nixos-module`, server included, in ~10 min).
  To look at nixpkgs, use the pinned rev from `flake.lock` directly, e.g.
  `nix build 'github:NixOS/nixpkgs/<rev>#linuxPackages.hid-fanatecff'`.
- Passwordless sudo works.
- Never search the whole filesystem (`find /` and the like): the read-only part of
  `/nix/store` is shared from the host over virtiofs, and walking it exhausts the host's
  file descriptors. Every program then fails with "cannot open shared object file:
  Error 23". To recover, push the inode cache out with memory pressure from bash alone
  (`s=x; for ((i=0;i<30;i++)); do s=$s$s; done; unset s`, about 1 GB), then
  `sudo sh -c 'echo 2 > /proc/sys/vm/drop_caches'`.
- Dependency sources (no `~/.cargo/registry`): they're tarballs in the store, e.g.
  `ls -d /nix/store/*cargo-src-egui-0.36*`; unpack into the scratchpad with `tar xzf`.
- Terrain bake tool: `cd tools/bake; nix develop /repo -c cargo build --release; ./target/release/bake …`
  (commands in README, "The map and baking it"; a bake takes about 20 s). The native
apps load `assets/gerach.terrain` at startup, so they see a re-bake at once (tests use
the embedded copy, `Terrain::baked`, the fallback otherwise); commit the re-baked file.
- Engine maps of every factory engine of the built-in catalogue (B16 and B18 since
  2026-11-26, see `catalogue/engines.rs`), plus two engines tests measure against (W20,
  `EngineHardware::default`; B16T, `EngineHardware::turbo`), are baked into
  `src/physics/engine/baked/<code>.maps` (listed in `engine/store.rs`'s `BAKED`). After changing `cycle.rs`, `maps.rs` (outside
  their tests; any edit counts, their text is in the fingerprint) or a factory engine's
  cylinder side, re-bake all of them (a few seconds) or
  `stock_maps_are_current` and the tests that expect baked maps fail:
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo test --profile release-dev bake_stock_maps -- --ignored'`.
  A new factory version needs an (empty) file there first, for `include_str!`, and a
  line in `BAKED`.

### Screenshots (`tools/dev`)

`tools/dev/unstrip.sh` copies `tools/dev/shot.rs` into `src/` and hooks it into
`lib.rs` and the apps; `tools/dev/shot.sh out.png [frames] [keys]` renders headless and
saves a screenshot (`BIN=editor` for the editor); `tools/dev/strip.sh` removes the hook.
Always run `strip.sh` and check that `grep -cw shot src/lib.rs src/main.rs
src/bin/*.rs` prints 0 for each before committing. Keys are pressed from a third of
the frames until the shot (`m` opens the map, `2` shows the telemetry panel, `f` shows the gizmos (off by default), `c` the driver's view, `b` looks back; keys stay held through the shot's frame),
so clicks on a window must come after that (`T` presses the key T, `E` Enter).
Shots play alone (on the game's own server, with the repo's shops and events) unless
`JOIN`, `MENU=1`, `OFFLINE=1` (no server, no traffic; the shot spawns the built-in
catalogue's first car itself, as no server gives one) or `LINEUP` (likewise, as a join
would move the car). `PLACE` works because `shot.sh` sets `CAR_SIM_TRUST_PLACEMENTS=1`
(the game's own server then takes up placements however far they move the car).
Shops need `PLACE` at the building's point
(the car lands on the marker and goes in): gas `-2603,-1736`, car dealer
`-1476,-1213`, alignment `-1480,-1305`, paint `3500,-63`, car workshop `-1566,-1329`,
tuning shop `-1950,-3160`, tire shop `-2502,-1763`, engine shop `120,2700`, dyno
`-2601,-3784`, garage park `-2680,-1793` (the shop's window takes the focus on its first row, so
`TAPS="Enter@200"` goes into the scene: onto the rollers, up to the pump…; older click
coordinates in these notes are from before the menus, 2026-11-23, and are wrong now;
the car hunt's marker `-95,2341`, its lobby window opens
at once; a race marker exactly (the repo list has none since 2026-11-24: add one with a
route; "Races Baunach" was at (−62, 2280)): `PLACE_EXACT=1 PLACE="-62,2280,150"`, then
`TAPS="Enter@200"` is "Time trial", `ArrowDown@200;Enter@204` "Race"; in the dyno's room
the page is the first row (`ArrowRight` steps it: runs, ECU tune, cams and boost), "Hold"
the third; a hold needs ~200 frames more to settle).
The editor (`BIN=editor`, hooked by `EditorShotPlugin`): `EDITCAM="x,z,height,yaw,pitch"`
(degrees, the first 20 frames), `CLICK="x,y,frame[,x2,y2]"` sets the window's cursor and
holds the left button (a drag with x2, y2), `UICLICK` is the window-event click for
egui (the map, buttons), `EDITKEYS="Digit2@25;ControlLeft+KeyS@60"` taps keys. A view
of a village road: `EDITCAM="-2986,-1954,45,0,-70"`, the road at (1000, 381). Saving
or baking in a shot writes the real map and terrain files: `git checkout map assets`.
`PLACE="x,z,frame"` puts the driven car on the road nearest that map point (joined, at frame 150 or later, or the join moves it: e.g. by a shop, `JOIN=127.0.0.1 NAME=Anna PLACE="-1476,-1180,150"`, 220 frames). Give a shot
its own config dir (`XDG_CONFIG_HOME=<scratch>/cfg`) for fresh settings. The start menu
is closed in shots unless `MENU=1`, the quick start before it unless `INTRO=1`; `JOIN=127.0.0.1 NAME=Anna` joins a server at once.
Two players: start the server (`target/sandbox/release-dev/server`, throwaway
certificate, in the background with `nohup`), then two shots with `CAR_SIM_INSECURE=1`
and their own config dirs, the second started a few seconds after the first, more
frames for the first (they take turns building). Don't `pkill -f` anything matching
your own command line (it kills the shell); `pgrep -x server`, then `kill`.
Likewise, never wait for a shot with `until … ! pgrep -f shot.sh` in a background
shell: the pattern matches the loop's own command line, so it never ends (three such
loops ran for hours, 2026-11-22). Wait for the output file, or `pgrep -x`/a PID.
`CLICK="x,y,frame;x,y,frame…"` clicks into the
1280×720 window, e.g. on the map. `CAM`/`CAMAT` env vars place the camera (see
`shot.rs`). `shot.sh` fetches Mesa and Xvfb from the pinned nixpkgs on first use.
A shot takes about 35 s.

For looking at cars: `CAR="chassis[,engine]"` starts in that car (its place in the
catalogue; bought from the server's showroom, `Buy`); `LINEUP="first,count"` parks that many cars one behind the other on an open,
flat straight (the spawn is in a village, where walls and slopes block the view);
`CARCAM="side,height,along"` looks at them from that far to their right (negative:
left, the sunlit side), up, and ahead. Side views of five:
`LINEUP=0,5 CARCAM="-21,2,0" bash tools/dev/shot.sh out.png 60 2`; one car close up:
`LINEUP=3,1 CARCAM="-3.6,1.9,4.6"`; from the driver's seat: keys `2c`.
`TRAFFICCAM="back,height[,nth]"` follows the nth nearest traffic car from behind, e.g.
`TRAFFICCAM="9,2.2,1" bash tools/dev/shot.sh out.png 200 2`.
`PROXYCAM="distance,height,bearing"` looks at the other player's car (a named remote car) from that far
towards the bearing (degrees from north). Two players side by side (the second needs
`PLACE` after it has joined, or the join puts it elsewhere): Anna with
`PLACE="-2986,-1954,150"` and many frames, Ben with `PLACE="-2986,-1975,150"` sees her
ahead. Egui headers can't be clicked open in shots (change `default_open` for a shot).

### Virtual wheel (`tools/dev/fakewheel`)

A uinput device named "Fake Wheel" with steering, three pedal axes, buttons and
FF_CONSTANT. It runs a script over wall-clock time (axes wiggle at 20 s so the game
registers them, gate 1 at 32 s, clutch out with throttle from 40 s, steer from 90 s)
and logs the force levels the game uploads.

```sh
cd tools/dev/fakewheel && nix develop /repo -c cargo build --release && cd /repo
sudo tools/dev/fakewheel/target/release/fakewheel > /tmp/fakewheel.log &
bash tools/dev/unstrip.sh
XDG_CONFIG_HOME=/repo/tools/dev/wheel-config WHEELLOG=1 bash tools/dev/shot.sh /tmp/wheel.png 600
bash tools/dev/strip.sh; sudo pkill fakewheel
```

`tools/dev/wheel-config` holds bindings for the fake wheel (note: the game saves
changed settings back there). Gotchas found with it:

- The game runs at a few frames per second under lavapipe; the script runs in wall time.
- Xvfb never focuses the window; `WHEELLOG=1` forces focus so force feedback runs.
- gilrs flips Y axes (ABS_Y reads inverted); binding by moving the control handles it.
- The kernel only reports changes, and gilrs doesn't read initial state: a control
  counts once it has moved.

## Career (removed 2026-11-19, to come back on top of multiplayer)

The first career (`src/career/`, `src/bin/career.rs`) is in git history up to c34bfbf.
Its agreed design, to revisit for multiplayer: start with €500 and mom's car (always full
and repaired); home with a garage that is its own 3D scene; gas station; pizzeria
(deliveries against the clock); car dealer (buy cars, deliver cars, trust unlocking
nicer cars and pay); parts shop (parts wait in the garage to be fitted, never touching
the ECU); a chassis dyno in its own scene (through the gears to fourth, then a full
pull; wheel and crank power, runs saved with names and notes); towing home, or mom
fetching you; 3 own bays, one save slot.

## Shops (agreed with the user, 2026-11-21; built 2026-11-22)

The tool windows (3–6) became places on the map to drive to; the keys 3–6 are gone.
`src/shops.rs`. **The server says where they are**: its own list, separate from the map
(`server --shops FILE`, `services.car-sim.shops = FILE`; none without one, at the
user's wish, 2026-11-22; the repo's list is `assets/shops.cfg` (tests read it): lines
`kind x z name`, the building's map point; one of each in Baunach and Rattelsdorf, and
shared by Gerach and Reckendorf, 1.6 km apart; names made up, never a real business or brand, at the user's wish), sent as `ToClient::Shops` after the
welcome (protocol 6). Alone they're the repo's lists (the game's own server). The client puts
each marker (glowing disc, 3.5 m, and a beam) on the nearest paved road at the edge on
the building's side (`kerb`); names float above within 500 m. Free (no money). The plan
as built, in order:

1. ✓ server list, protocol, markers and names. 2. ✓ Stop on a marker to go in (window opens,
   parking brake; drive off or Esc leaves). 3. ✓ **Ghosts**: a car stopped on a marker,
   in a shop or in the dyno is a ghost (a flag in its state): no collisions with players'
   cars or traffic (the server's traffic ignores it), drawn half transparent; it stays a
   ghost after leaving until it overlaps nobody. So several players use a shop at once.
   (`CarState::ghost`; `shops` sets `net::WantsGhost`, `client::haunt` puts `looks::Ghost`
   on our car, `move_cars` on remote ones; `push_off_remote_cars` skips ghosts, the server
   decides ghosts itself since 2026-11-24 (`server::haunt`, see "Server authority") and
   removes a ghost's `traffic::Visitor`; `visuals` swaps in a second, blended material set.)
4. ✓ Shops on the map (M), with a legend (`map::MapLegend`); their names only when zoomed in (under 1.6 m per screen point, `MapMark::label_below`; at the user's wish, 2026-11-22). 5. ✓ **Gas station**: 95/98/102; the tank keeps
   its octane, mixing by volume; the engine's knock limit reads the tank's octane (the
   panel's octane and fuel load sliders go). Since 2026-11-22 (asked for by the user) a
   scene like the alignment bay's (`gas.rs`): "Up to the pump" puts the car (drawn from
   `Looks`, engine off) at a pump on a forecourt 600 m **over** the marker (open air: a
   2.4 km meadow hides the land below; canopy, three islands, the shop, trees), the
   filler flap open on the right over the rear wheel (front wheel when the tank is up
   front), nozzle and a hanging hose from the pump; the pump's display (egui-painted in
   perspective, point by point) shows grade, litres of this fill and the tank; the grade's
   button lights and its nozzle leaves the holster. The window on the right (simplified
   at the user's wish, 2026-11-23): the grade, then "Pump" (at `FLOW`, 1 L/s, until
   stopped or full; while it pumps the focus is locked on it, now "Stop", and back does
   nothing), what's in the tank, back to the car. Shots: `TAPS="Enter@200"` is "Up to the
   pump", then "Pump" is the pump window's second row (`ArrowDown@236;Enter@240`);
   `FUEL=12` empties the tank to 12 L at frame 160. 6. ✓ **Alignment shop**: camber, pressures
   (from the garage), toe and brake torque/balance (from the old panel; into `CarBuild`).
   Since 2026-11-22 (asked for by the user) a scene like the dyno's (`alignment.rs`): the
   shop's window has "Onto the alignment lift", a bay 220 m under the marker with a
   four-post lift (turn plates, slip plates, ramps), the car on it drawn from the driven
   car's `Looks` (steer and spin zeroed), a head clamped on each wheel (the clamp tilts
   with the wheel's pose, the head hangs level), red beams along the sides and across
   under the bumpers, and the console's screen (egui-painted like the dyno's) with each
   wheel's camber (measured from the pose, so the car's weight shows), toe and pressure
   against the spec (kit camber ±0.5°, `FACTORY_TOE` ±0.1°, suited pressure ±0.15 bar),
   green or red. The window on the right is `garage::alignment`, applied live to the
   parked car (`car::Away` set, as in the dyno); drag swings, the wheel zooms. Shots: `TAPS="Enter@200"`
   onto the lift; the rows are back to the car, then camber, toe, pressure per axle.
7. ✓ **Paint shop**: the shop paints (swatches; gone from the garage). Every chassis
   leaves the factory in one dull paint, no choice (asked for by the user, 2026-11-22, so
   there's a reason to paint): `PAINTS` is the 10 shop paints (`SHOP_PAINTS`), then 11
   factory ones (beige, putty, primer grey…; the paint shop never offers those); traffic
   is in a factory paint (any, since there's one chassis), a `PAINTED` fifth in a shop paint (so a painted car can still
   hide in it). Protocol 15. Since
   2026-11-22 (asked for by the user) a scene like the alignment bay's (`paint.rs`): "Into
   the paint booth" puts the car (drawn from `Looks`, engine off) in a spray booth 240 m
   under the marker, sized to the car (white walls with seams, glowing filter ceiling and
   angled light rows, grates over pits along both sides, folding glass doors ahead, a
   yellow filter bank at the back, control panel and door on the left, a shelf of tins
   (lids in each paint, the picked one pulled out) and the gun on its hook on the right, a
   test card on a stand by the right front wheel in the picked paint). The window on the
   right: "Colour" (a row opening a picker screen of swatches: the arrows pick, the booth
   showing it, choose keeps, back goes back to the one before; at the user's wish,
   2026-11-23: in the list, up from the swatches always picked another colour first),
   "Spray it X" sprays a coat (`SPRAY` 6 s): a mist of the paint (12
   nested unlit ellipsoid shells round the body, `cull_mode` none, plus 600 droplets going
   round and sinking) thickens, the paint goes onto the parked car at `THICKEST` (3 s,
   `shops::rebuild`), then it clears. Shots: `TAPS="Enter@200"` into the booth; "Colour" is the first
   row (`Enter` opens the picker), "Spray it" the second; use `STEP=0.05`
   (60 frames to the thickest). 8. ✓ **Showroom**: since 2026-11-22 (asked for by the user) a scene like the gas
   station's (`garage/showroom.rs`): "Into the showroom" puts you in a hall with glass
   front and left side, 600 m **over** the marker (meadow, forecourt with flags and a
   pylon, the driven car drawn from `Looks` waiting outside the glass). Every chassis
   stands on the floor in its factory paint at its own `place` (3 left, 7 back, 3 right;
   `traffic::FactoryLooks`); the one picked turns on a turntable under spotlights (its
   floor place hidden), a sign beside it egui-painted with name, class and engine. "Start
   it" idles the picked engine (the model's `Motion::engine`, heard by `hear_others`) and
   blips it every 3.5 s. The window on the right: the list and `brochure`; "Drive it"
   builds the car and goes straight out to it (once built), in neutral with the parking
   brake on (`fit` leaves every car put down afresh so). The camera stays within `REACH` (7.4 m) of the
   turntable, clear of the floor cars. Shots: `TAPS="Enter@200"` in, then the first row is the car
   (`ArrowRight` the next), then the engine, "Drive it", "Start it".
   (`shops::shop_window` draws a shop's window for other modules, saying when the car came in).
9. ✓ **Parts shop**, since 2026-11-26 **car workshops, tuning shops and tire shops**
   (see "Owning cars and parts", milestone 6: the fitting window). Since 2026-11-22
   (asked for by the user, "car on a lift") a scene
   like the alignment bay's (`garage/bay.rs`): "Onto the lift" puts the car (drawn from
   `Looks`, engine off, wheels hanging at `DROOP`) up on a blue two-post lift (`RAISE`
   1.3 m) in a workshop 260 m under the marker; the arms' pads go under the sills behind
   the front wheels and ahead of the rear ones, made afresh for another chassis. An engine
   like the car's stands on an engine stand in front of the car (family size and layout:
   inline, V or flat; a turbo if it has one), tires like its in a rack at the back;
   workbench with pegboard, tool chest, shelves of boxes, roller door, oil drain. The
   fitting window (`garage::fit`) is on the right. Shots: `TAPS="Enter@200"` onto the lift;
   the first row is back to the car, then the car's slots by group (Engine first).
   Shots: `TAPS="Enter@200"` onto the lift; the rows start with back to the car and building. 10. ✓ **Dyno** (`dyno/`): the career's chassis dyno room back
   (`mod.rs`, `rollers.rs`, `chart.rs`). A copy of the car runs in the room 200 m under
   the marker; the driven car stays parked there, a ghost (`car::Away` holds its input and
   R, `EngineAudio::hushed`); the engine (tune, cam timing, wastegate, wear) and the fuel
   go back to it on leaving. The window (up all the time) has tabs: Runs (kept for the
   session only), ECU tune (`ecu_ui::tuning`), Cams and boost (applied to the copy,
   baking maps if needed, tune and wear kept). Hold: up through the gears, then the
   rollers are motored to the speed for an rpm in fourth (`HOLD_RATE`) at a set throttle,
   power = what the wheels push with. Runs can't be skipped (at the user's wish,
   2026-11-23). The engine dyno window (5) is gone. 11. ✓ Clean-up:
   the panel lost its setup section (springs, dampers, upper pivot, tire μ: they follow
   from the parts); **R only puts the car back on the road** (`Car::place` keeps fuel,
   tires, engine wear, and leaves it in neutral with the parking brake on, so a teleport
   (rescue, a round starting or over, joining) never stalls it; agreed: no hint suggests anything for an empty tank or a wrecked
   engine, for now); `ui::tool_window` is gone; help, README.

Not done yet: dyno runs aren't saved; the HUD shows the parked car's gauges while in the
dyno's room.

## Events (agreed with the user, 2026-11-22; built the same day)

Like shops, places on the map that the server lists (`server --events FILE`,
`services.car-sim.events`; repo list `assets/events.cfg`, lines `kind x z name`, names
made up). Unlike a shop's, an event's marker is exactly at its point, not moved to a road
(the user wanted the hunt in the middle of a parking lot, 2026-11-22; the server checks
"near the marker" against the point too). Each round of an event runs in a **world of its own** (see "Parallel worlds"):
its players see no cars, traffic or shops of the normal world and the normal world sees
none of theirs; traffic, if on, is simulated for that world alone; the chat is per world.
Another group can start an independent round at the same marker while one runs. The
players list says who's in which event. Every game runs on the same frame
(`round.rs`: `Round { event, players, host, game, phase, best }`, lobby → running →
result; `Game`, `Play`, `Outcome` are enums per game; the host is the hunt's hunted or
whoever opened a race's lobby).

**Car Hunt** (first event, "Car Hunt Baunach" in the middle of the Rewe's parking lot in
Baunach, (−87, 2286), the `amenity=parking` area's centroid):

- Stopping on the marker joins the lobby forming there (or opens one). The first one in
  is the hunted, all later ones hunters. The hunted can click another name to make them
  the hunted, sets the options and starts (at least one hunter). No late joining.
- Options (the hunted's): head start (10 s), time until the hunted wins (10 min),
  traffic (on), "hunters can reset" (on; R calls the rescue
  helicopter, whose time is the penalty for crashing; "to the start" was dropped at the
  user's wish, 2026-11-22; the hunted can never reset), cars ("player cars" (default) or "factory
  cars": everyone is lent the start car, factory-built, full and on fresh tires (was
  "muscle vs rally" until the other chassis went, 2026-11-26); a lent car leaves no
  trace, players get their own car back afterwards as it was), how the hunters win
  (`Catch`: "Tag them" (default) or "Stop them"; added 2026-11-22, the user: tagging is
  the objective, with one or two hunters stopping the hunted is near impossible).
- Everyone starts near the marker; hunters are held for the head start. Name tags on and
  everyone in the round on the map, whatever the server's settings.
- Hunters win when the hunted's car is under 10 km/h for 10 s, anywhere (after the head
  start), or, with "Tag them", the moment a hunter's car touches the hunted's (or the
  other way round); the hunted wins when the time runs out. As built: each game's
  `push_off_remote_cars` collects the remote cars ours touched (`net::Touching`), the lobby
  sends them in a running tag round (`Action::Touched(CarId)`, at most every 0.2 s), and
  the server ends the round (`Ending::Tagged { by }`) if the car's owner and the sender are
  the hunted and a hunter, past the head start, their states within `TAG_RANGE` (15 m).
  Ghosts touch nothing, so cars that waited overlapping in the lobby tag only once apart.
- The hunted sees the nearest hunter's distance in the bar (from `Connection::on_map`,
  1 Hz, carried on at speed; rounded to 5/10 m, yellow under 200 m, red under 50 m). If the hunted leaves, the round ends.
- Afterwards everyone is back in the normal world at the marker, in the lobby menu (the
  next round's lobby there, if any). Fuel, tires and wear of a player's own car carry over.

As built: `traffic::Realm` (component; none = `Realm::NORMAL`) on peers and traffic cars;
`Traffic::realms` overrides the density per realm; `populate` and `step` work per realm;
the server's send loop, map and chat only within a realm, and in any realm but the normal
one tags are on and the map shows everyone. `src/events/`: `mod.rs` (list, `parse`,
markers, map marks: `MapMarks`/`MapLegend` are keyed by owner now), `hunt.rs` (rules,
`Settings`, `Outcome`, their wire), `round.rs` (`Round`, `Phase`, `Action`), `rounds.rs` (server:
`Rounds`, `run_rounds` in `ServerPlugin` between `receive` and `send`; gives and takes
only the realms it manages; one lobby per event, a round over merges into the waiting
one), `lobby.rs` (client: `arrive` asks to enter when stopped on a marker, drive 8 m or
Esc leaves; `follow` places the car at `online::lane_spots` near the marker by slot
(a lent car and our own back come from the server as `YourCar`), sets `Car::hold` and
`car::ResetRule`; `window`: lobby window and the bar at the top). Protocol 7:
`ToClient::Events`, `Round(Option<Round>)`, `Busy`, `ToServer::Event(Action)`; protocol 8
added `Settings::catch`, `Ending::Tagged`, `Action::Touched`. Times are
on the server's clock (`Connection::server_time`). Shops and event markers are hidden
in a running round (`Connection::in_running_round`). Tests: `realms_see_only_themselves`,
`a_car_hunt_runs_in_a_realm_of_its_own`, `a_hunt_lends_cars_and_gives_them_back`,
`a_hunter_tags_the_hunted` (test apps have no shops plugin, so `WantsGhost` stays set
from the lobby unless a test clears it).
Not done: a round's own chat lines (who caught whom) for the server's chat.

**Races and time trials** (asked for by the user, 2026-11-22; built the same day;
grid and gates by hand since 2026-11-24, the user: the automatic gates in Baunach
weren't great): a `race` event (blue marker) has routes listed under it in the events
file, made in the race editor: `route loop|sprint NAME`, then `grid x z heading` lines
(compass degrees; up to 16; pole = nearest the first checkpoint, `race::order_grid`)
and `x z` lines, its checkpoints, **each a gate** (several routes per event; needn't
start at the marker; `events::read_routes` reads an export without a race line).
`events::resolve_route` (the server at startup, `Terrain::load`; the race editor live)
finds the way along **every** road but streams (`routes.rs`: a graph of stretches
between junctions like `traffic::network`, Dijkstra between consecutive checkpoints,
unpaved ×3 and under 3.5 m ×1.5 per metre), refusing a checkpoint over 30 m from a
road, a route turning back at one (way out against way in, cosine < −0.7 over 15 m),
shorter than 500 m, or without a grid; the line (simplified to 0.3 m, the checkpoints'
points kept), widths and `marks` (each checkpoint's index in the line) go out with
`ToClient::Events` (protocol 16). The repo's Baunach routes were removed (2026-11-24;
"Races Baunach" with them, as a race needs a route): the user makes new ones. The old
"Baunacher Ring" points are the `RING` test route in `events/mod.rs`. Agreed with the
user:
- On the marker a window asks: time trial (`Action::Trial`, a private round of one,
  `Game::Trial`, never merged or joined) or race (`Action::Enter`: the lobby there, the
  first in hosts and sets things; at most the route's grid spots; starts with 2+, not
  with more racers than the picked route's spots).
- Settings (`race::Settings`): route, laps (loops, 1–20, default 3; no reverse since
  the grid is by hand: a reverse race is a route of its own),
  traffic (off), cars collide (on; off: `net::GhostPlayers`, the client treats remote
  cars with a name tag (in a realm, players' cars) as ghosts, traffic still collides),
  racers can reset (on; R: `ResetRule::Spot`, the rescue helicopter to the last gate
  passed, right half of the road; time trials never reset, "Start again"
  (`Action::Restart`) instead). Player cars only, for now.
- Running (`race.rs`: `Course` = a route with laps): each racer on their grid spot by
  slot (R before any gate goes back there), held (`Car::hold`) for `COUNTDOWN` (5 s),
  the parking brake off, in neutral (put in gear during the countdown; agreed with the
  user, 2026-11-22); the clock runs from the green light. Gates
  (`race::checkpoint_gates`): at each checkpoint's mark, facing the line's way in and
  out over 10 m averaged; a sprint's are all checkpoints (the last the finish), a loop's
  lap is the second on, then the first (passing the first from the grid is the start,
  not a gate; lap 1 includes the way there); the server judges them from the cars'
  states (`Gate::crossed`: forwards, within road half-width + 6 m, not a jump over 60 m),
  in order. "Wrong way!" only within 30 m of the line (not on the way from the grid). Standing (`Racer`:
  gates, last gate's time, laps, finished, left) goes out with the round on every gate.
  First one through: the rest have `FINISH_GRACE` (60 s); over when all are through or
  gone; results (`race::Results`) in the lobby. Leaving mid-race lists you as "Left".
- Best times: per (event, route, traffic), time trials only, a loop's laps or
  a sprint's whole way, top 10, one per name, for the session (`Rounds::best`, sent as
  `Round::best`).
- Client (`racing.rs`): the chooser, the race lobby window, the bar (countdown, lap,
  place, clock, lap times, gap to the one ahead from `EventState::gate_times`, a painted
  arrow and distance to the next gate (the font has no arrow glyphs), "Wrong way!"),
  gates on the road (two posts and a banner: next green, the two after white, the finish
  yellow), the route on the map (`map::MapLines`) and the next gate as a mark.
Tests: `a_race_runs_from_the_grid_to_the_results`, `a_time_trial_puts_its_lap_on_the_best_times`,
`the_races_routes_are_found_along_the_roads`, `race::tests`.
Not done: race-specific cars ("everyone the same car"), saving best times, rolling
starts, a minimap.

**Race editor** (asked for by the user, 2026-11-24, instead of recording a route while
driving; `src/race_editor/`, `src/bin/races.rs`, `cargo run --bin races [FILE]`): the
map editor's fly camera (`editor::camera`, 3× as fast: `SpeedScale`; Shift+wheel no longer sets its speed) and
ground hit (`editor::overlay::ground_hit`), no cars. `mod.rs`: `Draft` (the `Plan`:
looped, grid in placement order, checkpoints; undo/redo of whole plans, `touch` while
dragging/turning and `commit` at the end, `change_merged` for wheel notches; the route
found again at most every 0.1 s by `resolve_route` on a `RoadGraph` built at startup;
`text` = the export: comment, commented `race` line at pole, `Route::text`). `edit.rs`
(G grid tool: click places a box of the biggest car's body box, `show::CarBox`, turning
to face the pointer until the next click, Esc calls it off; click on a spot's disc or
box turns it again; Shift+wheel 1°; C checkpoint tool: click appends, Ctrl+click
inserts into the nearest leg of the found line (straight legs while none), a click on
the first checkpoint's disc loops; discs drag (4 px makes a drag); Delete removes what's
under the pointer; Ctrl+Z/Y). `show.rs` (boxes red where they overlap, arrows, discs,
gates via `racing::spawn_gate` (start green, finish yellow), the line as gizmos, red
straight lines while not found, egui labels, map lines and marks). `panel.rs` (window;
Open and Export dialogs asking a file name in the working directory, `.cfg` added,
asking before overwriting). Shots: `BIN=races` (unstrip hooks `EditorShotPlugin`; keys
`KeyG`/`KeyC` in `EDITKEYS`); a road at a village: `EDITCAM="-2965,-1972,28,-153,-62"`
runs up the screen through x 600–660 (`CLICK="600,520,30;600,380,40"` places a spot
facing up it). Nothing of it tried with a real mouse yet.

## Helicopters (asked for by the user, 2026-11-22)

`src/heli/`: `mod.rs` (`HeliPlugin` in `CorePlugin`: the model made of primitives, red
with a white stripe, rotors turning with a blur disc, flashing beacons; `Heli { cable,
load }` + `Transform` is one on screen; `Flown` ones (another player's rescue, the hunt's)
are dead reckoned from their latest `HeliPose`; never fogged, and drawn at 900 m shrunk to
look as big when farther, beacons at least ~0.004 rad across, so the hunt's is seen from
anywhere; `Chase`, the hunt's flight) and `rescue.rs`.

- **Rescue** (R with `ResetRule::Road`, also a hunter's in a hunt): flies in from 140 m
  ahead of the car (where the chase camera looks), holds over its roof on a 12 m cable
  (longer to stay 4 m over trees and houses within 8 m: their `Upright::top`), lets the
  cable down, lifts the car (upright), carries it at ~10 m/s average over a lift clearing
  land, trees and houses along the way, sets it down at the drive spot (a random way along the road, see "Beginner help"), unhooks
  (`Car::place`) and flies off 8 s. ~10 s near a road
  (`the_helicopter_takes_the_car_to_the_road`). Neutral and the parking brake on from the
  call, also once set down. Lets go if the car is moved by something else (> 2 m) or the
  driver's away.
  **The server's alone** (agreed with the user, 2026-11-24: a lean client; a later quick
  transport or reset to the garage needs no client change): R is only
  `DriverInput::reset` (a press in an input that came too late for its step is pressed in
  the next, `Shadow::late`: at a few frames a second R got lost); `Shadow` holds the
  `Rescue`, calls it by its `ResetRule`
  (`Round::reset_rule`, `Shadow::rule`; `Rescue::press`), drives the car as nobody's at
  the wheel while it has it, runs `rescue::fly` after the collisions
  (`Shadow::finish_step`) and refuses placements meanwhile. While it has the car
  (`Shadow::taken`), `Yours::notice` carries what to tell the player ("⚠ You are being
  rescued ⚠\nPlease do not resist!"), and the server sends the player their own car like
  anyone's (`Cars`, no tag; our number comes in `Welcome::car`). The client
  (`car::Spectating` holds the notice) puts `car::ServerDriven` on the active car: not
  driven, simulated, predicted, checked or force-fed, hidden, its engine hushed, its body
  and `Transform` following the remote car of our number (`client::spectate`), which is
  drawn and heard (helicopter from `CarState::heli`) like any other; the HUD shows the
  notice (`hud::server_notice`, first line large; only the border's colour pulses, not its
  width, which resized the frame and wrapped the last letter, and its lines never wrap). **Scripted cars** (protocol 29,
  asked for by the user, 2026-11-26): while the server has a car (`Shadow::taken`) its
  states carry `CarState::scripted` ("nothing a game does changes where it goes"), and
  every game shows it `SCRIPTED_DELAY` (0.1 s) before what's heard, on a cubic between
  the states around that time (`client::interpolate`; `KEPT` 12 states), carried on at
  most `SCRIPTED_AHEAD` past the latest, instead of dead reckoning it (which overshot as
  the helicopter sped up, slowed and set down); its rescue helicopter is drawn at the
  same time between its poses (`RemoteCar::heli_at`, `fly_flown`), so the cable stays on
  the roof. Generic, for later transports; the hunt's helicopter (not a car) is still
  dead reckoned (`a_scripted_car_is_shown_on_its_way_between_its_states`). Set down, the server gives it back as
  a move (`Yours::moved`, notice none), the client takes it up and predicts again
  (`the_server_rescues_a_players_car_while_their_game_looks_on`). The menu's reset row is
  enabled whenever we're driving: whether R does anything is the server's call. No sound
  yet.
- **Hunt helicopter**: the server flies a `Chase` per running round in `run_rounds` over
  the hunted (50 m up, at least 30 m over the ground; no top speed, acceleration ≤ 7 m/s²,
  so it lags, overshoots a hard stop and turns back; wanders ±6 m over a slow target) and
  sends `ToClient::Heli` 15/s to everyone in the round, wherever they are
  (`Connection::hunt_heli`; drawn only in a running round).
- Shots: the rescue runs on the game's own server, on real time (not `STEP`), so tap R
  at a frame and shoot soon after: `TAPS="KeyR@30" HELICAM="-40,12,-30" bash
  tools/dev/shot.sh out.png 44` (the car carried over the houses; the server has it from
  frame ~30 to ~54 at lavapipe's pace). `TOPPLE="side,frame"` (the car onto its roof off
  the road) is a placement the server refuses now, and `OFFLINE=1` has no rescue.

## Server authority (agreed with the user, 2026-11-24; branch `server-physics`)

The server is to be in charge of every car's physics (later the shops too, and in the end
the client knows nothing of cars but generic code, the server sending models, parameters,
purchases). Plan agreed (Rocket League's way, not sim racing's trust-the-client):
- Clients send only their inputs, numbered by simulation step; the server simulates every
  car once, forwards, never rewinding. Each client predicts its own car ahead and, when
  the server's state for a step disagrees beyond a threshold, takes it and replays its
  stored inputs since (physics snaps, the drawn car and the force feedback smooth).
  Normal driving must never feel delayed; in collisions drift and snaps are fine.
- Only our own car is fully predicted; other cars stay dead reckoned. Small differences
  (native vs wasm, glibc's libm by CPU) are tolerated, not eliminated; the `libm` crate
  is the fallback if normal driving diverges.
- A physics change needs client and server updated together (ends "client-only changes
  need no server update" for anything driving). Shop changes are trusted until the shops
  move to the server. Cosmetics (FFB, sound, camera, later smoke, marks) stay local.
- Phases: 0 groundwork; 1 shadow mode (server drives a copy, logs divergence and cost:
  try on the VPS); 2 authority (server states to others, owner reconciles, collisions on
  the server, hunt tags from server contacts); 3 discrete gameplay on the server (reset
  and rescue, holds, ghosts, lending cars, parking, fuel); 4 car changes as requests;
  5 shops on the server, generic client.

Built (phases 0, 1, 2 and the start of 3):
- `car::DriverInput`: a step's input (axes, buttons pressed since the last step,
  `away`/`held`), ~10 bytes on the wire; `DriverInput::take` passes it through its
  encoding so the client drives with exactly what the server gets. `Car::drive` applies
  it (all the car's rules: shifting, ignition, parking brake, key steering) in
  `FixedUpdate` before `simulate`; `StepInput { step, input }` numbers the steps. Every
  step is exactly `sim::STEP` (the fixed clock's nanosecond `Duration` as f32 differs,
  which broke bit-equality at once).
- `snapshot.rs`: `Snapshot` (save, load into a value of the same setup), the `snapshot!`
  macro listing a struct's state fields in its own module; a car is ~1.1 kB; a loaded car
  drives on to the bit (`a_loaded_car_drives_on_as_the_saved_one`). Any new state field in
  the physics must go into its module's `snapshot!` list, or replays and the server drift.
- `spec.rs`: `CarSpec` (CarBuild with alignment and brakes, Parts, Ecu), checked on read
  (ranges, the engine fitting, the tune's engine).
- Protocol 19: `ToServer::Car(CarSpec)`, `Inputs { last, inputs }` (the steps since the
  last message plus 12 again, at most 64), `Check { step, state }` (snapshot after every
  `client::CHECK_STEPS`th step, taken in `FixedLast`); `ToClient::Pace { early }`.
- One server timeline: `shadow::Tick` counts the server's steps (`step_shadows` in
  `FixedUpdate`, 120 Hz set by `ServerPlugin`); a player's step `tick + offset` is driven
  at `tick`, the offset set when inputs first come (or come over 30 late / 120 early) so
  they come `MARGIN` (4) steps early. The server tells the least margin 4× a second
  (`Pace`); the client's `pace` stretches its fixed timestep by up to 3 % to keep it at 2
  (each step still simulates `sim::STEP`). A missing input is guessed (the one before
  without presses; nobody at the wheel after 30 silent steps).
- `net/shadow.rs` (`Peer::shadow`): the server builds the car (baking maps on a thread,
  cached in `BakedMaps`), drives it from the inputs on its clock, compares at each check
  (a check ahead of the copy waits for it), logs every 10 s (`… car on the
  server: N of M checks to the bit, …, µs each, … without their input`), then takes the
  player's state and re-drives the steps since. In the game against the dedicated server
  here: every check to the bit, ~178 µs a step (the VPS ~4–5× slower: ~10 % of its vCPU
  per player). `step_cost` (ignored) prints one car's cost on the map (147 µs a step; it
  was 715 before the relief fix, which sped up the clients too).
- Authority (protocol 20): the server's car is the truth. `ToClient::Yours { step,
  placed, state }` every 12th step; `net/predict.rs` keeps each step's input and state
  (240), takes up a differing state and re-drives the steps since (`reconcile`, Update
  after `receive`), not while `away`, and not states from before our latest placement.
  Anything changing our car outside the simulation (between steps, or after the
  simulation within one: the rescue helicopter) is noticed by comparing snapshots and sent
  as `ToServer::Place { step, state }`, which the server takes up (trusted for now; phase
  3 removes that). Checks are only logged. Others see the server's car (`Shadow::car`,
  only while the player's inputs come; else the player's own state). The driver's aids
  are part of the input; `knocks` isn't in the snapshot (the crash sounds clear it).
  Server margin: moved once a second by the worst lateness (a game at 4 fps sends 30
  steps at once); the client's frames may catch up 1 s of game time (Bevy's 0.25 s let
  a slow game fall behind the server's clock for good).
- `car::Correction`: after a correction the active car is drawn where it was and glides
  to where it is (0.1 s; past 4 m at once); the camera follows the drawn car (its
  `Transform`), not `car.body`.
- **Remote cars on our car's timeline** (2026-11-26, protocol 28; the user found
  rear-ending traffic weird: a knock here, traffic driving on): our car runs a round trip
  plus the margin ahead of what the server's states show, so dead reckoning to
  `their(now)` met cars ~v × (RTT + 30 ms) too near (1.95 m at 7.7 m/s, 100 ms each
  way). `Yours::time` is when the server's step that drove `Yours::step` ended (its
  clock); `Connection::step_time` maps any step of ours to the server's clock, and
  `meet_time` carries every remote car to the time of our step (not scripted cars
  (see "Helicopters"), nor our own spectated car; not while our steps don't run, or more than
  `AHEAD` off what's heard). Bodies are placed per step (`place_bodies`, `FixedUpdate`
  before `push_off_remote_cars`) and drawn as of the last step (`move_cars`). A push our
  car gives a remote car is a `Kick` (velocity change from `collision::resolve` on the
  clone, at the step's server time) carried on until a state from then on comes. Replays
  in `reconcile` meet the remote cars as of each step (`others_at`, `meet_again`; knocks
  restored, the crashes were heard). Tests over a link 6 frames each way (`Slow`):
  `other_cars_are_met_where_the_server_has_them_as_it_drives_our_step` (2.7 cm),
  `a_car_rear_ended_is_met_in_the_same_step_here_and_on_the_server` (1 step apart; 25
  before). Sustained pushing between players still corrects every ~0.1 s by up to ~0.2
  m (the other car's reaction can't be known; 1.4 m before kicks); replay contacts made
  no measurable difference there. Not tried with real traffic in the game yet.
- Collisions on the server (`server::step_shadows`, after the cars' steps): players'
  cars push each other (same realm, neither a ghost by the client's word,
  `Rounds::players_collide`), push themselves off traffic, and traffic reacts to the
  server's car (`Visitor` body). The client still pushes itself off remote cars, as its
  prediction (`players_cars_meet_on_the_server`).
- Phase 3: the rescue helicopter is the server's alone (protocol 24, see "Helicopters"):
  the client only looks on while the server has the car. **The server moves cars** (protocol 22): `Shadow::put` on joining
  (`online::join_spot`, needs the server's `Network`), at a round's start
  (`events::start_spot`: by the marker or on the grid; again when a time trial restarts)
  and when it's over for a player (`events::marker_spot`, the lobby's slot, else the
  round's), tracked in `Rounds::started`; `Yours::moved` counts the moves and the client
  takes such a state up whatever it had (even away, `reconcile`; a lent car not yet
  spawned retries with the next state). The client still moves its car alike (the same
  shared functions). A placement may not move the car: more than `REACH` (2 m, plus
  1.5 × speed × its age) from the server's car is refused (acknowledged, so the client
  takes up the server's state); a car built anew (`set_spec`) goes where the old one
  was or where the server put it (`Shadow::anchor`)
  (`a_players_game_cant_move_their_car_but_the_server_can`). **Holds** (protocol 23):
  `Car::hold` (s left, in the snapshot, counted down in `Car::drive`, kept by `place` and
  `rebuilt`; a placement can't shorten it) replaces `DriverInput::held`; set where the car
  is put at a round's start (`Spot::hold` = `Round::hold`), so the release is predicted
  exactly. The `Held` resource is only the round clock's view (rumble, tests). **Ghosts**
  by the server's word (`server::haunt`, `Peer::ghost`): rescued, away, stopped on a
  shop's or event's marker until 9 m off it (`Peer::at_marker`), in a lobby, and a ghost
  stays one while it overlaps a car; that's what collisions, `Visitor` and the state sent
  to others use (the client's `CarState::ghost` only while the server has no car)
  (`a_car_stopped_at_a_shop_is_a_ghost_by_the_servers_word`). The client still decides
  its own `Ghost` for its prediction (`haunt`). **Fuel**: a placement with more fuel
  than the server's car (beyond `FUEL_SLACK` + `FUEL_PER_SECOND` × its age) is refused
  unless the car is stopped at a gas station's marker (`Shadow::refuel`, from `haunt`)
  (`a_car_is_only_filled_up_at_a_gas_station`). **Lent cars**: the server lends them
  itself (`Shadow::lend`, `Round::lent_chassis`, a factory build with stock maps), keeps
  the player's own car (`Shadow::own`) and gives it back as it was (`give_back`) when the
  round's over for them, telling the player each (`ToClient::YourCar`, which the game
  builds); meanwhile `set_spec` is ignored. **Car changes** are requests since
  milestone 6 (see "Owning cars and parts"): the server checks them and sets new tires',
  engines' and turbos' condition itself. Still the client's word (trusted): the condition
  otherwise (wear, fuel, by placements), the parking brake pulled at markers (harmless:
  it only stops our own car).
- Not done: hunt tags from the server's contacts (clients still send `Touched`), the rest
  of phase 3 (placements are trusted: a modified client can teleport), checking a
  client's inputs (rates, ranges are clamped by the encoding).

## Server catalogue (agreed with the user, 2026-11-24; built the night after)

The game knows no cars or parts by itself: the server defines which exist and tells
players on joining, like shops and events. Later players buy cars and parts, which the
server keeps for good (accounts, money, saved cars by string ids: not built); for now
the server gives the start car and the showroom and garage give cars and parts away.
Decisions (the user's answers): car changes become requests to the server as part of
this work; the server sends engine maps (the client doesn't bake factory engines); the
catalogue stays Rust data in the server for now (a `--catalogue FILE` later).

As built:
- `src/catalogue.rs`: `Catalogue` (chassis, gearboxes, clutches, suspensions, paints
  with a `shop` flag, compounds (`TireCompound`: name, about, the `tire::Compound`
  rubber), engine families (`Version`: code, name, `block: EngineHardware`, `Preset`),
  part slots, `start`), compared by a fingerprint; `catalogue::Current` resource (the
  server's; on the client inserted when `ToClient::Catalogue` comes). `builtin()` (from
  `catalogue/{chassis,engines,parts}.rs`, the old statics) is the server's default; it
  and the baked maps (`engine/store.rs`) aren't compiled for wasm outside tests, which
  proves the web client needs neither. The convenience `CarSetup::factory`,
  `CarBuild::factory`, `Parts::factory`/`stock`, `CarSetup::default` are builtin's
  (tests, server, dev tools); game code uses the `_in` versions with a catalogue.
- `CarBuild` and `Parts` carry their catalogue (`Shared` = `Arc<Catalogue>`) and refer to
  entries by place, so the simulation, looks and shops never read a global. Engine
  parts are data: `Part { name, effects }`, `Effect` = change a hardware `Field` (times,
  over, plus, to, at least, rounded to a step), remove the cat/silencer, regrind cams,
  switching cams, a `TurboKit`; a slot has a `Readout` (its label's detail) and
  `needs_turbo`. Every factory car and part came out bit for bit as before (dumped and
  compared). Traffic's chassis weights are `Chassis::traffic`.
- Wire (`catalogue/wire.rs`): the whole catalogue, checked to hold together on reading
  (18 kB built-in). `Reader::with(bytes, catalogue)`/`decode_with`: cars (`CarBuild`,
  `Parts`, `Looks`, `CarSpec`) only read with a catalogue.
- Protocol 26: after `Welcome` the server sends `Catalogue` and `YourCar(spec)` (the start
  car, built in its shadow); lent cars and the own car given back come as `YourCar`
  too. The client (`client::take_your_car`) builds what it's told (in place of the
  active car, or the first one by the spawn); there's no car before joining (the camera
  looks down the spawn's road under the start menu); shops open once there's a car
  (`garage::open`). `ToServer::Buy { chassis, engine }`: the showroom's "Drive it" asks,
  the server builds it and answers `YourCar`. `ToServer::Maps { fingerprint }` →
  `ToClient::Maps { fingerprint, text }` (zlib, `wire::Blob`, none if the server hasn't
  got them: only the stock ones for now): `take_your_car` waits for them when they're not
  in the maps cache (`workshop::keep_text`), and bakes only if the server has none.
  `CarSetup::to_look_at` (blank maps and ECU, `EngineMaps::blank`, `Ecu::blank`) is for
  cars only looked at: the showroom's and traffic's looks.
- Shops' changes are requests (`fitting::Work`, see "Owning cars and parts", milestone 6).
- Not done: the dyno's cam changes bake without asking the server for maps; the server
  only gives the stock engines' maps (not ones it baked); the race editor uses the
  built-in catalogue (biggest car box); a catalogue file.

## Owning cars and parts (planned with the user, 2026-11-26; milestones 1–9 built)

The whole plan, with milestones: `docs/cars-and-parts.md` (keep it up to date).

**Built so far** (2026-11-26, milestone 7 on 2026-11-27): milestone 1, parts as data (`src/items.rs`, the hatch's
definitions in `catalogue/items.rs`). `PartDef` (string `id`, `Kind` of slot it goes in,
`Does` what the game's code does with it, `fits` chassis names, size box, mass, price
in cents, painted, `Knob`s, `SlotDef`s), `Item` (definition by place in
`Catalogue::defs`, paint, `variant` (rim and tire sizes index `Rim::all`/
`TireSize::all`, anti-roll bars the chassis' sizes), knob values, slots), `Trim` (a
stock car's tree; one per chassis × factory engine). Engine slots sit in the long
block (`LONG_BLOCK_SLOTS`), on the engine (`ENGINE_SLOTS`) or in the car (`CAR_SLOTS`);
cam gear timing and boost are the engine item's knobs, brake force and balance the
chassis', camber and toe each damper's, pressure each tire's. **The bridge**: the
simulation and the shops still use `CarBuild` + `Parts`; `Item::resolve` works them
out of a tree (the left corner of an axle counts), `items::assemble` puts a changed
`CarBuild`/`Parts` back into a tree, keeping what they don't say (a new paint goes on
every painted part). `CarSetup::car` is the tree; shops rebuild with
`CarSetup::changed`. `CarSpec` on the wire is the tree and the tune (protocol 31);
reading checks every item against the catalogue (`Item::fine`: in it, in a slot of
its kind, required slots filled, knobs within range).
Milestone 2 (one car): the Vireo is an early-90s three-door (4.07 m, 2.57 m wheelbase,
14" wheels, 820 kg sprung; a cabin point's glazing 2 is a painted side with the rear
window glazed: the C pillar), engines B16 and B18 (baked), trims `vireo.1-6` (steel
rims, 185/60 R14, solid front discs, rear drums, open differential) and `vireo.1-8`
(15" alloys, 195/55 R15, vented discs, rear discs, limited slip, roof spoiler: an item,
not drawn until the body is in pieces) from `catalogue/items.rs`'s `trim_specs`. A
factory car is its trim's (`CarSetup::factory_in`, `to_look_at`, via `items::trim_of`;
so the server's start car, the showroom's "Drive it" and lent cars are trims);
`assemble` keeps a part that does the same as asked (steel rims stay steel). Shots:
`LINEUP` counts stock cars (each chassis × factory engine); its straight is in a
forest now, `OFFLINE=1 CARCAM="-6,1.3,0"` shows the car side on in the village.

Milestone 3, the body in pieces (2026-11-26): `body.rs` builds the body from a `Dress`
(the shell's colour and each fitted body part's, `items::dress` from the tree: the first
of a pair of slots is the left one; `Side::Middle` for the single ones). Regions of the
outlines are pieces: `Body::bumpers` (lengths in from the ends) below `bumper_top` (a
crease point in every lower-body section), `Body::door` (x of the doors' edges; the
door glass from the windscreen's base to the rear edge, a station added there), the
bonnet (deck top up to the windscreen), the hatch (deck top behind the cabin, the tail
above the bumper, the rear window), fenders (the sides ahead of the doors); the rest is
the shell. Details belong to the piece they're on (`piece_at`). Lip, skirts, spoiler,
diffuser and mirrors are shapes of their own (`add_ons`), drawn only with what they're
fitted to. The shell is hollow: the lower body has inner faces (`Builder::skinned`; the
bay's are 0.7 of the paint, the cabin's trim, the floor carpet), the deck is open under
the greenhouse (whose floor is gone; sills along the cabin, a bulkhead at the
windscreen's base), the greenhouse's painted panels have inner faces (headliner, trim),
its glass none (the driver sees out). Loft ends are strips across (`Face::Cap`).
`mechanicals.rs` draws what's under the body from the tree as a second mesh (cached by
`mechanicals::key`): the engine from its long block's family (transverse: exhaust
towards the bulkhead, gearbox on the left), intake runners by manifold, throttle by
size, filter (box, sport panel, open cone), headers by length, a turbo and intercooler,
the cam cover red on a reworked head, radiator, battery, seats per seat slot, a
dashboard and carpet, the tank and the exhaust line (catalyst, silencer, tip by the
fitted parts). `Looks::car` is the tree (protocol 32); `visuals` caches bodies by
`Dress::key`. Shots: `UNFIT="Bonnet,Door"`, `FIT="FrontLip,SideSkirt,Diffuser"`,
`FITPAINT=0` dress the lined-up or `OFFLINE` car (`TRIM=1`: OFFLINE's is the 1.8);
never name a shot variable `STRIP` (the dev shell exports `STRIP`, the strip tool).

Milestone 4, accounts and saving (2026-11-26): `db.rs` (native only; sqlx `Any` over
SQLite (bundled), MySQL/MariaDB, PostgreSQL by the URL's scheme, a plain path is an SQLite
file made if missing; migrations per dialect in `MIGRATIONS`, `schema_version`; `?`
placeholders numbered for PostgreSQL by `Store::sql`): tables `accounts` (name, `folded`
lowercase unique, argon2 PHC hash; empty hash = no password, the default user's),
`active` (the instance lock: account, instance, refreshed ms; taken if ours, missing,
older than `STALE` 30 s, or by force alone; refreshed every `REFRESH` 10 s; deleted on
leaving), `cars` (owner, text; one per account for now). `Database` runs a `Store` on a
thread of its own with a current-thread tokio runtime, jobs in order (logins' answers
picked up by `answers`, `flush` waits). `net/accounts.rs`: `Accounts` (the server's,
`Server::with_accounts`; a stub type on wasm), `Admitted`. The server: `Hello` has a
`password` (protocol 33; read as empty when missing, so an old game is told the version),
a player with accounts waits (`Peer::logging_in`) until the database answers, then
`admit` (the old join code): the saved car (`saved::read`; unreadable: the start car,
logged) with its `Condition` given to the shadow's car when it first has a state (a move,
`Yours::moved`, so the game takes it up), else the start car. An account plays once per
server, a name once (case-insensitive). Saved (`saved::write` of `Shadow::kept`: the own
car while one is lent) every `SAVE_EVERY` 60 s (`keep`), when the player leaves (then
released), and by `Server::save_everyone` when the game's own server stops (`LocalLink`'s
drop waits up to 4 s). `saved.rs`: `part` lines (tab-separated depth, slot name, def id,
paint name, variant (rims/tires by label), `knob=value`s), a definition gone drops its
item, then `fuel`, `tread`, `engine`, `cylinder` lines, then `tune` and the whole ECU
(`Ecu::to_text_whole`), read into `Ecu::blank`, checked by a wire round trip. The server
binary: `--database URL`, `--database-file F`, `--guests on|off` (off; needs a database),
`server --database URL user add|password|remove NAME | list` (password from the terminal
via rpassword, else a stdin line). Playing alone: `Accounts::alone`, SQLite at
`storage::data_dir()/alone.db` (`~/.local/share/car-sim`), the default user `default`, by
the name the game gives; tests use `sqlite::memory:`. The start menu has
`kit::password_field` (never saved). NixOS module: `database` (default
`sqlite:///var/lib/car-sim/car-sim.db`, `StateDirectory`), `databaseFile` (credential),
`guests` (false), and `car-sim-user` (runuser as car-sim, or root with a URL file).
Tests: `db::tests` (all dialects through `on_another_database`, ignored:
`CAR_SIM_TEST_DB=postgres://…|mysql://…`; tried against PostgreSQL 18 and MariaDB 11.4
started by hand from nixpkgs in the scratchpad), `saved::tests`,
`a_players_car_is_kept_between_sessions`. Shots get a fresh `XDG_DATA_HOME` each
(`DATA=dir` keeps one, e.g. to see a car come back); `JOIN` with `PASSWORD`.

Milestone 5, garage park, several cars, trunks (2026-11-26): **holds** (`cargo.rs`):
`Chassis::hold`, 10 cm cells in areas with a floor and ceiling each (the hatch's trunk
0.7 × 1.2 × 0.4 m, the rear bench's and passenger seat's places only while their slot is
empty); `Grid::pack` puts the largest first, turned every way, where its top ends lowest
(a heightmap: things rest on things); packed afresh whenever the cargo changes, so it's the
same everywhere and nothing about places is sent. `CarSpec::cargo`/`CarSetup::cargo` (a
car's loose parts; `set_cargo` keeps what fits) and `cargo_load` (each part's mass where it
lies, loaded on the body in `Car::load_fuel`); saved cars have a `cargo` section (format
2). **Owned** (`owned.rs`, server `net/stash.rs`): `Owned` per `Peer` (garage cars as
`Parked` spec + `Condition`, garage parts, parts held per shop name; limits `GARAGE_CARS`
50, `GARAGE_PARTS` 2000, `SHOP_HOLD` 20), told as `ToClient::Owned { owned: OwnedView,
why }` (`Connection::owned`, `refused`); `ToServer::Stash` (`Unload`, `Load`,
`ShipHome`, `Ship`, `Drive`) by where the car stopped (`Peer::at_shop`, the marker's
shop); `Buy` parks the old car in the garage; what a shop takes off goes into the trunk
(since milestone 6 by the fitting rules). The database: migration 2 (`cars.place`
`driven`/`garage`, an `items` table, place `garage` or `shop NAME`); `Belongings`; the car
driven saved alone unless `Owned::revision` moved (then `save_belongings`, all in one
transaction). Every `YourCar` the game builds is placed (`Prediction::forget_end`): the
server's copy of a car built anew waits for that. **Garage park**: shop kind `garage`
(`Garagenpark Gerach`, (−2680, −1793)); `garage/park.rs`: its window (into the garage,
your cars) and the hall 250 m under the marker (a bay per car, at least 3, the car driven in
the first; parts on shelves at the back, `MOST_SHOWN` 120; rebuilt when what's in it
changes; the camera looks at the car whose row has the focus); `trunk.rs`: "Trunk and
parts" in every shop window (`AtShop::stash`, a stack of `trunk::Screen`s drawn in the
shop window's place; back pops it). **Special rescue**: `ToServer::Rescue { to }` (a gas
station or car workshop), `Shadow::special_rescue` only for `Car::stranded`; the central
menu's in-game actions list the nearest of each; a carry takes at most `LONGEST_CARRY`
(30 s). Shots: `CARAT=frame` buys the `CAR` then (so the first car goes into the garage);
the garage park `PLACE="-2680,-1793,150"`, `TAPS="Enter@200"` into the hall. Protocol 35.
Test: `a_player_keeps_cars_and_parts_in_their_garage`.

Milestone 6, fitting at shops (2026-11-26; the user's answers: the parts shops split into
car workshops and tuning shops, tire shops new; every shop's assortment listed in the
shops file, nothing by default; no long-block work until the engine shop, milestone 7).
**Shops file**: kinds `car`, `tuning`, `tire` (`parts` gone; `ShopKind::fits_parts`),
`sells` lines under a shop (ids, `*` at the end a prefix; `Shop::sells`, sent with the
shops, protocol 36; `what_the_shops_sell_is_in_the_catalogue`); tire shops in Gerach
(−2502, −1763), Baunach (174, 2587), Rattelsdorf (2903, −844). **Catalogue**:
`assemblies` (`items::Assembly`: the factory engines as the trims have them, `engine.b16`,
`engine.b18`), `PartDef::absent` (the "None" turbo, intercooler, catalyst: stands for
nothing, put in when taken off, never kept), `SlotDef::needs` (lip → front bumper, mirrors
→ doors, spoiler → hatch, diffuser → rear bumper; `Item::fine` checks it); loose parts may
lack what their slots require (a gearbox without its final drive). **`fitting.rs`**
(shared, the server's rules, the game's previews): `Work` (`Fit { slots, supply }` with
`Supply` new `Part { def, variant, paint }` / `Assembly` / `Trunk` / `Held` / `Wheel(rim,
tire)`; `Remove`; `Knob`; `Paint`; `Tune`; `MakeWheel`; `Split`), `Path` = slot places
from the car. `Bench` does works in order on a copy, all or none; `finish` checks the car
(needs, `fine`, `resolve`, every wheel against the room, a wire round trip of the spec),
sets the tune's ECU model to the fitted ECU's, puts what came off into the trunk, else
held at the shop, else thrown away (said), and reports `Fresh` (new tires per corner,
engine, turbo), for which the server sets the condition (`stash::renew`). Who does what:
`fits_into` (car/tuning: all but rims, tires, long blocks and their insides; tire: wheels,
rims, tires), `knobs_at` (alignment: chassis, dampers, tires; dyno: the engine; car/tuning:
the final drive), paint at paint shops (shop paints only), tunes at dynos; a new part is
one the shop sells, made for the chassis, a size it comes in, painted in a shop paint or
the car's own; what's fitted to the old part moves into the new one's empty slots.
**Protocol**: `ToServer::Work { id, works }` → `YourCar`, `Owned { why }`, `Done { id }`
(protocol 37); the game builds the server's car once all its requests are answered
(`Connection::work`, `waiting`, `server_spec`); settings changed in the game (alignment,
paint, the dyno's cams and tune) go as works (`fitting::changes`, `only_settings`); any
other local difference falls back to the server's car. `ToServer::Car` is gone (protocol
39); `Buy { trim, trade_in }` (protocol 38) only at a car dealer selling the trim (the
shots' `CAR_SIM_TRUST_PLACEMENTS` server anywhere), trading in drops the old car, trunk and
all. A car whose engine's maps the server hasn't got bakes them here without blocking
(`Connection::baking`). **Game**: `garage/fit.rs` (in the workshop scene): the car's
slots the shop works on by group, a slot's screen (fitted, what's on it the shop works on,
take it off, new from the shop, brought), a ware's screen (size: rims and tires that go
with what's on the wheel and in the well; paint), fit here / both on the axle / all four;
the final drive's ratio at car and tuning shops; tire shops: a new wheel (rim kind,
diameter, width, tire kind and size), loose wheels taken apart or made of a rim brought.
The parts shop's window (engine builder, repairs, new tires) is gone: repairs are new
parts. Paint shop: "What" picks the whole car or one fitted body part. Showroom: trims
(those not sold there only to look at), "Trade in the car I drive". Tests:
`fitting::tests`, `the_server_takes_a_car_changed_in_a_shop` (refused away from a paint
shop), the garage test (a clutch at a tuning shop, a trade-in). Not done: prices and money
(all free), the rack and engine stand showing what's picked, removing "None" parts from the
catalogue for empty slots, intercooler needing a bumper opening.

Milestone 7, the engine shop (2026-11-27, built while the user slept; my decisions are
listed in the plan's "Decided while building milestone 7"). **Items carry their wear**:
`PartDef::wear` (`items::Wear`: pistons, rods, valves (in the cylinder head), head gasket,
bearings (the crankshaft), turbo, tread (tires)), `Item::wear` (a value per cylinder or
rib, trailing zeros left out, at most `MOST_WEAR` 8; on the wire, protocol 40).
`Condition::put_on` puts a car's live `Damage` and treads on its parts, `of_parts` reads
them back; the server does that around every shop's work (`stash::try_work`), so a new
part is new, one taken off keeps its wear, one brought brings it (`Fresh`/`renew` gone),
and parked cars' trees carry it. Saved cars: format 3, a `wear=a,b,…` field after the
knobs (older files' `tread`/`engine`/`cylinder` lines go onto the parts). The "Internals"
slot (forged rods/pistons) carries no wear. **Bores**: `items::OVERSIZES` (0, +0.25,
+0.5, +0.75, +1.0 mm), `PartDef::bores` (long blocks and the Pistons slot's parts: their
`variant` is the oversize step), `Parts::bore` (resolved from the block's variant; adds to
the hardware's bore, so the maps change). **Engine shop** (`ShopKind::Engine`, word
`engine`, `fits_parts`): `fits_into` a long block into an engine, anything inside one, and
a whole engine into the car (the kept one swapped in); car and tuning shops still never
touch long blocks. Paths may start in a loose part: `fitting::IN_TRUNK`/`HELD`, then its
place in the trunk or what's held (`root_of`, `item_in`, `slot_kinds_in`; a loose part may
lack required parts, `Remove` allows it; `shifted` moves a path's place when the part
fitted comes from the same list). `Work::Rebore { at, size }` (engine shop, a block, only
bigger); `finish` refuses pistons not the block's size anywhere (`bores_mismatch`), new
blocks come only with the stock bore, and an engine shop holds one engine and one long
block per player (`engine_shop_takes`, also for unloading and shipping there). The repo's:
"Motorenbau Mühlbach", Baunach (120, 2700), selling the internals and the B16/B18 long
blocks. **The server bakes** (`shadow.rs`): maps neither stock nor in `BakedMaps` bake on
a thread at the lowest priority (`setpriority(PRIO_PROCESS, 0, 19)`, Linux; the scoped
worker threads inherit it); a car already driving goes on as it was (`Shadow::pending`;
`spec()` is the pending one, what further changes build on, `driven()` what the game is to
build), then is built anew (its condition's fuel as now) and the player told
(`take_switched` → `YourCar`); the work's answer says the maps are being worked out.
`BakedMaps` keeps the packed text too, and `ToServer::Maps` is answered from it
(`store::pack`). **Game**: the workshop scene (`bay`) takes engine shops; `fit.rs`: the
car's Engine slot, its long block with a "Rebore" row and the internals, "Brought" (the
trunk's and held engines and long blocks, `Screen::Loose`), the rebore screen (oversize,
then "Rebore, new X pistons" or pistons brought of that size: one request of two works),
pistons only in the block's size; every workshop shows a part's wear ("worn 40 %", from
`fit::Bench::worn`, the tree with the car's condition now). Shots: the engine shop
`PLACE="120,2700,150"`, `TAPS="Enter@200"` onto the lift, then `ArrowDown@210;Enter@214`
the engine, `ArrowDown@222;Enter@226` the long block, `Enter@234` the rebore. Tests:
`fitting::tests` (`an_engine_shop_works_on_long_blocks_only`,
`a_rebored_block_takes_pistons_of_its_size`,
`an_engine_kept_at_the_engine_shop_is_worked_on_and_swapped_in`, the tire test's wear),
`saved::tests::the_wear_goes_onto_the_parts_it_is_in`,
`the_engine_shop_bakes_a_rebored_engine_while_the_old_one_runs`. Not done: bore wear (a
rebore repairs nothing by itself: the pistons carry the knock wear), selling the long
blocks' insides loose (they're only fitted), the engine stand showing the engine worked
on, a priority below normal for the bake on other systems than Linux, the dyno's own bakes
going to the server.

Milestone 8, new parts and physics (2026-11-27, built while the user slept; my decisions
are in the plan's "Decided while building milestone 8"). **The tree feeds the setup**:
`CarSetup::with_car(build, parts, engine, car)` works out what `CarBuild` doesn't say from
the tree (`new` assembles one; the callers that used to overwrite `setup.car` call it).
**Seats' masses** count where they sit (front row `FRONT_SEATS`, bench `REAR_BENCH` behind
the eyes); the chassis' mass is without them (770 kg at 1.2665, the car's balance as
before); a "Bucket seat" (6 kg). **Aero**: body parts `Does::Body(Aero)` (drag, downforce
front/rear at the design ride height, `ground`: share more per cm nearer the ground; a
`wing` scales by its angle knob, `Aero::at`, numbers at `WING_REFERENCE` 10°, no lift at
−4°); a missing piece adds `Panel::missing_drag` (mirrors take drag away);
`CarSetup::downforce(lowered)` per axle from the axles' mean travel (`GROUND_LEAST`..
`GROUND_MOST`), so ride height and pitch matter; lip, skirts, diffuser have ground effect,
the stock roof spoiler a little downforce, "Rear wing" (`vireo.wing`, knob "Wing angle"
0–16°, drawn on stands, `Dress::wing`). **Ride height**: the sport and track springs' knob
"Ride height" (mm, sport −40..0 from −25, track −70..+10 from −40; stock springs none),
`CarBuild::ride_height`/`wing` (resolve/assemble, the left spring per axle),
`AxleSetup::lowered` shortens the spring's rest length (`spring_rest_length`). Alignment
shops set springs' and body parts' knobs (`knobs_at`); the alignment window has the ride
height per axle and the wing's angle. **Brakes** (`physics/brakes.rs`): `Does::Brake(Brake)`
(torque per wheel at the pedal floored, smallest rim, heat capacity, cooling standing and
per m/s, fade temperature, the pads' life in MJ); the car's torque and balance from the
front and rear parts (the chassis' `brakes` share is gone; `hub` without the brake);
`BrakeState` per corner (temperature, pad wear, the pads' torque this step) in the
`Corner` snapshot, heated by torque × ω, `grip` fades over `fade` (to `FADED` 200 K
over) and falls with worn pads (past `WEARING` 0.8, `THROUGH` worn through); a "Big
front brake kit" needing 15" rims (`fitting::finish` checks each corner). **Clutch and
gearbox** (`drivetrain.rs`): `clutch_wear` (by the slipping work, `CLUTCH_LIFE` J per N·m of
the clutch; `clutch_grip` from 0.6 on, `clutch_capacity`), `Gearbox::torque`
(5-speed 200, 6-speed 360 N·m; more wears the gear in, `OVERLOAD_WEAR`), `gear_wear` per
forward gear (synchro work ½·`SYNCHRO_INERTIA`·Δω² against the input shaft's speed
`shaft`: the gear's in gear, the engine's in neutral with the clutch up, kept while
pressed; `synchro_reach` shrinks from 0.5 to `SYNC_RPM` worn out: refused gears grind
(`grind` 0.3 s with the automatic clutch) and stay out; double-clutching gets them in).
Wear kinds `Pads`, `Clutch`, `Gears` on the items; `Condition` has `pads`, `clutch`,
`gears`; `Car::place` keeps them, `rebuilt` for the same parts. **Superchargers**
(`physics/engine/supercharger.rs`): `EngineHardware::supercharger` (`Supercharger`:
Roots' displacement per rotor turn and leak area, or a centrifugal `Compressor`; ratio;
belt drag; a bypass `Diverter`), `Effect::Supercharger(SuperchargerKit)` (scaled like a
turbo kit); in the turbo's slot, renamed `catalogue::TURBO_SLOT` "Turbo or supercharger"
(ids stay `turbocharger.*`; saved parts whose slot name is gone go into a free slot of
their kind), parts `ROOTS`, `CENTRIFUGAL`. `Engine::tract_inflow` takes the rpm; the charge
follows the blower's outlet temperature (`CHARGE_LAG`); `Engine::torque` takes the drive
torque off (`blower_torque`). On the 1.6: Roots ~0.5–0.6 bar from 2000 rpm, centrifugal
0.05/0.12/0.42 bar at 2000/3000/6000; +20–27 % torque (knock-limited on the stock
compression). Maps unchanged (not in the fingerprint: no bake); `Ecu::calibrated` reaches
300 kPa with any forced induction and takes the supercharger's charge temperature
(`expected_charge`). **Needs/provides**: `PartDef::needs`/`provides` (tags), checked in
`Item::fine` for whole cars (`items::unmet_need`) and `fitting::needs_missing`: the Roots
blower needs `BONNET_CUT_OUT`, which "Bonnet with a blower cut-out" (`vireo.bonnet.cut-out`)
provides. Drawn in `mechanicals.rs` (the blower sticking up through the bonnet with the
cut-out's frame in the bonnet's paint, the centrifugal snail at the belt end); a whine in
the engine sound (`EngineCharacter::whine`, orders of the crank: Roots 6 × ratio,
centrifugal the impeller's; `WHINE*`). Telemetry (2) shows clutch and gear wear, downforce,
a supercharger's boost, brake temperatures and pads; hints for fading brakes, worn pads, a
slipping clutch and a grinding synchro. Shops file: car shops the stock brakes, tuning
shops the wing, bucket seat, big brakes, cut-out bonnet and superchargers. Protocol 41–44.
Shots: `FITIDS="vireo.bonnet.cut-out,turbocharger.roots-blower,vireo.wing" WING=12` (parts
by id into the first slot of their kind, the car rebuilt), e.g. with `OFFLINE=1 TRIM=1
FITPAINT=0 CARCAM="-2.4,2.3,3.2"`. Tests: `car::tests_with_parts` (ground effect, lowered
springs, the wing, missing parts' drag, seats, brakes fading), `brakes::tests`, the
drivetrain's `a_worn_clutch_slips_and_wears_on`, `worn_synchros_grind_then_refuse_the_gear`,
`torque_over_what_the_gearbox_takes_wears_the_gear`, `supercharger::tests`,
`fitting::tests::a_big_brake_needs_a_big_rim`, `a_roots_blower_needs_a_bonnet_with_a_cut_out`.
Not done: the dyno's boost knob does nothing for superchargers (fixed pulleys), an
intercooler with a supercharger, superchargers' wear, brake parts per corner (the left
one counts, as for every per-axle part), front/rear brake parts kept apart (any brake
goes in any corner), the tune left as it was after fitting forced induction (as with
turbo kits: retune at a dyno), the whine and fading never heard or felt here.

Milestone 9, trading (2026-11-27, built while the user slept; my decisions are in the
plan's "Decided while building milestone 9"). **Wallet** (`trade.rs`: `START_MONEY`
5 000 €, cents everywhere, `euros` formats "5 000 €"): `accounts.money` (migration 3,
default 500000, a test checks it equals the constant), `Account::money` → `Admitted::money`
→ `Peer::money` (guests and servers without a database: the start amount, for the
session). Money changes only on the server: `Peer::pay(delta, accounts, catalogue)` adds
it and, with an account, sends `Job::Pay` (`Store::pay`: `add_money` (`WHERE money + ? >=
0`) and the whole belongings in one transaction); `Peer::paid` when the database did it
already. The memory's wallet is a lower bound (only other instances' sales add to it): the
lock refresh also reads the wallets (`Answer::Balances`), taken only if the server didn't
change that wallet since asking (`money_changes`, `Server::asked_money`). Told in
`OwnedView::money`; the central menu ("Money") and every `shop_window` show it (`Shops::money`).
`stash::belongings` saves a car being built anew (no state yet) from its spec and waiting
condition instead of dropping it. `db::Answer` is an enum now (logins, balances, market),
`Accounts` sorts them (`answers`, `balances`, `market`). **Used parts shops** (kind `used`):
`trade::used_parts` makes up `USED_PARTS` (12) wares from the `sells` lines every
`STOCK_PERIOD` (30 min; seeded by shop name, period and a per-server number,
`net::trading::Stocks`), worn at random (`wear_out`; rims/tires in the trims' sizes, any
paint), sold at `selling_price` (half of `worn_value`, at least 5 €), into the trunk or
held there; anything brought (trunk or held) bought at `buying_price` (15 %, at least 1 €)
and gone. While catalogue prices are 0, `new_value` is a guess per `Kind` (×2.5 for tuning
ids). **Used car dealers** (kind `usedcars`): `used_cars` (`USED_CARS` 4 of the sold
trims, any paint, `wear_by_mileage` 25–240k km: tires/pads anyhow, clutch, gears, engine by
mileage, nothing failed), `car_price` (40 % of worn value, less up to 45 % by mileage, in
fifties), `Trade::BuyCar { trade_in }` like the showroom (`stash::bought` takes a
condition; `spec_of_trim` builds the worn tree), the trade-in paid at `trade_in_price` (15
% of car and trunk). **Marketplace** (kind `market`, "Marktplatz Baunach" (100, 2450);
needs a database): table `offers` (migration 4: seller, seller_name, what `part`/`car`,
data as saved text, price, made); `Store::offer` (at most `MOST_OFFERS` 20 each, the
seller's belongings without it saved in the same transaction), `offers` (newest 300),
`withdraw` (into the seller's garage), `buy_offer` (in one transaction: the offer must still
be there at that price, `DELETE` counting rows decides between two buyers, buyer debited
(not for a guest), seller credited, into the buyer's garage). The server asks through
`Accounts::offers/offer/withdraw/buy_offer` and answers in the `market` system
(`trading::answered`: tells `ToClient::Market`, a refused offer goes back into the garage,
a seller here is paid at once and told in the chat, the offers re-listed);
`Peer::market_waiting` stops `save_all` overwriting what the database did meanwhile, and
a player leaving waits for those answers (`flush`). Guests buy, never offer. Protocol:
`ToServer::Trade(Trade)` (`Look`, `BuyPart`, `Sell`, `BuyCar`, `Offer { what: Offering,
price }`, `Withdraw`, `BuyOffer { offer, price }`), `ToClient::Stock` (wares and cars),
`ToClient::Market` (`OfferView`: seller, mine, price, `Offered::Part`/`Car(ParkedView)`);
protocols 45–48. **Game** (`dealers.rs`, `DealersPlugin`): window-only shops (no 3D
scenes): the used parts list and "Sell a part", the cars with a car screen (mileage,
condition, trade-in toggle), the marketplace (offers, your offers, "Offer something" → a
price slider over `trade::price_steps`, 1 € to 1 000 000 €), and `dealers::inspect` (every
part in its slot with its wear, focusable rows) for used cars, offers and the garage's
cars (trunk's car screen). Screens go on `AtShop::stash` as `trunk::Screen::Deal`; a window
asks `Look` again every second until something came. Shops file: `used` in Gerach
(−2560, −1700) and Rattelsdorf (3000, −500) selling `*`, `usedcars` in Reckendorf (−1380,
−1150) and Baunach (450, 2330), the marketplace; `every_shop_is_by_a_road` also checks no
two share a marker. Shots: `PLACE="-2560,-1700,150"` (used parts), `"-1380,-1150,150"`
(used cars), `"100,2450,150"` (marketplace), 300 frames. Tests: `db::tests` (`wallet`,
`market`, also in `on_another_database`: passed on PostgreSQL 18 and MariaDB 11.4),
`trade::tests`, `a_used_parts_shop_sells_and_buys_parts` (and the wallet kept),
`a_used_car_dealer_sells_worn_cars`, `players_trade_at_the_marketplace` (offer locked,
wrong price, bought once, paid, withdrawn, paid while away, a guest). Not done: 3D scenes
for the three, shipping straight to the marketplace from a shop, money earned otherwise,
prices at the other shops (so new free parts sold to a used parts shop, or a free new car
traded in at a used car dealer, make money).

The game is about tuning every aspect of a car. Agreed so far (the user's answers):
- **Parts are items** you own (a definition from the server's catalogue + condition),
  fitted to cars or carried. A car is a chassis with parts in slots; the engine is a
  long block plus bolt-ons. Body parts (bonnet, bumpers, wing…, more aero and drag; a
  bonnet cut-out for a supercharger) and most other parts are chassis-specific.
- **Money** is part of the plan, every price 0 € for now (earning comes later).
- **Persistence** in a database: SQLite playing alone, MySQL for a single server,
  PostgreSQL for several server instances sharing it (a player active on one at a time:
  a lock row refreshed while playing, taken over once stale, ~30 s).
- **Accounts**: username and password; a server option allows guests (kept for the
  session only); playing alone uses a default user. (Built: milestone 4, above.) A catalogue entry changed by the
  server changes the items of it; entries are never meant to be removed (if one is, its
  items go, smoothly).
- **What's sold where**: car shops only stock parts, tuning shops standard tuning parts;
  special parts only from multiplayer events (weekly races or so): that's what makes a
  part rare and traded. A stock car is an assembly of part definitions, in trims (more
  than the engine: bigger brakes…); no generated stock parts.
- **Garage park**: one shared place on the map, everyone their own garage there; storage
  only (cars and parts), a limit high enough never to be hit by playing (it only keeps the
  database from growing without end). Switching cars only there.
- **Trunks** are a grid of their own per car (the floor, with a height per area), parts
  placed like Tetris **by the game** (no placing by hand; no folding seats; passenger and
  rear seats are parts that can be taken out, giving a bigger grid and saving weight,
  insulation unchanged; the driver's seat can be replaced, e.g. a carbon bucket); what's
  carried adds its mass to the car (its place in the trunk). Loading and unloading
  everywhere (shops, garage, marketplace); shipping is the paid alternative (free for
  now), also from the garage to a shop (e.g. an engine to wait at the engine shop).
- **Slots**: body parts are all optional (missing ones add drag); every shop can take an
  engine out and put one in, the long block's insides only the engine shop; a car always
  has an engine (trailers maybe later).
- **Shops** (unlimited stock, every chassis; no time, a cost for fitting and for parts
  not brought): the **car shop** sells stock parts and fits any non-long-block part you
  bring; the old part goes into your trunk (if it fits) or the shop discards it.
  **Tuning shops** the same, with aftermarket parts (exhausts, sport cats, body parts).
  A **tire shop** for wheels and tires (individual items, no spare wheel). The
  **alignment shop** and the **dyno** stay; the **paint shop** paints every body part
  fitted; a new body part's colour is picked when buying it, and parts can differ in
  colour. The **engine shop** works only on long blocks (in the car or brought in a
  trunk), sells their internals (the user, 2026-11-27), keeps one engine for you (bring it in the van, come back with
  the car, they swap). Repairs are new parts, except the engine shop's rebore (needing
  oversize pistons). Shops deliver anything to the garage. Before milestones 7–9
  (2026-11-27): two supercharger kits (Roots through a bonnet cut-out, centrifugal), and
  a wallet per account in 9 (shops stay 0 €, marketplace prices set by sellers); see
  the plan's last section.
- **Showrooms** may later sell only some chassis; **used car dealers** and a **used parts
  shop** (buys and sells; new-part shops only sell); buying a car, the old one goes to the
  garage or is traded in, as the player picks; the used parts shop sells cheaper, worn
  parts. A **marketplace**, a place on the map: players bring (or ship) what they offer
  there, which locks it, others buy (rare parts are traded; whole cars too). Offered
  engines and cars show what they're built of.
- A car that can't drive gets a **special rescue**: to a shop or a gas station.
- Crashes don't damage cars. Wear for brakes, clutch and gearbox (too much torque,
  synchros), as for the engine and tires: worse before failing (brakes fade, the clutch
  slips, synchros grind, then refuse). Built in milestone 8.
- Lent cars in events: your own car, trunk and all, comes back as it was.
- Taking an engine out is only a swap (the new one in the trunk or waiting at that shop).
- Accounts are made by the server's admin for now; the username is the name in the game;
  a forgotten password is reset by the admin.
- Parts sold to the used parts shop disappear; what it sells the server makes up.
  Assortments are per shop (the shops file). With several instances everything persisted
  is shared, marketplace included: instances only spread the physics.
- **The body is built of real pieces**: a missing bonnet shows the engine and intake
  under it; upside down you see the silencer, cat or straight pipe.
- Showrooms sell trims, used car dealers worn ones. Weekly races with rewards: later,
  nothing planned yet.
- **One car for now**: the hot hatch with 1.6 and 1.8 engine options, made to look like
  an early-90s hatchback (the user: like an EG Civic; no real names or badges); the other
  twelve chassis go (others come later).
- Engines come out whole (bolt-ons on); the ECU, exhaust after the manifold and cooling
  stay with the car. Rims and tires are items of their own too (loose in trunks and
  garages), put together only at the tire shop. Springs, dampers, brakes, wheels per
  corner. Doors are parts. Gearboxes (gear set, final drive, differential) are worked on
  by car and tuning shops, not the engine shop.

## Multiplayer (agreed with the user, 2026-11-19)

A multiplayer sandbox first; the career comes back later, built on it. Native and web
native offers **"Play alone"** and **"Join server"** (a start menu, chosen at runtime);
the web only asks for the name (and password: an account's, none for a guest) and joins the server the page came from (no playing
alone, no address field; agreed with the user, 2026-11-21). **Alone is joining a server
of the game's own** (agreed with the user, 2026-11-24, so there's one way to play and
alone has shops and events): `net::local::start` runs `net::server_app` (the
dedicated server's setup too) on a thread, joined over `Loopback`, with the repo's
`assets/shops.cfg` and `events.cfg` (embedded), the default traffic (20; the "Cars
around" slider went), chat off, one player; the `LocalLink` stops it when dropped (the
`Connection` goes). `online::Menu::alone` says it's the game's own (the central menu
then has "Join a server" instead of the players). A player never simulates traffic
(`traffic::TrafficPlugin` is the server's; players have `NetworkPlugin`: the road
network and `FactoryLooks`).
No hosting from the game: a dedicated server (`src/bin/server.rs`, native, headless:
`MinimalPlugins` and the world core) runs on the user's VPS.

- **Each player owns their car**: full local physics, engine and force feedback, as
  alone. ~30 snapshots/s (pose, velocities, wheels, rpm, throttle, gear, lights; ~100 B,
  unreliable datagrams). Other players' cars are **remote cars** (model, wheels, sound),
  dead reckoned like traffic (see "Every car from the server alike"); each client
  collides its own car against them and applies the impulse only to itself.
- What others **see and hear** of a car (`Looks`: chassis, paint, wheels, ride height and
  a sound description: layout, cylinders, `ExhaustSound`, turbo) the server works out
  from what it has the car built of (since protocol 27; players sent them before), and
  sends again when that changes.
- **The server owns** the session (players, names, looks), **shared traffic** (one
  traffic world: spawned 260–550 m from the nearest player and out of everyone's view,
  removed past 700 m from everyone, density per area, not per player; sent like player
  cars, as pose and velocities with the wheels' roll and steer, dead
  reckoned (decided while building: the states come at 3–30 Hz, rails would need the
  driver's random route); the server simulates crashed traffic too, players' cars are
  `traffic::Visitor`s there (followed, crashed into, never moved), and a player's car
  pushes itself off traffic on its own side, like off other players' cars) and **interest
  management** (128 m grid, ~600 m radius, rates falling with distance ~30/10/3 Hz).
  A silent player (a hidden browser tab stops the loop) is frozen for others and
  dropped after 30 s.
- **Transport**: WebTransport (QUIC, one UDP port, default 4433) with a real
  certificate; `wtransport` for the server and native client, the browser's
  WebTransport for the web (maybe `xwt` for both clients). Datagrams for snapshots and
  traffic, streams for the handshake, looks and events. Handshake checks the protocol
  version and the terrain's fingerprint. Dev only: without a certificate the server makes
  a throwaway one and a native client can skip verification (browsers can't join).
- **Nix**: `packages.default` (`native`) is the game alone (`--bin car-sim`; with every
  binary, each a thin-LTO link of Bevy, it took about four times as long; the editors
  have no package, they're for the dev shell; agreed with the user, 2026-11-24),
  `packages.server`, `packages.web`, `nixosModules.server`:
  `services.car-sim = { enable; domain; port; openFirewall; traffic; maxPlayers;
  web.enable; }`, certificate via `security.acme` (nginx's `enableACME` with
  `web.enable`, else ACME's standalone listener on port 80), taken up by the server
  itself when renewed, hardened service as system user `car-sim` (not `DynamicUser`: systemd refuses a dynamic user named like the existing `car-sim` certificate group, seen on the user's VPS). With `web.enable` the web build is served at the
  same domain, and its "Join server" connects to its own host.
- Steps: 1. remove the career (done); 2. what's seen and heard decoupled from `Car` (done:
  `looks.rs`, `Looks` (build, body offset, linkage per axle, `audio::EngineCharacter`)
  and `Motion` (arm angle, steer, spin per wheel, `audio::EngineRun`), filled from a
  `Car` by `show_cars`; visuals and engine sound read only these); 3. headless world core, `bin/server.rs`
  running traffic around several centres (done: `WorldPlugin`; `bin/server.rs` is
  `MinimalPlugins` + `LogPlugin` + `WorldPlugin` at 60 updates/s, every schedule on the single-threaded executor (idle 0.6 % of a core instead of 2.3 %; the user's VPS has one slow vCPU, about 4–5× slower than the sandbox's cores, and fell behind with traffic until the traffic paths were cached); traffic populates
  around every `Car` and `traffic::Anchor`; the
  server still links all of bevy, so it needs the render/audio libraries at runtime); 4. protocol and server/client plugins over a
  loopback transport (done: `src/net/`: `wire.rs` hand-written encoding, checked on
  reading (finite floats, catalogue indices, sane sizes); `protocol.rs` `ToServer`,
  `ToClient`, `CarState` (~120 B); `link.rs` `Link`/`Listener` traits, `Loopback`;
  `server.rs` `Server` resource + `Peer` entities (`traffic::Anchor` while heard
  from), 128 m grid, looks re-sent by revision; `client.rs` `Connection` resource,
  remote cars (then `Proxy`, now `RemoteCar`) with `Looks`/`Motion`, shown where they
  are by now on the sender's clock (offset = least our-clock-minus-theirs seen, rising at most 5 ms per second (`ClockOffset`; by time, not per sample: the shared traffic clock, rising 1 % per sample, sat mid-jitter, so cars close by were predicted to before their latest state and jerked back and forth); states stamped at the end of their simulation step (`net::stamp`)), contacts in
  FixedUpdate push only our car (since 2026-11-19 dead reckoning instead of drawing
  0.1 s in the past, which looked laggy, worst for traffic at 3–10 states/s: `predict`
  carries the latest state on along its turn (angular velocity) with the acceleration
  between the last two states in the car's frame (estimated on the client, agreed with
  the user: sending it would only gain one state at the onset), stopping rather than
  reversing, for up to 0.8 s; `Drawn` closes on the prediction with a 0.1 s time
  constant, snapping past 5 m; wheels roll on at their last rate; drawn as of the last simulation step's end (`net::stamp`), when our own car and the camera are, not now: a car keeping pace shook by its speed × the overstep); the plugins idle without the resources; tests in
  `net/mod.rs` step time by hand with `TimeUpdateStrategy::ManualDuration`); 5. WebTransport,
  start menu, join dialog, player name, name tags, player list (done: `net/quic.rs`
  (wtransport, tokio runtime "network", stream framed by u32 length + datagrams ≤ 1200 B,
  certificate files reloaded on change via `Endpoint::reload_config`), `net/web.rs`
  (hand-made wasm-bindgen bindings, JS objects in a thread-local by link id),
  `net::connect`, `online.rs` (`OnlinePlugin` in the sandbox: menu, players window,
  name tags, joining cars placed in lane spots near the spawn by player id),
  `bin/server.rs` options `--port --cert --key --traffic --max-players`; tried with a
  local server and two headless games: both joined, saw each other, shared traffic); 6. shared traffic (done; since protocol 5 sent like any car, see below;
  since 2026-11-24 only servers simulate traffic);
  7. Nix packages and module, README for the VPS (done: `packages.server`
  (crane, `--bin server`), `packages.web` (= `wasm`), `nixosModules.server` =
  `nix/module.nix` (options `enable domain port openFirewall traffic maxPlayers
  web.enable web.package package`; no `reloadServices`: the server stats the
  certificate files every 60 s and reloads them itself, so renewals drop nobody and the
  provisional self-signed certificate at first boot is replaced); `checks.nixos-module`
  evaluates both modes into a JSON summary. Building flake outputs here works through
  `nix build 'git+file:///repo#…'` (copies only tracked files; stage new ones first),
  unlike `.#…`).
- Timing logs (for cars jumping): the server warns every 10 s that it didn't keep up
  (frames over 40 ms, simulation behind the clock; nothing when fine); the client logs
  every 10 s the farthest a new state of a near car (sent ≥ 10/s) landed from its
  prediction and how many states came over 30 ms past the quickest (meaningless in shots
  here: a few frames per second; `shot.sh` hides info lines anyway).
- Checking a deployed server from outside: `CAR_SIM_SERVER=host cargo test --profile
  release-dev join_a_real_server -- --ignored --nocapture` (says hello with this build's
  protocol and map, as a guest, and prints the answer: a server without guests refuses
  it, which shows it's up). Closing a QUIC link waits up to 2 s for the send stream to be
  acknowledged (`FINISH_TIMEOUT`): closed at once, a refusal never arrived (found
  2026-11-26; the version and reload refusals had the same problem). "No answer" with the page loading over HTTPS
  means UDP is dropped before the server (seen 2026-11-19 on the user's IONOS VPS; QUIC
  out of the sandbox works, e.g. `curl --http3-only https://cloudflare.com`).
- **Updates** (discussed with the user): the server checks only `PROTOCOL` and the map
  fingerprint, so client-only changes (driving physics, engine, tires, FFB, aids, UI)
  need no server update, and old and new clients play together. Bump `PROTOCOL`
  whenever a message's content changes (`protocol.rs`: `CarState`, `Looks`, `Motion`,
  a new message). Catalogue changes (cars, parts, paints…) need only a server update
  since the server sends its catalogue (see "Server catalogue"); a new kind of part
  effect or a catalogue field the game reads needs both. The server needs updating
  too for: traffic behaviour
  (simulated on the server), and a re-bake (fingerprint). With `web.enable` the web
  build updates with the server (`nixos-rebuild` restarts it, dropping everyone).
  A browser left on an old page (its cache) is refused with "Press Ctrl+F5 to reload
  the page without the browser's cache" first (asked for by the user, 2026-11-21): the
  server words it, since the old page can't change; it knows a browser by the
  `Origin` header (`Link::browser`), and adds it when the page's protocol is older or,
  with the same protocol, its map differs.
- **Server settings for gameplay** (agreed with the user, 2026-11-21): command line
  options plus a NixOS module option each. The first: who's on the map (M) of the other
  players, `--map-players self|near|all` (default near) and `--map-near` (m, default
  200) / `services.car-sim.mapPlayers`, `mapNear`. The client knows nothing of it: the
  server sends `ToClient::Map` (id, position, velocity, forward, paint; unreliable) once
  a second with whoever the setting shows; the client keeps each for 3 s, carries it on
  at its speed (`Connection::on_map`), and `online.rs` hands them to the map as
  `map::MapCars` (the map itself knows nothing of networking). Protocol 3.
- **Spawn** (agreed with the user, 2026-11-22: not part of the map any more; format 14
  dropped it from the bake): `terrain::Spawn` (map point, optional compass heading),
  `Spawn::DEFAULT` in Gerach for playing alone, tests and the editor's camera (also a
  resource). The server's `--spawn "x z [heading]"` / `services.car-sim.spawn = { x; z;
  heading; }` (default: the same) goes out in `Welcome` (protocol 9); `online`'s
  `place_on_joining` picks lane spots around `Connection::spawn` (any direction: the
  heading only picks the first road spot). One point for now.
- **Ping** (asked for by the user, 2026-11-21): the server reads each connection's QUIC
  round-trip time (`Link::rtt`, `quic.rs` stores `Connection::rtt` every 0.5 s; loopback
  0) and sends everyone `ToClient::Pings` every 2 s; the players window lists them, ours
  too. Protocol 4.
- **Chat** (asked for by the user, 2026-11-21): server option `--chat on|off` (default
  on) / `services.car-sim.chat`, told to clients in `Welcome`. `ToServer::Chat`,
  `ToClient::Chat { from, text }` (`from` = `protocol::SERVER` (0) for the server's own
  lines: who joined/left, "Not so fast" past 5 lines in 10 s); `clean_chat` makes one
  line of ≤ 200 characters. Client: `Connection::chat` (last 50), `write`; `chat.rs`
  shows the last 8 bottom left, fading after 20 s; T opens the field (Enter sends, Esc
  cancels). Shots: `SAY="text@frame"` writes in the chat.
- **Telegram** (asked for by the user, 2026-11-20): `server --telegram-token-file F
  --telegram-chat-file F` (module: `services.car-sim.telegram = { tokenFile;
  chatIdFile; }`; both secret, the user manages them with sops; the module hands them
  over as systemd credentials, `%d/telegram-token` and `%d/telegram-chat`) sends "The server
  started" and who joined/left (`net::Presence` messages from `net::server`) through
  `ureq` from a thread of its own.
  Tried here only against Telegram with a fake token (401, logged, token kept out).
- **Every car from the server alike** (agreed with the user, 2026-11-21, so players can
  hide in traffic; done, protocol 5): one counter (`traffic::CarNumbers`, `CarId`) on
  the server for traffic and player cars, never tied to a `PlayerId` on the wire
  (players list, chat, pings, map keep `PlayerId`). `ToClient::Cars(Vec<(CarId,
  CarState)>)` (unreliable, 8 a message), `ToClient::Looks { car, looks }` (reliable, by
  revision; traffic's too), `ToClient::Tag { car, name: Option<String> }` (what name the
  car shows, if any: players' names today, named NPCs or none in a game mode later),
  `ToClient::Gone { car }` (out of range or gone; the client's 2 s timeout is only a
  fallback). What a player was told of a car (`server::Told`: looks revision, tag, next
  state due) is forgotten when it leaves range, so it's told afresh on return. The
  server stamps every state on its own clock (player states converted with a per-peer
  `ClockOffset`), so the client has one clock (`Connection::clock`) and a car's own clock
  gives nothing away. Server `send`: one list of cars (players' from their `Peer`,
  traffic's from `TrafficCar` + `Looks` + `Motion`, its state at the centre of mass,
  `TrafficCar::transform`/`center_velocity`) and one loop over them per player. Client
  joined: `RemoteCar` (states, `Drawn`, body for contacts) with `Looks`/`Motion` and an
  optional `client::Tag`, dead reckoned (no rail prediction); looks and tags that come
  before a car's first state wait in `Connection::known` (30 s); no traffic of its own;
  `forget_cars` removes the remote cars when not joined. Name tags
  follow `Tag`. On the server: `TrafficCar` simulates as before,
  `traffic::show` gives it `Looks` (`FactoryLooks`, per chassis with its base engine,
  16 ms for all 13) and `Motion` (steer, spin from rolled, suspension at rest, the
  engine from the driver's `traffic::powertrain::Shifter`)
  and a `Transform` at the centre of mass, and `visuals.rs` draws it like any car (shared
  materials, bodies cached per chassis and paint). Remaining tells: traffic's suspension
  doesn't move, how it drives.
  Tried with a local server and two headless games: name tag over the other player,
  traffic shown, a leaving player's car gone at once.
- **Parallel worlds** (planned with the user, 2026-11-21, not built): groups of players
  in a challenge (race, hide and seek) separate from the normal world and each other.
  Built on the above: a world component on cars, peers and anchors; the sending loop
  sends only a player's world, `populate` counts density per world, traffic `step`
  collides only within a world; the client needs nothing (maybe a message for the UI).
  Per-world settings like `--map-players self` and no tags for hide and seek.
  Being built for the events (see "Events").
- Not done yet: the browser path tried in a real browser (no
  browser here; it builds and links), rate limits on what a player sends (chat has one).
- Later: sanity checks on snapshots, Safari, career.
- **Name tags** (agreed with the user, 2026-11-21: in the scene, covered like anything
  else; on or off is the server's choice, `--name-tags on|off` (default on) /
  `services.car-sim.nameTags`: off, players' cars get no `Tag`; no client toggle since
  2026-11-21, at the user's wish):
  `name_tags.rs`, each name rasterised once with ab_glyph in egui's Ubuntu-Light (no
  bevy_text in the build) into a texture on an unlit, blended quad facing the camera,
  scaled with distance (constant size on screen), within 150 m. Systems that move the
  camera after `Update` go before `name_tags::PlaceTags`.

## Wheel support

Wheel and pedals come through gamepad input (gilrs), bound in a control profile in the
central menu (Esc, Settings, Controls) and saved to a config file. `src/wheel_base.rs` sets the base's rotation range
and plays force feedback, both through hid-fanatecff on Linux.

### Design decisions (agreed with the user)

- Target: Linux with hid-fanatecff. Force feedback only there; input works everywhere.
- Force feedback: the rack torque (`sim/steering.rs`, `Rack`, averaged over the
  substeps): every front tire force's moment about its steering axis from the linkage
  (the axis tilted back for 30 mm of caster trail, which the linkage lacks), pneumatic
  trail (falls with slip angle and slip ratio), Fx at the carcass' sideways deflection,
  and the driveshaft torque along the shaft (inner joints by the differential, 0.12 m
  left of the middle with a transverse gearbox: torque steer). Asked for by the user
  (light cars felt too light, wanted bumps and FWD/AWD power): full scale is 0.055 m ×
  the front axle's weight (`full_scale`, every car ~0.6 at 0.7 g), and "Road feel"
  (`ffb_road`, default 2.5) scales what's off a 0.03 s low-pass. `steering_feel`
  (ignored) prints every car. Damping/friction from the base's tuning
  menu (evdev 0.13 can't send damper parameters). Rate-limited, fades in with speed.
- The game sets the base's rotation range (default 540°) and restores it on exit;
  either end is full lock. If it can't, soft lock like BeamNG: the base's range is read
  from sysfs (readable even when not writable) or given in the menu, the axis is
  scaled to the game's range, and force feedback pushes back past either end (full
  force 10° past; stays on with the tire forces switched off). Only the force feedback
  device's axis covers the base's range (`WheelBase::axis_range`; on Linux the device
  taking a constant force): any other steering axis, a pad's stick, covers the game's
  (it snapped to full lock at a small push with the base plugged in, 2026-11-24), on a
  curve. **Curves** (asked for by the user, 2026-11-24): each control profile has
  `input::Curves` for steering, gas and brake. Drawn curves since 2026-11-24 (asked for
  by the user, like a MIDI controller's): `input::Curve` is 2–9 points (ends at push 0
  and 1, any height), joined by a monotone cubic (Fritsch–Carlson, no overshoot) and
  baked into a 257-entry table whenever the points change, so `Curve::at` is a lookup
  and a lerp. `input::Preset`s: Linear, Gentle (^1.25, gas default), Progressive (^1.5,
  brake default), Stick (^1.8, steering default), Precise (^2.6), Smooth (S), Quick.
  `wheel.cfg`: `curve steer|throttle|brake x,y x,y …` after the `profile` line; an
  older `curves s g b` (exponents) or `stick_curve` becomes `Curve::power` (five
  points). The menu: Controls, Bindings, Curves, a screen per curve
  (`menu::curve::editor`): the graph with the control's push now (`input::push_value`),
  left/right pick a point, choose grabs it, arrows move it, choose keeps, back puts it
  back (`CentralMenu::grabbed`/`cancel_grab`); the mouse drags, double-click adds,
  right-click removes; rows for the preset, adding and removing points. Always applied, no guessing what's a stick (triggers report as
  buttons 0..1 or axes -1..1, some pedals share one axis): a wheel profile sets them to 1;
  the steering's never applies to the force feedback wheel's axis. A pre-profile file's
  "Wheel" profile is linear; an older global `stick_curve` goes into every profile.
- Starter (asked for by the user, 2026-11-22): the ignition button (I or bound) held
  runs the starter (`Drivetrain::starter`: `STARTER_TORQUE` falling to nothing at
  `STARTER_SPEED`, ignition on); the ECU fires after `SYNC_TURNS` revolutions and the
  engine has caught past `CATCH_RPM` (0.5 s for small engines to 1.25 s for the V12);
  pressed while running it switches the engine off (`switch_off`), which stays off (the
  automatic clutch's restart is only for stalls). Cranking is heard as the engine at
  the starter's speed, with the starter's whine (`audio/effects.rs`).
- Clutch pedal: engine can stall, the starter restarts it. Without a bound pedal the
  clutch is automatic and the engine never stalls. The pedal works like a real one
  (`drivetrain::ENGAGEMENT`, asked for by the user: it had to be floored and stalled at
  a slight lift): fully engaged over the top 12 %, apart over the bottom 28 % (and gears
  go in from there), biting at 55 % with 3 % of the capacity (creeps at idle), rising
  steeply to the top. On idle alone it pulls away when held around 45–50 % until
  rolling; with 10 % throttle even a half-second release works (stock coupé, 350 N·m).
- No automatic gearbox for the player (removed at the user's wish, 2026-11-20; real
  automatic gearboxes are to come later). `Drivetrain::automatic` (rpm-based shifting)
  is only kept for the test drivers (consumption, flat-out runs).
- H-shifter: with a clutch pedal bound, a gear only goes in with the clutch pressed
  (otherwise neutral, grinding), or when the engine already turns within `SYNC_RPM`
  (150) of the gear's speed (engine off and standing, or rev-matched; asked for by the
  user, 2026-11-22). Without a pedal, the automatic clutch shifts, blipping the throttle
  up to the lower gear's speed on a downshift (`BLIP_RPM`). Paddles shift
  with the automatic clutch even when a pedal is bound.
- Each H gate is a button held while the lever is in it; neutral is "no gate held".
- Put in neutral by the game (`Drivetrain::neutral`: a reset, the rescue, a round's start
  or end, a car built anew), the gate the lever is still in counts as neutral until the
  lever moves (`Drivetrain::parked`, `lever`; asked for by the user, 2026-11-26: it went
  straight back into gear and stalled or ground). A car put down afresh doesn't know the
  lever, so the first gate it sees waits too. Protocol 30 (snapshot fields).
- No hill hold (removed at the user's wish): the handbrake is a parking brake worked
  by Space or a handbrake button (`Car::parking_brake`, on when a car spawns or resets,
  a red (P) under the gear); a handbrake lever (an axis) works directly on top of it.
  `car::parking_brake_rule` (the user, 2026-11-24, "keep it simple"): pressed while on,
  it comes off; pressed standing (under `PARKING_SPEED`, 0.5 m/s), it goes on; pressed
  while driving, no toggle, the handbrake only brakes while held; **any gas releases
  it**, whatever the gear or clutch. Holding it until the car stops doesn't park it.

- Gamepad rumble on crashes (asked for by the user, 2026-11-24: only without settings):
  `rumble.rs`, bevy's `GamepadRumbleRequest` (bevy_gilrs plays it), on the gamepads the
  profile in use binds, never the force feedback wheel; the crash rule is the crash
  sound's (`effects::CrashGate`; read in `Update`, before `effects::cue` clears the
  knocks); not while away, held or while the server has the car; native only. Strengths are a first guess
  (`WEAK`, `LENGTH`), never felt here.

### Still to check on the real wheel

- "Test: push left" in the menu (Controls, Force feedback): turn on "Reverse direction" if it pushes right (the sign
  convention of hid-fanatecff couldn't be verified without hardware).
- Tune the default force strength (0.5) and road feel (2.5) on the real wheel; the
  physical road content is small (std ~2 % of full scale on a rough road, ~5 % floored
  in a FWD car), the 0.5 per-step smoothing in `force_feedback` also eats some of it.
- That the game can set the rotation range (see below; last seen failing), and the soft
  lock where it can't (strength and feel on a direct drive base; may need damping).

### Troubleshooting on the user's machine

The base is a CSL DD in PC mode (USB `0eb7:0020`); it shows up as two input devices,
both named "Fanatec FANATEC Wheel" (padded with spaces). NixOS config uses
`hardware.hid-fanatecff.enable = true`, which adds the kernel module and its udev rules.

- **"No constant-force support" on every device:** the base is bound to `hid-generic`
  (input works, no force feedback) instead of the `fanatec` driver. Check
  `basename "$(readlink -f /sys/class/input/eventN/device/device/driver)"`,
  `lsmod | grep hid_fanatec`, `sudo evtest /dev/input/eventN` (EV_FF with FF_CONSTANT).
  A reboot into a kernel with the module fixed it; replugging the base can also help.
- **"Couldn't set the wheel range in …/range: Permission denied":** the game falls
  back to the soft lock (the menu says "Wheel base is at …°"). The `range` sysfs file is root's; the udev
  rule only does `chown -f :games` on it when the device is added. Needs a `games` group
  (NixOS doesn't create one; `users.groups.games = { };`), the user in it (log in
  again), and the rule to have run after that (replug, or
  `sudo udevadm trigger --action=change --subsystem-match=hid`). Check with
  `ls -l /sys/module/hid_fanatec/drivers/hid:fanatec/*/range`, `getent group games`
  and `id`. If the group owns it but can't write, the mode needs `g+w` too.
- The menu (Controls, "Wheel, pedals and gamepad") shows "Wheel base set to 540°" once the range was written, and names the
  force feedback device or what's wrong with each candidate.

## Cars

Agreed with the user: lots of variety; a chassis is independent of engines and the
other components, engines are shared between cars, and a bigger (or better) engine can
be dropped into another car. **Space is the only hard limit** for a swap. Front- and
all-wheel drive exist. One car at a time, picked from a menu. Later this becomes a game
with a campaign, upgrades and personal cars, so a car is an instance built from
catalogue entries, and things like swap subframes are named so they can be priced.

- **Catalogues** (data in a `catalogue::Catalogue`, the server's, see "Server
  catalogue"; entries referred to by their place; engine versions also by code). The
  built-in one (`src/catalogue/`): `chassis.rs` has the chassis (only the Vireo hatch, B16 and B18, since 2026-11-26,
  see "Owning cars and parts"; the other twelve (Minim, Linnet, Tarn, Halden, Skarn,
  Harrow, Calder, Rheon, Vespera, Wisp, Drover, Carrow) and their engines are in git
  history before that; names are made up, no real cars), gearboxes (the two
  transverse ones), clutches, suspension kits and paints (the tire
  compounds' rubber is `physics/tire.rs`'s `COMPOUNDS`, named in `catalogue/parts.rs`).
  `engines.rs` has the families (the B series, versions B16 and B18; `EngineHardware::default`
  is still the W20 2-litre four everything is scaled from). Tests needing another kind
  of car change the one: `catalogue::builtin_changed` (heavier, rear-wheel drive…).
- **A car** is a `CarBuild` (chassis, paint, gearbox, clutch, final drive, limited-slip,
  suspension, a `Wheel` per axle) plus its engine's `Parts` (family, version, a part per slot, ECU).
  `CarSetup::new` works everything the simulation needs out of them: the engine's and
  the gearbox's masses sit where the chassis' bay has them, which gives the balance and
  the inertia; springs and dampers come from ride frequencies and the weight on each
  axle (`AxleSetup::tune`); brake torque is a share of the stock tires' grip (about
  0.8, so a floored pedal doesn't lock wheels on a dry road) and its balance follows the
  weight shift. The linkage is the same double A-arm for every car, moved to its track
  and wheel size.
- **Rims and tires** (`wheels.rs`, agreed with the user: sizes like real ones, every car
  on a matching factory size, upgradable per axle): a `Wheel` is a rim (diameter and
  width, tenths of an inch), a `TireSize` (`205/55 R16`) and a compound. A tire goes on
  rims of its diameter within 1.25" of its ideal width (0.8 × mm / 25.4); the chassis'
  `room` per axle limits the outside diameter, the width and the smallest rim (brakes;
  brake swaps are meant to come later). The body and its wheel arches stay built around
  the factory wheels. Size sets: mass (rim, tread, sidewalls, like real ones) and spin
  inertia (`chassis.hub` is the corner without its wheel); the tire's design load
  (14.5 kN per metre of width, × √(diameter/0.64)), at which it grips its compound's
  friction (`tire::GRIP` 1.05 × compound), with load sensitivity 0.27 (wider grips
  more, about 5 % for a fifth more width; there's no per-chassis grip any more); tread
  heat capacities, cooling and life ∝ tread area; rolling resistance per compound ×
  √(width/215 mm); a lower sidewall is stiffer and peaks at a smaller slip angle; extra
  tire front adds drag. Factory compounds: Minim and Drover touring, Calder, Rheon and
  Vespera sport, the rest street. `tires_are_sized_for_their_cars` holds three cars at
  70 % grip; `cornering_temperatures` (ignored) prints every car. Sandbox: per axle in
  4 (another rim diameter or tire width keeps the outside diameter).
- **Anti-roll bars** (asked for by the user: a part, never adjustable; each chassis has
  its own sizes per axle, `Chassis::bars`, `RollBars`; picked in 4; the suspension kits are only
  springs, dampers and camber): solid bars, roll stiffness ∝ diameter⁴ from a 20 mm
  rate. In the sim each axle's bar is a force ±rate × the travel difference on the two
  corners' arm equations (`Corner::bar_force`, set every substep, so the substep loop
  runs over substeps outside and corners inside). Factory bars (real-car style, agreed):
  roll 1.4–5 °/g at 0.5 g, load transfer front-biased on RWD, rear-biased on FWD
  (`roll_stiffness` prints every car; before the bars, roll was 2.5–9.4 °/g with the
  transfer following the springs). The power-slide bot got shallower slides with less
  snap back on most cars (Halden W20T and Rheon H30 now caught), but the Tarn S30 now
  spins and the Wisp B16 snaps back 28° (was 6°). Not done: hollow bars, a bar's mass.
- **What fits** (`Chassis::fit`): the engine's length, width and height against the
  bay's (a transverse engine needs its length plus 36 cm for the gearbox across the
  car). That's all that can stop a swap. An engine family the factory never offered in
  the chassis "needs a swap subframe" and a gearbox no factory car pairs with the family
  "needs an adapter plate": both are derived and only shown (nothing to buy yet).
  A gearbox must be of the chassis' kind.
- **Headers depend on engine and chassis**: the bay's `header_room`, less when the
  engine fills the bay sideways, caps the exhaust primaries' length
  (`Parts::header_room`, set by `CarBuild::install`). A swapped engine can so differ
  from the factory's on the cylinder side and needs its maps baked. Factory offerings
  must not be capped (a test checks), or the game would bake at startup.
- **Parts are relative** to the engine they go onto (`parts.rs`: a stroker is +7 %
  stroke, turbos are sized by displacement), with labels worked out from the result.
  The cylinder head (port flow) is a slot too: the cycle simulation's valves and ports
  scale with it, which is what lets the sporty engines breathe at high rpm.
- **Bodies** (`body.rs`) are generated: a deck outline and a cabin outline in metres
  from the nose and above the ground, plan taper, tumblehome, and details as boxes. The
  lower body is a loft with wheel wells; the cabin a loft with glass, roof and pillars;
  an open car's windscreen is a one-sided pane in a frame (two cabin points). One mesh
  with vertex colours, in pieces (see "Owning cars and parts"). The cockpit is placed by
  the chassis' eye point.
- **Showroom (car dealers) and workshops** are in `garage/` (`showroom.rs`, `fit.rs`); the
  server builds what's bought or fitted, and the game builds what it's told
  (`client::take_your_car`): a changed `CarBuild` respawns the car's entity (its meshes
  are children); an engine-only change swaps it in while driving.
- **Traction control** (my addition, on by default, in the central menu's driving aids): without
  it full throttle on the keyboard just spins the wheels of anything strong. It eases
  the throttle to hold the driven wheels at a slip ratio of 0.12, at a rate that follows
  the slip smoothly (no chopping; turbos still hunt a little with the boost lag).
  Traction control and ABS are the driver's, not the car's (asked for by the user):
  `car::DriverAids`, saved in `~/.config/car-sim/settings/aids.cfg`, handed to the
  active car by `drive`.
- **ABS** (asked for by the user, on by default, next to traction control): per wheel,
  eases the brake share to hold a slip ratio of 0.1, the same smooth way (`ABS_*` in
  `sim.rs`), off below 2 m/s. Without it, braking while steering locks the inside
  front wheel of most cars and they go straight on (`abs_keeps_the_wheels_turning`).
- **A clutch slips** when the engine makes more torque than it passes, so a swapped
  engine may need the sport or race clutch. Gearboxes have no torque limit yet.
- Gotchas met: engines much bigger than 2 litres on light flywheels hunt at idle (the
  V8/V10/V12 flywheels are heavier than realistic for that); wild cams idle faster
  (ECU idle speed follows the cam). With all-wheel drive, locked front wheels drag the
  rear ones along through the centre differential. Unsteered test runs leave the road
  within a few seconds in the quick cars.
- **Fuel** (`physics/fuel.rs`; filled at gas stations (95/98/102, the tank's octane mixes by
  volume, `Fuel::fill`, and the car sets `EngineBuild::octane` from it every step); R
  keeps the fuel; with starvation): each chassis has a `Tank` (litres, centre
  behind the front axle and height; mid-engined Calder and rear-engined Rheon up front).
  The engine reports what it injects (`Engine::fuel_flow`) and gets `fuel_supply`, a
  share of it. Level kept in f64 (a step's burn vanishes against a full tank in f32).
  The fuel is a point mass at the tank: `RigidBody` has a centre-of-mass offset from its
  reference point (`center`, `load`, `add_force_at_center` for gravity and drag), reset
  and reloaded each step from the empty car (`Car::load_fuel`). Springs, tires and
  brakes are matched to half a tank (`corner_mass`). Starvation: the fuel needs 1 % of
  the tank plus 12 % per g (sideways and lengthwise, against the floor, so slopes
  count) to cover the pickup, sloshing over 0.4 s, the lines bridging 0.25 s. Nearly
  empty in a long corner it leans out until the car slows enough. Empty, the engine
  stops; with the automatic clutch it restarts by itself once fuel comes back. Cars
  start full. `consumption` (ignored) prints every car at 100 and 150 km/h; the coupé's
  2-litre does about 7 L/100 km at 100, idling 0.7 L/h.
- **Collisions** (`physics/collision.rs`, `obstacles.rs`): a car's body is one box
  (`CarSetup::hull`, from the body's outline, widened to open wheels). 26 points on it
  (corners, edge and face middles) push against the ground; houses (`buildings::Shell`,
  up to the ridge) and tree trunks (`trees::Tree::trunk`, squares; bushes none) are
  `Upright` boxes in a CSR grid of 8 m cells in `Terrain::obstacles` (610k, ~17 MB,
  built by a startup system in ~0.15 s; empty in tests unless set). Box against box by
  separating axes (edge axes need a 5 % better overlap to win), the contact point by
  projecting back and forth between the boxes. Impulses (sequential, accumulated, with
  friction, restitution above 1 m/s, 40 % of the overlap pushed out per step) between
  `RigidBody::integrate_velocity` and `integrate_position`. Car against car is
  `sim::collide_cars`, run over all pairs in `simulate` after the steps (only one
  player car in the game; traffic has its own, see below). No damage, sounds or wheel collisions yet; a crash at speed
  just stops the body (wheel spin is left as it was).
- **Traffic** (`traffic.rs`, `traffic/network.rs`; agreed with the user: no real
  physics needed, following the roads matters, crashes at junctions are fine, 0–60 cars, default 20 (the
  server's `--traffic`); no HUD or map changes).
  The network: roads 4 m and wider (not water) split into edges at shared OSM points,
  a lane each way a quarter of the width right of the centreline (1.05–2.6 m), 1781
  edges / 151 km, built in ~5 ms. Speed limits every 20 m, German defaults from what the
  map has: open road 100 (6 m+), 80 (4.5 m+), 60; unpaved ≤ 50; 8 houses within 90 m =
  built-up, 50, or 30 under 5.5 m wide. Drivers: front axle on rails along the path
  (lanes joined by quadratic corners towards where the lanes' directions meet), rear axle
  dragged, body set on the ground under the axles. Speed: intelligent driver model
  towards the lowest of limit × keenness (0.88–1.1) and bend speeds (2.5 m/s² sideways)
  ahead, braking at 2 m/s² for them; leaders are anything reaching into the lane ahead
  (oncoming cars only with their middle within 1 m). Random way at junctions (wide roads
  likelier), U-turn at dead ends. Any box contact with another car crashes it: a loose
  `RigidBody` on its box (down to the ground) with ground and obstacle contacts, asleep
  once still. Spawn 260–550 m from the player (50 m at the start), despawn past 700 m
  (crashed 350 m). `drivers_follow_the_roads` runs 30 cars for a minute (~1 crash at a
  merge, about 0.21 ms per step for all; each driver keeps its path, rebuilt only when the route changes: rebuilt every step it was 0.85 ms, and a small VPS server couldn't keep up).
  **Engines and gears** (`traffic/powertrain.rs`, asked for by the user, 2026-11-22: proper
  engine sound and accelerations, cheaply): per chassis its factory gears, final drive,
  driven wheels and the base engine's full-throttle torque curve (`Powertrain`, every
  500 rpm, 55 ms for all 13, made when first needed); per driver a `Shifter` (keenness
  sets the shift points, launch rpm and how much of the torque it uses, 45–80 %, but
  enough for 0.6 m/s² if the car has it): shifts up earlier the less torque it would
  take, later wanting more than it gets, down below idle + 450 or when the gear can't give
  what the IDM wants (kickdown); clutch slips pulling away; load = engine torque for the
  wheel force (acceleration, slope from the body's pitch, rolling, air) over the curve;
  0.4 s without pull per shift; turbos spool with rpm and load. The IDM's acceleration is
  capped by `most_acceleration` (0–100: 23 s keen; calm 23 s Vespera to 37 s Minim; all
  do 60 up 10 %; `traffic_shifts_through_the_gears` prints every car). Only the server's
  `Motion` changes: no protocol bump. Not done: right of way, overtaking, sounds,
  wrecks blocking a lane hold the queue behind them until they despawn.
- Not done: the engine sound per layout was never listened to (no audio in the
  sandbox); other suspension types; turbo kits limited by bay room; prices and saved
  cars; gearbox and clutch damage; collisions of the body.

## Handling

The user drives with the wheel and found slides hard to catch on the power (spin, or
snap back the other way). `src/sim/handling.rs` has a power-slide test on flat ground
(`Terrain::flat`, tests only) with a bot driver: circle, floor it, catch after 0.15 s by
pointing the front wheels where the front axle goes (less a little against the yaw),
lifting more the further it's sideways. `power_slides` (ignored) prints every car with
and without traction control; `trace_power_slide` prints one step by step
(`SLIDE_CAR=chassis,engine SLIDE_TRACE=1`). Findings so far:

- The pedal moved the throttle plate linearly, so the first third of the pedal gave
  nearly all the torque. Now a progressive linkage (`drivetrain::LINKAGE`, about
  proportional at 3500 rpm). This was the biggest single factor for the bot.
- Snap back comes from countersteering to zero front slip while the rear is still
  saturated; the rear keeps yawing the car until it swings the other way. Real drivers
  unwind early, helped by the caster pulling the wheel: mechanical trail is now 30 mm
  (was 15) and the force feedback may change faster.
- Load sensitivity (`TireParams::load_sensitivity`, then 0.1, now 0.27) made slides
  slightly deeper, not easier, but makes axle balance matter.
- Relaxation (asked for by the user): `Corner::slips` are the slip ratio and the slip
  angle's tangent the tire acts on, lagging the patch (`TireParams::relax`, exact
  exponential over the substep; standing, they integrate the creep, so the carcass is a
  spring). Lengths (`TireParams::relaxation`): 0.5 m across on the reference tire
  (`RELAXATION`, × sidewall height^0.4, × √(load/design), ÷ pressure stiffness), 0.15 of
  that along; shortened by the Magic Formula's local slope (to 0.1 at saturation), since
  the first-order model otherwise winds up a huge deflection in a slide and reverses far
  too late (that made the bot spin the Halden). Low-speed damping below 4 m/s (0.01 s
  along, 0.03 s across) against the unshortened carcass lengths; damping against the
  shortened one blew up on locked wheels. No `LOW_SPEED` slip floor any more (only the
  steady `force` for tests). `tires_hold_a_parked_car` pushes a parked car 0.15 g
  sideways and, on the brakes, along. Snap back in caught power slides grew a little
  (Tarn W20 4.6° → 6°, Halden S30 12° → 15°); the Halden W20T without traction control
  now spins with the bot.
- With the bot, mid- and rear-engined cars still spin (plausible), as does the 6.2 V8
  Harrow floored in second, and the 5-litre V8s snap back; the other front-engined cars
  were caught (asserted by `a_power_slide_can_be_caught` until the rear-wheel-drive cars
  went, 2026-11-26).
- Tire compounds (`tire::COMPOUNDS`, agreed with the user; touring added later: least
  grip, best at 55 °C, lasts longest, rolls easiest): street (grips from cold,
  best at 65 °C, lets go gently past the peak, overheats when driven hard), sport, race
  (needs heat, best at 95 °C, peaks early and drops off, wears fast). Each has grip,
  peak slip angle/ratio with the Magic Formula C (stiffness B follows from the peak), a
  temperature window and a tread life (sliding energy). Cars start on cold tires; parts
  shops fit fresh ones. A car's tire state survives an engine swap. Tests about other things
  call `Car::warm_tires`.
- Ribs, heat and pressure (`tire.rs`, asked for by the user: grip from the surface, a
  spin must not cook the tire, temperatures across the width, pressure, adjustable camber,
  no permanent overheating damage): each tire is 5 ribs (`RIBS`, inside to outside
  everywhere, also the HUD and 2). `Corner::step` samples the ground under each rib's
  lowest point (no raycast any more); `TireParams::patch` turns the depths into rib loads:
  the belts pass on 0.6 of a tilt, the sideways force rolls the carcass under (load to
  the outside shoulder of the outside tire), and pressure against what the load wants
  (`DESIGN_PRESSURE` 2.5 bar hot at the design load, × √load) loads the shoulders
  (under-inflated) or the middle (over). Each rib grips by its own load (load
  sensitivity punishes an uneven patch: that's what makes camber and pressure have an
  optimum), surface temperature and wear; the forces are the ribs' grip summed on one
  Magic Formula curve. Camber thrust as an equivalent slip angle (0.06 × lean). Heat:
  surface per rib (small; sliding heat, the road's share growing with √slide speed; cools
  into the core, the air, and the road only while sliding over it), core per rib (rolling
  resistance's flex power × (load + grip force), 0.7 of it; rubber conducts poorly, so it
  warms by flexing more than from the surface), carcass and air (0.3 of the flex, from the
  cores; sets the pressure by the gas law from `inflation`, the cold pressure). Pressure
  also sets the vertical stiffness ((p + 0.6)/(2.5 + 0.6)), the peak slip angle and
  rolling resistance. A 3 s spin at 12 m/s flashes the surface over 100 °C and back
  within seconds, the core a few K; steady cornering keeps the outside tire's surface
  near its core, hottest on the outside shoulder; on a straight the surface cools below
  the core. Near the limit on a 40 m circle (`tire_temperatures`, ignored) the outside
  cores reach about 50 °C in 30 s and 80 °C in 2 min on street tires. `CarBuild` has
  `camber` and `pressure` per axle (factory: `Chassis::camber`, `Chassis::pressure`,
  checked against `suited_pressure` by `factory_pressures_suit_the_tires`); a suspension
  kit comes aligned to its extra camber and allows ± its reach (`SUSPENSIONS`). Set at alignment shops
  (`garage::alignment`, with toe and the brakes' force and balance, `CarBuild::toe`,
  `brake_force`, `brake_balance`; not sent to others; applied live by `Car::rebuilt`).
- Not tuned by driving yet: the heating rates, the windows (now surface temperatures),
  how much grip overheating costs, wear rates, how strongly camber and pressure matter.
  Possible next steps: rim width affecting the sidewall, brake heat into the rim and the tire's air, toe settings.

## Engine model

Plan agreed with the user: a mean-value engine model that runs live, with an ECU that
only sees sensors and only drives actuators, and maps baked offline per engine build.
Hardware changes should eventually cover almost everything (cams, rods, pistons, turbo
and compressor, NA runner lengths, header lengths, cat/silencer restrictions, cam
phasing, VTEC-style cam switching).

- **Baked per build (cylinder side; re-bake when the bottom end, head, cams, runners or
  headers change):** volumetric efficiency by rpm, intake pressure, exhaust/intake
  pressure ratio, cam profile (discrete slices), intake and exhaust phase (coarse axes,
  length 1 without phasers); residual gas fraction (same axes); MBT spark, indicated
  efficiency at MBT and λ=1, knock-limited spark at reference conditions, exhaust
  temperature, heat to coolant, all by rpm and load; friction by rpm (and oil
  temperature). Load is air per cycle relative to filling the displacement at ambient.
- **Universal curves:** torque vs. spark offset from MBT, efficiency vs. λ, exhaust
  temperature shift with spark retard and λ, knock-limit corrections (intake temp,
  coolant temp, octane).
- **Live components (no re-bake):** throttle (orifice flow), idle valve, filter,
  intercooler, cat, silencer, pipes (pressure drop ∝ ṁ²/ρ), compressor and turbine maps,
  wastegate, turbo shaft inertia, manifold volumes, mechanical limits (valve float,
  rod loads; done, see damage below).
- **Live states:** intake (and boost) pressure, exhaust manifold pressure, turbo speed,
  temperatures, knock/damage accumulators.
- The maps come from a cycle simulation in the game crate (`cycle.rs`), not in
  `tools/bake`: parts are swapped in the game, so any build may need baking. It is
  one cylinder, crank degree by crank degree: single-zone gas with fresh/burnt mixture,
  cam lift curves through orifice valves, Wiebe burn, Woschni heat transfer, Douaud-Eyzat
  knock integral (its constant calibrated so the stock NA engine is knock-limited a few
  degrees at low rpm), intake runner and header primary as acoustic delay lines (wave
  reflected inverted from the plenum/collector after 2L/a), which is what tunes them.
  About 1 ms per converged operating point; a whole build bakes in about 5 s on one core
  (`maps.rs` spreads it over threads, none on wasm). Friction is still a correlation.
  Calibration knobs are the constants at the top of `cycle.rs`.
- `EngineMaps::for_hardware` takes embedded maps when the fingerprint (cylinder-side
  hardware plus the bake code) matches, else bakes synchronously. `maps::Bake` bakes a
  batch of grid points at a time, for the garage.
- **ECUs:** each has its own idea of the engine (e.g. its own VE table for fuelling), so
  changed hardware needs a retune. The first ECU is speed-density (rpm, MAP, throttle
  position), with fuel (VE table × target λ), spark map, rev limiter and idle valve.
  Later ECUs differ in sensors and features: closed loop, cam switching and phasing,
  wastegate/boost, knock, launch control, anti-lag.
- Order: mean-value model with the simple ECU and placeholder maps; then turbo, thermal
  model, knock; then an ECU tuning UI; then the offline cycle sim and hardware parts.
- Done so far: `src/physics/engine/` (hardware, placeholder maps, simple ECU with
  overrun fuel cut and an intake temperature sensor, runtime model); the drivetrain
  calls it. Default build is tuned to the old torque curve (about 200 N·m at 4200 rpm).
  Coolant/oil lumps with thermostat, radiator and fan (`thermal.rs`); knock limit and
  damage (see below); turbo (`turbo.rs`: parametric compressor
  and turbine, spring wastegate, intercooler, diverter valve). The setup panel swaps
  between the NA and turbo builds (about 310 N·m on 0.7 bar) and sets the octane.
  No exhaust thermal mass yet: the exhaust temperature is that of the gas leaving the
  cylinders, and it drives the turbine directly.
- ECU tuning (`src/ecu_ui.rs`, a tab in the dyno's room): VE, target λ and spark grids with the
  operating point, block selection and +/-/set, "Correct VE here" (scales the cells
  around the operating point by measured/target λ), rev limiter, idle, factory tune.
  Tunes are saved by name as text (`src/tunes.rs`: `~/.config/car-sim/tunes/*.tune`,
  local storage on the web) with the build they were made for; loading reads over the
  engine's factory tune and warns if the build differs. Swapping the engine build in
  the setup panel replaces the tune with the new build's factory tune.
- Engine parts (fitted at car workshops and tuning shops, see "Owning cars and parts"):
  parts from the catalogue (`engine/parts.rs`: a
  slot per kind of part, each part a list of `Effect`s on the engine's hardware, relative
  to it; the built-in ones in `catalogue/parts.rs`; a factory version is a block and a
  `Preset`); cam-gear timing and wastegate spring are
  set in the dyno's room (`dyno::cams`). The car's `EngineSetup.parts` says
  what it's built from. A car built anew bakes the maps if the fingerprint changed and the
  server hasn't got them (native: a thread; wasm: 12 ms slices per frame), and caches
  them (`storage.rs` folders: `~/.cache/car-sim/engines/<fingerprint>.maps`, local storage
  on the web). A new engine is new (no damage); the tune stays (the ECU is the car's:
  retune at a dyno). The redline follows from the hardware (`safe_rpm`: 95 %
  of the valve float speed, rods' pull under 70 % of their strength); the factory tune's
  rev limit (first cam profile's with the stock ECU) and the rev counter follow it.
- Chassis dyno (dyno shops, `src/dyno/`; it replaced the engine dyno window (5),
  removed 2026-11-22, in git history before that): see "Shops". The curves are
  piecewise linear between the maps' 500 rpm grid points.
- Damage (`damage.rs`, agreed: interference engine, engine and turbo only for now):
  wear 0..1 per part. Per cylinder (with a fixed spread so they fail one by one):
  pistons (knock, divided by `piston_toughness`; holed = dead cylinder), valves (float
  past 6 %: they meet the pistons; bent = dead cylinder), rods (fatigue above 75 % of
  their strength, `load^40` per cycle; broken = wrecked). Engine-wide: head gasket
  (coolant overheating, and peak pressure with knock spikes over what it seals; blown =
  less efficiency, coolant heated), bearings (oil overheating; spun = wrecked), turbo
  (over its compressor tip speed of 520 m/s, bursting 12 % past it; turbine inlet over
  1150 °C; failed = stops, no boost). Rod loads: pull m·r·ω²·(1+r/l) at TDC, push from
  the baked peak pressure (raised by spark advance, saturating at +35 %; knock spikes
  only for the gasket) against 2.5× the strength in buckling; peak pressure averaged
  over 0.1 s so a step's transient doesn't count. Valve float speed from spring force at
  full lift, moving mass, lift and duration (`ValveTrain::float_rpm`); floating loses VE.
  The factory tune is knock-limited at boost and stays safe; greedy spark on race fuel
  with big boost breaks stock rods. Repairs are new parts (a whole engine, a turbo; a
  long block's insides at the engine shop, to come). Factory tunes richen where the turbine inlet would come within `TURBINE_MARGIN` (40 K)
  of its limit (the X25T sat right on it and lost its turbo in ~10 min of hard driving;
  2026-11-22). Exhaust temperatures run hot (no losses before the turbine),
  hence the high turbine limit.
- ECUs are switchable (agreed with the user): `EcuModel` (Stock, Programmable) is chosen
  in the garage like a part; the stock one stays. One `Ecu` struct for all models, the
  model deciding which features it uses (cam control lives in every `Ecu` but only the
  programmable one drives it, and only it writes the cam settings and maps to tunes).
  Tunes record `ecu <model>`; loading keeps the fitted model and warns. Programmable:
  cam switch-over (rpm and MAP, hysteresis) and intake/exhaust phase maps; its factory
  tune switches where the second profile breathes better all the way to the redline,
  and phases for the most air from 45 to 75 kPa up (at rest below, for the idle).
  Parts: a switching cam (236°/264°) and phasers (intake 40°, exhaust 30°); with the
  stock ECU they stay on the first profile, at rest. Phase axes have 3 points and the
  ratio axis is coarser with phasers (bake: intake phaser about 2 s, both 6 s, with the
  switching cam 13 s on 4 cores).
- Next ideas: closed loop (wideband sensor), knock sensor/control, boost control,
  launch/anti-lag; damage to the clutch and gearbox; misfire sound for dead cylinders.

## Map editing (agreed with the user, 2026-11-18)

The map is ours: `map/gerach.osm` (imported from OSM once, edited by the user in JOSM,
committed; `upload='never'`) is the only input of the bake besides the lidar. `bake
import` adds new areas without touching what's there (deleted elements stay in the file
as `action='delete'`). Tags we add go into the map file and `KEEP` in
`tools/bake/src/osm.rs`. The plan, in this order:

1. Map file (done).
2. Surfaces (done): `physics/surface.rs`, grip share and rolling resistance factor per
   `Surface`; road kinds from OSM `surface`, untagged tracks by `tracktype` (the user
   checked: OSM's tags are right, keep the rules); else by land use. `Terrain::flat` is
   paved, so driving tests that would leave a road use it (`flat_road` has one road).
3. Roads as their own geometry (done): bake `roads.rs` (spline centrelines 1 m apart,
   crown, terrain cut and filled, carved under bridge decks) and `profile.rs` (network
   least squares, block SOR over chains of roads running on into each other). Game:
   `Terrain::ground(x, z, y)` blends overlapping roads (weights fade at road ends;
   roads sharing a point, `Road::meets`, also out to `JOIN_REACH` (2 m) past their edges,
   each weighted by its margin + 2 m times how near both edges are, since 2026-11-23: at
   a Y the edges parted up to 35 cm apart in height, a step and a hole the user found at
   (−2644.4, −2216.4); the bake's profile is too stiff for ties at the tips to fix it,
   tried) and
   has levels (a deck carries what's within a metre below its surface); `sample` is the
   level beneath any bridge, `land` the bare terrain. Bridges are drawn as slab and
   parapets; parapets are obstacles. Width changes (asked for by the user, 2026-11-22;
   format 15): where exactly two roads on the ground meet end to end (no road passing
   through the node) and differ by 0.25 m or more, the narrower one widens to the wider
   over 8 m per metre of difference (6–30 m, at most half the road) with a smoothstep
   (`roads::taper`, `Road::ends`, the game's `Road::width_at`; 123 ends). Where two road
   ends meet at an angle, both ribbons end on the mitre (`join_ends`), so no wedge opens
   on the outside of the bend. Curbed corners (asked for by the user, 2026-11-22; format
   16; `tools/bake/src/corners.rs`): at nodes where 3+ roads on the ground leave (none
   with a bridge), between each two neighbouring ones 25–160° apart, the inside of the
   turn is filled to an arc of radius by the narrower road (2.5 m under 3.5 m wide, 5 m
   under 6 m, else 8 m; ×0.6, ×0.35 if it doesn't fit in 0.8 of the arms or its arc
   reaches into a house). Since 2026-11-23 (the user: corners didn't meet curved roads
   or odd angles, gaps) built on the roads' edges as drawn (`Arm::stations`, across as
   the game's `Strip::side`): the arc's centre is where both edges moved out by the
   radius first cross, its ends where it touches them; the outline runs from the node
   out along one centreline, across to the arc, round it and back along the other
   centreline, so all but the visible part is under the roads (the cut adds no outline
   segment lying under a road, `Terrain::under_road`: corners either side of a road
   would overlap along its centreline, which spade asserts on). Heights: each arm's
   surface at the nearest point, weighted 1/(distance beyond its edge + 0.05)². The
   game's `Corner`s are
   in the road grid (entries past the roads), count as ground where no road covers, and
   are part of the ground mesh since 2026-11-23 (see below). **The ground is cut out under the roads** (asked
   for by the user, 2026-11-22: no seeing under a road's edge, no terrain through it;
   `src/terrain/cut.rs`, `RoadRibbons::ground_mesh`): per chunk, the grid cells a
   ribbon's or corner's outline crosses go into a constrained Delaunay triangulation
   (crate `spade`) with their corners, sides and diagonals (so the rest stays the grid's
   triangles) and the outlines (`add_constraint_and_split`); pieces whose centroid is
   inside a ribbon segment are dropped, those inside a corner fan are the corner (road
   colour); a point on a ribbon outline gets the drawn ribbon's height, else on a
   corner outline the corner's, else inside a corner its surface, else the grid's.
   Corners were separate fans before, 4 mm under the ribbons: with nothing beneath
   them, a corner lower than the blended ribbon over it opened a gap to see under the
   map (the user found one at (−2646, −2217)). Corner surfaces follow the ribbons:
   `Terrain::settle_corners` divides each spoke (first point to an outline point)
   ~1.5 m apart at load, heights on a road = the roads' blend (`ground_and_roads`), off
   the roads every near road's drawn surface just inside its nearest edge, weighted
   1/(distance + 0.05)² (no baked heights: those made it lean towards the terrain;
   threaded natively; physics uses the same `Corner::at`). `rays_never_pass_under_the_roads` casts low rays over junction
   blocks (`rays_at_many_junctions`, ignored, every hundredth junction); slits under
   1 cm don't count (where two overlapping roads' edges cross at heights a few mm
   apart, the ground can meet only one). Not cut:
   bridges, waterways. The ribbon segment is shared
   code (`Strip`), so outline and ribbon agree exactly; its last row is exactly the next
   segment's first. ~0.8 ms a chunk. Spade asserts on degenerate input: an outline
   added twice once split (so outlines are deduplicated, and a road's end row isn't one
   where it runs on into another road's end); `every_chunk_is_cut` cuts the whole map
   (the web build aborts on a panic; natively a failed chunk stays uncut). Tests also
   check the ground is closed up to the outlines and nothing is uncovered from above
   (sample points within ~1 mm: the mesh is f32, a quarter millimetre apart at x 3000).
   The physics' `land` still is the grid's triangles, so right beside a road the drawn
   ground differs from it by the edge's mismatch. The bake's `conform` (`Near`): under
   a road up to `SINK` below the lowest road over it (out of sight), within the 1 m
   shoulder level with the edge, blended between several roads by inverse distance⁴
   (so an island between roads at different heights slopes instead of a wall at one
   edge), beyond that `EMBANKMENT` from the nearest. A 2 m shoulder was tried and
   dropped (terrain rose under bridge ends). The bake is deterministic (BTreeMaps where
   it iterates; the same map bakes to the same bytes). Overlapping roads are one
   surface in the profile solve (`tie_overlaps`, 2026-11-23): at five places across
   each road point lying ≥ 0.25 m inside another road on the ground, connected or not,
   a tie (`Problem::ties`: weighted terms with a target, solved like the bends) pulls
   the two surfaces (crowns included) together; road ribbon triangles steeper than 26°
   went from 352 m² to 44 m² (waterways on steep banks aside).
   **No two ribbons on top of each other** (asked for by the user, 2026-11-23: a cobbled
   lane showed through Grubenweg at (−2945.5, −1939.2)): `RoadRibbons::mesh` clips each
   ribbon triangle against the better roads' ribbons over it (`Strip::covered`,
   `outranks`: kind as `RoadKind::ALL`, then width, then index; bridges and waterways
   neither), leaving `SEAM_OVERLAP` (3 cm) under their edge against slits. The cut
   needs no change (the ribbons' union is the same); the physics' surface is the best
   covering road's. `join_ends` also mitres where more than two ends meet (the
   straightest pair of equal width; unequal widths on a mitre leave the cut open edges),
   so a road split at a side road's node has no wedge. ~5 % more meshing time.
   `where_roads_overlap_only_the_best_is_drawn` checks three chunks by colour.
   Not done: junction middles as one mesh (the overlapping ribbons' facets show faint
   shading seams), tunnels, piers.
4. Road condition (done): `relief.rs`, OSM `smoothness` to ISO 8608 roughness (waves
   0.5–20 m), features from `traffic_calming=bump|hump|table` and `hazard=pothole`
   nodes. The baked file is zlib-compressed, heights predicted from neighbours.
5. Roads off buildings (done): bake `clearance.rs` moves roads sideways (≤ 2 m, tapered,
   shared points pinned) and writes what's left to `map/gerach.conflicts.gpx`
   (gitignored), for JOSM. 303 overlaps → 106: 50 near junctions, 46 between houses
   narrower than the road (wants a `width` tag), a few needing more than 2 m.
6. Fences (done; the user dropped trenches): `barrier` ways (imported 2026-11-18:
   184 guard rails, 35 walls, 32 fences, 15 hedges, 14 railings) baked by
   `tools/bake/src/fences.rs` (gates cut as gaps, barriers along roads pushed 0.5 m
   off the edge, those along bridges dropped, the rest on roads listed in the conflicts
   file) into format v12; the game's `src/fences.rs` draws six kinds in pieces ≤ 2.5 m
   on `Terrain::sample` and puts a solid box per piece in the obstacle grid. Agreed:
   solid (not breakable), gates open, only what's in the map (no generated rails).
7. Map editor (agreed 2026-11-18, built the night after): see "Map editor" below.

### Map editor (step 7, agreed with the user)

A third app, `src/bin/editor.rs` (`cargo run --bin editor`), native only, its own
plugin list on `CorePlugin`'s shared pieces but **no cars** (no simulation, traffic,
HUD, sound). Decisions:

- **Edits go straight into `map/gerach.osm`**: the editor reads and writes it itself
  (the bake's `osm.rs` model and JOSM-style writer, shared), with its own undo/redo and
  Save. No JOSM connection at all. Written the way JOSM writes: new elements get
  negative ids, changed ones `action='modify'`, deleted ones stay as
  `action='delete'`, so JOSM can still open the file and `bake import` still merges.
- **Camera**: fly. Hold RMB to look, WASD to move, Q/E down/up, Shift faster, mouse wheel
  sets the speed. Left click selects and edits. Teleport via the 2D map (M, click).
- **Edits, all of them**: tags on ways (roads: smoothness, width, surface, highway,
  bridge…; fences: barrier, fence_type, height; buildings: levels; areas: kind), point
  features (bumps, humps, tables, potholes, gates: place, move, delete), geometry (move,
  add, delete nodes; draw new roads, fences, buildings, areas; split and delete ways),
  road heights (`ele` on a node pins the road's height: a hard constraint in the bake's
  profile solve; raised and lowered in 3D), land-use areas (draw and edit closed ways for
  paved areas, forest, fields, meadows, water…).
- **Terrain areas** (closed ways with our own tags, applied by the bake to the lidar
  before roads are fitted): flatten to a height, raise/lower by an amount (faded at the
  edge), smooth, and slope (a plane through heights set at its corners).
- **Re-bake on a key/button**: save, run the full bake (~20 s, progress shown) and
  reload the map in place.
- **The driving apps load `assets/gerach.terrain` at startup** on native (embedded copy
  as fallback and for the web), so a bake shows up without recompiling.
- Inspector: what's under the cursor (way id, tags), and the conflict markers from the
  bake shown in 3D with next/previous.

Built (all in `src/editor/`, see its module docs and README "The map editor"; nothing
of it tried with a real mouse yet, only headless): `doc.rs` (the `Osm` with node
positions, ways per node, `Change`-based undo via `Doc::edit`), `index.rs` (`Class`
of a way by tags, `PointKind`, 64 m grid rebuilt per revision), `overlay.rs` (gizmo
lines draped 0.3 m above `Terrain::sample`, cached per revision; hover picks the
nearest node within 12 px × 1.3, else segment, in ground metres), `tools.rs` (`Tool`,
`Command` shared by keys and panel), `panel.rs` (an `egui::Area` top right: a
`Window` kept its old size and scrolled), `bake.rs` (`sh -c`: `cargo build --release`
in `tools/bake` without `CARGO_TARGET_DIR` (the dev shell's `cargo run` adds
`--profile`), then the binary; on success `Terrain::parse` + `scenery::Reload` +
`MapView::rebuild`), `conflicts.rs`. Decisions made alone: tags `carsim:terrain`
(flatten|raise|smooth|slope), `carsim:height` (m above sea level), `carsim:offset`,
`carsim:radius`, `carsim:fade` (default 5 m, inside the outline); slope corners and
pins are `ele` (above sea level; baked format 13 on stores `Terrain::base`, the lowest
point's height, to convert). egui input is **not** absorbed in the editor (bevy_egui
then resets the mouse buttons whenever egui is over the pointer, dropping drags):
systems check `EguiWantsInput` themselves. The bake is shared code-wise: `src/osm.rs`
is `#[path]`-included by `tools/bake` (std + quick-xml only). Map file ids: new
elements without `version` (JOSM rejects one on new elements).

## Graphics (agreed with the user, 2026-11-24)

`src/graphics.rs` (`GraphicsPlugin` in `CorePlugin`; the editors don't have it and
draw at the nicest): `Graphics` (`graphics.cfg`) with shadows (off; low: 1 cascade,
1024², 60 m; high: 4 cascades (WebGL 2 has only the first), 2048², 120 m), antialiasing
(off, FXAA, MSAA 4×), view distance (near: fog 120–320 m; far: 180–480 m; `scenery`
loads chunks to the fog's end + 80 m and drops them past + 180 m), render resolution
(50/75/100 %), post-processing and the time of day, and natively vsync and a frame rate limit (a sleep in `Last`). Defaults:
web low/FXAA/near/75 % on a display with a scale factor ≥ 1.5 (else 100; settled once
the window is known), native high/MSAA/far/100 %. No presets (the user's wish). All
apply live.
- Render resolution: the 3D camera (`ChaseCamera`) always draws into an image (`View`,
  window size × resolution; `ImageRenderTarget::scale_factor` = window scale × resolution,
  so `world_to_viewport` still gives window points), and egui sits on a `Camera2d` of
  its own (order 1; `EguiGlobalSettings::auto_create_primary_context` off) that paints
  the image over the window at the start of every egui pass (`Backdrop`, an
  `egui::Plugin`: bevy_egui runs the pass in `PostUpdate`, so painting from `Update`
  would be lost, and shapes in the background layer draw in the order added). At 100 %
  the shot is pixel for pixel the same as rendering to the window. Not tried in a
  browser yet.
- Post-processing (asked for by the user, 2026-11-24): `src/grade.rs` (`GradePlugin`,
  added by `GraphicsPlugin`), a colour grade as bevy's `FullscreenMaterial` (one
  full-screen pass, `src/shaders/grade.wesl`, embedded as
  `embedded://car_sim/shaders/grade.wesl`): exposure/tint, the sky (bright pixels high
  on screen) held down, saturation, black crush, S-curve, split toning (teal shadows,
  warm highlights), shadow lift, vignette, grain, colour fringes at the corners. The day
  is graded like a modern Bond film (asked for by the user, 2026-11-24). Settings
  `post on|off` (default on natively, off on the web), `time day|dusk|night|cycle`
  (`TimeOfDay`, default cycle) and `day_length` (minutes, `DAY_LENGTHS`, default 24).
  A `grade::Look` per time of day (DAWN, DAY, DUSK, NIGHT) is the grade plus the sun's
  colour/strength and the ambient light's (shares of `grade::SUN`, `AMBIENT`); the cycle
  (`grade::Clock`, 0 sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight, starting at 0.12)
  blends them; a look also tints the sky dome (`sky::SkyMaterial`), the fog and the
  clear colour (`Look::sky`). Night is made by the light (sun 7 %, dim blue ambient,
  dark blue sky and fog); its grade only cools it, so lamps keep their colour. With
  post-processing the camera is HDR with `Bloom` (0.04 by day + 0.2 × darkness) and the
  grade runs after tonemapping, before FXAA (`schedule_configs`). "Day and night"
  (`day_night on|off`, asked for by the user, 2026-11-24) off: always day, still graded.
  With post-processing off the lights are the plain day's, LDR, no bloom; in a shop's
  scene (`car::Away`) it's always day. The clock is local, not shared online. Uniform
  fields are `Vec4`s (WebGL 2 padding). The sun's direction doesn't move.
- Night lights (`src/night.rs`, 2026-11-24; local and cosmetic, asked for by the user:
  nothing sent, every car's lamps on at night): `grade::Darkness` (0 day … 1 night, from
  the sun's share) scales the emissive of `night::Glow` (head and tail lens materials,
  shared) and `scenery::WindowGlow`. `Body::lamps` makes per-chassis lens meshes a
  little proud of the body's lamps (head lamps: `Tone::Lamp` in the front half; the
  tail's `Lamp` is the number plate), drawn as children of each car's body (not on
  ghosts). Houses have windows (`buildings.rs`: 1 × 1.2 m every 3.2 m per storey, 3 cm
  off the wall, unlit ones dark glass in the objects mesh, 40 % by hash in a chunk mesh
  of their own with the glowing material). Our car (`car::Car`) has two `SpotLight`s
  at its head lamps (no shadows), hidden by day.

## Sky

`src/sky.rs`: a dome (900 m, inside the 1000 m far plane, following the camera) with an
unlit, unfogged texture made at startup (2048×512, lat-long of the upper half): gradient
from `ZENITH` to `HAZE` (the fog's colour too, so the land fades into the horizon), the
sun's glow and disc towards `SUN` (also where the directional light sits), and a flat
cloud layer of value-noise fbm seen in perspective (octaves finer than a texel dropped).
Knobs at the top. The sun is 59° up, so it's never in the chase view.

## Audio

The engine's layout (`families::Layout`) sets how many pulses there are in a cycle and
which bank's exhaust each goes down (`audio.rs`, `sound`): a cross-plane V8's banks fire
unevenly, a flat four's pulses bunch up through unequal headers, the rest fire evenly;
inline engines have one exhaust (no panning).

The exhaust hardware colours it (`ExhaustSound::of`, parts compared with the stock ones
for the engine's size): silencer and cat muffle (level, upper resonances; straight pipe
loud and rasping), bore (deeper, louder), header length (a quarter-wave resonance), a
turbine (smeared, quieter pulses). The turbo whistles: band-passed noise (Q 4) centred at 1 Hz per shaft rev/s,
level ∝ share of its limit², more under load. All constants at the top of `audio.rs`,
none tuned by ear yet. No diverter "pssh" or overrun pops (fuel cut leaves nothing to burn).

Other cars are heard too (asked for by the user, 2026-11-21; `hear_others`): any car
with `Looks` and `Motion` that isn't a `Car` (remote cars), the nearest 6
within 150 m (heard from 135 m, so one at the edge doesn't flicker), each an
`AudioPlayer` of its own on a `Voice` entity (fading out over 0.2 s when dropped). The
same synth, placed by `Targets::place`: mixed to one source, panned from the camera
(constant power), as loud as our own at 6 m and falling with 1/distance (faded out over
the last fifth), a low-pass from 10 kHz at 6 m falling with √distance, and Doppler (the
whole engine's time scaled by 343/(343 − closing speed), the closing speed from the
distance's change, smoothed 0.1 s, capped at 100 m/s). Runs in PostUpdate after
transform propagation (the shot cameras move in PostUpdate). The engine comes from the
car's latest state, network delay and all, never predicted (agreed with the user: real
sound is late too). Not done: propagation delay, occlusion, the cost of 6 voices on wasm
untried (a slow page could lower `VOICES`).

Sound effects (asked for by the user, 2026-11-22; `audio/effects.rs`, a second synth
next to the engine's, with the same sound setting): **tire squeal** of the driven car per wheel (band
of noise at ~780 Hz with two overtones, wobble and flutter, pitch rising past the peak),
from the combined slip relative to the peak (from 0.55, full at 1.4), the tread's sliding
speed, the load, asphalt (cobbles half, unpaved none); **gear grinding** while an
H-shifter's lever is in a gate whose gear can't go in (`Drivetrain::grinding`: clutch
not pressed, reverse while rolling) and 0.3 s on a refused reverse by key (clicks at the
dog teeth's clash rate from the speed difference ringing metallic resonances); the
**trumpet** of a race's countdown (`racing::countdown`: G4 at 3, 2, 1, a C major chord
at the green; `Effects::call`); **crashes** from `Car::knocks` (the body's speed change
from houses, trees, fences, cars and, since this, the ground; taken every frame by
`effects::cue`): 1 m/s in a frame is a crash, 15 the hardest (a gliding thud, a crunch of
crackles ringing panel resonances, glass pings from 0.45), knocks kept up while moving
scrape. the **starter** while cranking (2026-11-22: the motor geared 12:1 to the crank;
partials at the pinion's 11 teeth and twice that, the commutator's 22 segments and the
motor's hum, brush noise; each compression dips its speed and swells it; the solenoid
clunks in and the pinion clacks out; let go or overrun, it spins down over ~0.5 s).
`write_effects_wav` (ignored, `EFFECTS_WAV=path`) renders
them all to listen to. Not done: other cars' squeal and crashes (`Motion` has no slip), tuned by ear.

Listening to the engines offline (2026-11-24, for reworking the engine sound):
`ENGINE_WAV=dir ENGINES=M50,B16 cargo test --profile release-dev write_engine_wavs --
--ignored --nocapture` renders each factory engine (all without `ENGINES`) to
`dir/<code>.wav` (23 s, ~2 s for all 22): idle, three blips, a full-throttle pull from
2000 rpm to the redline, lifting off, a cruise, then a pass-by at 90 km/h from 8 m
(the remote-car path: mono, low-pass, pan, Doppler). Raw levels, the same program
every time, so renders before and after a change can be compared. First render: nothing
clips, and the V8s (RMS 0.155) are quieter than the 1-litre three (0.19): nothing in
the level grows with displacement. Since then (with the user listening): the pulse
lasts a crank angle (20° up, 70° down), each cylinder has a fixed strength and lateness
(`CYLINDERS`: the lope), and the pulse runs through three pipes as delay lines (`Tube`:
header, front pipe, tailpipe; inverted reflections from the muffling, losses above
`PIPE_LOSS`) before the old broad resonances, so harmonics swell and fade 4–7 dB over a
pull (was ~2). Upper harmonics: the pulse's front sharpens with load (20° to 4°, starting
with a kink), and what's above 1 kHz bypasses the broad resonances (`BRIGHT`, set by the
muffling; the stock exhaust counts as ~0.89 muffled): the V8 falls ~4–6 dB per octave of
orders at 5000 rpm (was ~10). Tried and dropped: steepening in the header as a
pressure-dependent delay (skipped samples lost the pulse's peak: −2–3 dB in the pull). Tire squeal at half its level, swelling over 0.25 s (scuff first, the tone over it).
**Inside and outside** (`audio/paths.rs`): outside, the road's reflection of the exhaust
(`ground_delay` from the car's and the camera's places: ~0.6 ms behind our car, sweeping as
a car passes), scaled so the lows keep their level; from the driver's view (`ChaseCamera::
driver`) every car sound but the race's trumpet goes through `Cabin` (body transmission
two-pole at 450 Hz plus a 15 % leak, three cabin modes 72/118/155 Hz, four early
reflections per ear, narrow stereo), crossfading over 0.3 s on an S-curve (our road reflection keeps its delay inside: dropping it clicked; the render writes `<code>-switch.wav`). Each car its own cabin
(`CabinShape::of`, asked for by the user, 2026-11-24): modes from the cabin outline's run,
the body's width and the roof over the floor; how much comes through and how bright from
`Chassis::insulation` (a constant per body, 0 stripped or open to 1 the saloon) and the
engine's distance from the eyes (the mid-engined cars and the V12 supercar brightest);
open cars (cabin outline of two points or none) get a third of the cabin. One
`paths::Listener` (inside or not, the cabin) shared by every synth (`Listening`
resource). Synthetic (no recorded IR).
**Loudness from the gas** (branch `engine-gas-flow`, protocol 17, tried by the user before
merging, 2026-11-24): each pulse's strength is the gas a firing pushes out, displacement
per cylinder (`EngineCharacter::displacement`, in `Looks`) × how full it got
(`Drivetrain::fill`, air per cycle over the displacement, above 1 on boost; sent as
`EngineRun::fill` in `load`'s old slot, so no extra bytes per snapshot; the 0–1 tone load
is `drivetrain::firing_load(fill)` on the receiver; traffic and the showroom send
`fill_for(load)`). The factor making fewer cylinders louder is gone; `CYLINDER` (0.5 L at
full fill = strength 1) leaves the big engines room under the limiter. A-weighted at full
throttle: three −27.5 dB, 1.6 four −24.2, V8 −22.6/−21.3, V10/V12 −20 (the V12 was 2.4 dB
under the 1.6 before); the whole game ~4 dB quieter. The render writes `<code>-inside.wav`
too. Idle is ~4 dB louder inside (the boom), a V12's pull 5 dB quieter (its energy is
where the body blocks). Measured from the renders with numpy (order levels over the pull, envelope
modulation at half orders); the user judges by ear. Not done: displacement in the level
(needs it in `EngineCharacter`: protocol bump), the road reflection outside, the cabin
inside, the whistle (now ~34 dB under the exhaust at 4000 rpm) never tuned.

The engine sound goes through bevy_audio, which can't set the output buffer size. An
attempt to open the output with rodio/cpal directly (fixed 4096-frame buffer against
underruns) gave no sound at all in the wasm build and was dropped. If it's tried
again, keep the bevy_audio path on wasm.

Web: the browser creates the `AudioContext` suspended unless the page already had a
click or key press, and cpal resumes it only once, at startup, so the game was silent
whenever loading finished before the first input (found 2026-11-19). `web/index.html`
resumes every context on the first pointer/key/touch event. Checked in headless
Chromium (from the pinned nixpkgs, `--use-angle=swiftshader --enable-unsafe-swiftshader`)
with a script that hooks `AudioContext` and `AudioBuffer.copyToChannel` (cpal copies
every buffer through it) and reports the state and the RMS to the server, and clicks
sent through the DevTools protocol (`Input.dispatchMouseEvent`).
