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
- Keep the notes up to date when something worth knowing next session comes up: this
  file for what applies everywhere, the topic's `CLAUDE.md` next to its code (or its
  skill) for the rest (see "Where the rest of the notes are"). Notes refer to sections
  by name (e.g. see "Helicopters"); that list says where each is.

## Cross-cutting rules (always apply)

- Any new state field in the physics goes into its module's `snapshot!` list, or replays
  and the server drift.
- Bump `PROTOCOL` (`protocol.rs`) whenever a message's content changes; a physics change
  needs client and server updated together.
- Any text whose size follows the camera goes through `menu::scene_text` (egui's font
  atlas otherwise flickers).
- After changing `cycle.rs` or `maps.rs`, re-bake the stock engine maps (see "Sandbox
  workflow"); after changing the icon's drawing (`src/icon.rs`), re-run the ignored
  `write_icon` test.
- Names of cars, shops, places in events: made up, never a real car, business or brand.
- Before committing after screenshots: `bash tools/dev/strip.sh`, and `grep -cw shot
  src/lib.rs src/main.rs src/bin/*.rs` must print 0 for each.
- Never `pkill -f` anything matching your own command line, and never wait with
  `until ! pgrep -f …` (the pattern matches the loop itself): use `pgrep -x` or a PID.

## Where the rest of the notes are

Topic notes live next to the code and load when working there; update them there.

- `src/menu/CLAUDE.md`: Menus
- `src/garage/CLAUDE.md`: Shops
- `src/events/CLAUDE.md`: Events
- `src/heli/CLAUDE.md`: Helicopters
- `src/net/CLAUDE.md`: Server authority, Multiplayer
- `src/catalogue/CLAUDE.md`: Server catalogue, Owning cars and parts, Exhaust manifolds
- `src/input/CLAUDE.md`: Wheel support
- `src/physics/CLAUDE.md`: Cars, Handling, Engine model
- `src/editor/CLAUDE.md`: Map editing
- `src/audio/CLAUDE.md`: Audio
- Skills (`.claude/skills/`, loaded on use): `screenshots` (shot.sh, formerly "Screenshots" under the sandbox workflow), `fake-wheel` (the virtual wheel), `windows-build`.
- Plans: `docs/cars-and-parts.md`, `docs/exhaust-manifolds.md`, `docs/pushrod-v10.md`, `docs/vireo-lineup.md`.

## Apps (agreed with the user)

The game is a library (`src/lib.rs`) and apps, each the list of plugins it adds: the
**sandbox** (`src/main.rs`: the shared `CorePlugin` (`WorldPlugin` (terrain, obstacles,
simulation, traffic: all the server has too), scenery, input, driving, visuals, camera,
HUD, map, sound, egui), help (quick start, hints), the car the server gives (its catalogue's start: the Vireo hot hatch, B16, the 8-valve 1.6),
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
- Engine maps of every factory engine of the built-in catalogue (B16, B20 and B16R since
  2026-11-27 (the B18 before), R80, see `catalogue/engines.rs`), plus two engines tests measure against (W20,
  `EngineHardware::default`; B16T, `EngineHardware::turbo`), are baked into
  `src/physics/engine/baked/<code>.maps` (listed in `engine/store.rs`'s `BAKED`). After changing `cycle.rs`, `maps.rs` (outside
  their tests; any edit counts, their text is in the fingerprint) or a factory engine's
  cylinder side, re-bake all of them (a few seconds) or
  `stock_maps_are_current` and the tests that expect baked maps fail:
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo test --profile release-dev bake_stock_maps -- --ignored'`.
  A new factory version needs an (empty) file there first, for `include_str!`, and a
  line in `BAKED`.

## Career (removed 2026-11-19, to come back on top of multiplayer)

The first career (`src/career/`, `src/bin/career.rs`) is in git history up to c34bfbf.
Its agreed design, to revisit for multiplayer: start with €500 and mom's car (always full
and repaired); home with a garage that is its own 3D scene; gas station; pizzeria
(deliveries against the clock); car dealer (buy cars, deliver cars, trust unlocking
nicer cars and pay); parts shop (parts wait in the garage to be fitted, never touching
the ECU); a chassis dyno in its own scene (through the gears to fourth, then a full
pull; wheel and crank power, runs saved with names and notes); towing home, or mom
fetching you; 3 own bays, one save slot.

## Graphics (agreed with the user, 2026-11-24)

`src/graphics.rs` (`GraphicsPlugin` in `CorePlugin`; the editors don't have it and
draw at the nicest): `Graphics` (`graphics.cfg`) with shadows (off; low: 1 cascade,
1024², 60 m; high: 4 cascades (WebGL 2 has only the first), 2048², 120 m), antialiasing
(off, FXAA, MSAA 4×), view distance (near: fog 120–320 m; far: 180–480 m; `scenery`
loads chunks to the fog's end + 80 m and drops them past + 180 m), render resolution
(50/75/100 %), post-processing and the time of day, and natively vsync and a frame rate limit (a sleep in `Last`). Vsync, the limit and "Show frame rate" (`fps on|off`, off by default; fps and ms averaged over half a second (renewed that often, readable without vsync) top right, orange under 50, red under 30; the gizmos' position goes under it) are on the menu's Display screen but kept in `Graphics` (2026-11-27, the user's wish). Defaults:
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

## Tire smoke and dirt (asked for by the user, 2026-11-27)

`src/spray.rs` (`SprayPlugin` in `CorePlugin`), local and cosmetic, nothing sent: every
car drawn within `RANGE` (150 m) of the camera, ours, other players' and traffic alike,
from its `Looks`, `Motion` and drawn `Transform` alone. A wheel touching the ground
(`Terrain::ground` within `TOUCH`) slides at its speed there (frame to frame) less what
it rolls (spin change × radius), smoothed over `SLIP_LAG` (remote cars' drawn places
jitter); jumps over `JUMP` m/s count as placed. Asphalt and cobbles smoke from sliding
only (`SMOKE_SLIDE` 3–12 m/s); off the road rolling (`ROLL_SPEED`) and sliding
(`DIRT_SLIDE`) throw up puffs (dust, mist) and chunks (mud, turf, stones, drops) per
surface (`rates`, `particle`). One blended unlit mesh of camera-facing quads rebuilt
back to front every frame (`PostUpdate` in `PlaceTags`), a generated texture (soft puff,
round blob), dimmed by `grade::Darkness`. Graphics setting "Tire smoke and dirt"
(`Graphics::particles`, `particles off|low|high`; web low: 600 at 0.4 the rate, native
high: 2000). Shots: a burnout is the Merrow with traction control off in the shot's
config (`settings/aids.cfg`: `traction_control false`), `TRIM=1,0 STEP=0.02 OFFLINE=1
TAPS="KeyE@20"`, keys `w`, 120 frames (`CARCAM="-7,1.8,3"` from the side); the Vireo
floored from the spawn without `STEP` runs onto a meadow by frame 150. Not done: tuned
by playing, soft particles (puffs cut hard into the ground), dirt left on the cars,
tire marks.

## Sky

`src/sky.rs`: a dome (900 m, inside the 1000 m far plane, following the camera) with an
unlit, unfogged texture made at startup (2048×512, lat-long of the upper half): gradient
from `ZENITH` to `HAZE` (the fog's colour too, so the land fades into the horizon), the
sun's glow and disc towards `SUN` (also where the directional light sits), and a flat
cloud layer of value-noise fbm seen in perspective (octaves finer than a texel dropped).
Knobs at the top. The sun is 59° up, so it's never in the chase view.
