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
HUD, map, sound, egui), help (controls screen, hints), stock coupé, R reset,
gizmos, 2 panel, 3–6 tools, F11 fullscreen in all apps). The career was removed for the multiplayer work (see
"Multiplayer"). Keep it that way: no mode flags; something only one app has is a plugin
of its own (e.g. `car::ResetPlugin`, `ui::PanelPlugin`), and
shared pieces are public functions (e.g. `workshop`, `garage::start`). An app,
the map editor (`src/bin/editor.rs`, native only), has no `CorePlugin` (no cars): its
`editor::EditorPlugin` adds the terrain, scenery, sky and map plugins itself.

- Window keys are on the number row (moved from F1–F6 at the user's wish, 2026-11-19):
  1 gizmos, 2 panel/settings, 3–6 tool windows. The F keys belong to the browser on the web
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
controls screen (`egui::Modal`; a click anywhere or Esc closes it, H reopens it; the
start menu waits for it) lists the keyboard controls; the panel (2) and the tool window
bars are hidden by default. Hints over the road say what's wrong, what to press and what
that does: gas with the parking brake on, reverse asked for while rolling
(`car::ReverseRefused`, a message from `drive`), on the rev limiter below top gear, and
gas without moving for 3 s (R). Added alone: in neutral, stalled, wrecked engine, empty
tank. R (`ResetPlugin`) puts the car on the nearest road (`Terrain::drive_spot`: the
right-hand lane on roads with traffic lanes). The map teleport was removed at the user's
wish (2026-11-21). Keys: M map, 0 mute, B (held) looks back.

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
- Engine maps of every factory engine (22, see `families.rs`) are baked into
  `src/physics/engine/baked/<code>.maps`. After changing `cycle.rs`, `maps.rs` (outside
  their tests; any edit counts, their text is in the fingerprint) or a factory engine's
  cylinder side, re-bake all of them (about 40 s, 1.5 MB of text changes) or
  `stock_maps_are_current` and the tests that expect baked maps fail:
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo test --profile release-dev bake_stock_maps -- --ignored'`.
  A new factory version needs an (empty) file there first, for `include_str!`.

### Screenshots (`tools/dev`)

`tools/dev/unstrip.sh` copies `tools/dev/shot.rs` into `src/` and hooks it into
`lib.rs` and the apps; `tools/dev/shot.sh out.png [frames] [keys]` renders headless and
saves a screenshot (`BIN=editor` for the editor); `tools/dev/strip.sh` removes the hook.
Always run `strip.sh` and check that `grep -c shot src/lib.rs src/main.rs
src/bin/*.rs` prints 0 for each before committing. Keys are pressed from a third of
the frames until the shot (`m` opens the map, `0` mutes, `2` shows the panel (hidden by default
since 2026-11-20; older notes here used `2` to hide it), `3` the ECU window,
`4` the garage, `5` the dyno, `6` the showroom, `f` shows the gizmos (off by default), `c` the driver's view, `b` looks back; keys stay held through the shot's frame),
so clicks on a window must come after that (`T` presses the key T, `E` Enter).
The keys `3`–`6` didn't expand the tool windows in screenshots (as F3–F6) (seen 2026-11-17, also on
the code before the app split): click their title bars instead. (2026-11-18: neither
`CLICK` nor `UICLICK` on the "Garage (F4)" bar (now "Garage (4)") at (1190, 57) or its arrow expanded it.)
`PLACE="x,z,frame"`The editor (`BIN=editor`, hooked by `EditorShotPlugin`): `EDITCAM="x,z,height,yaw,pitch"`
(degrees, the first 20 frames), `CLICK="x,y,frame[,x2,y2]"` sets the window's cursor and
holds the left button (a drag with x2, y2), `UICLICK` is the window-event click for
egui (the map, buttons), `EDITKEYS="Digit2@25;ControlLeft+KeyS@60"` taps keys. A view
of a village road: `EDITCAM="-2986,-1954,45,0,-70"`, the road at (1000, 381). Saving
or baking in a shot writes the real map and terrain files: `git checkout map assets`.
`PLACE="x,z,frame"` puts the driven car on the road nearest that map point. Give a shot
its own config dir (`XDG_CONFIG_HOME=<scratch>/cfg`) for fresh settings. The start menu
is closed in shots unless `MENU=1`, the controls screen before it unless `INTRO=1`; `JOIN=127.0.0.1 NAME=Anna` joins a server at once.
Two players: start the server (`target/sandbox/release-dev/server`, throwaway
certificate, in the background with `nohup`), then two shots with `CAR_SIM_INSECURE=1`
and their own config dirs, the second started a few seconds after the first, more
frames for the first (they take turns building). Don't `pkill -f` anything matching
your own command line (it kills the shell); `pgrep -x server`, then `kill`. The tool windows (3–6) are always there,
collapsed to title bars stacked in the top right (`ui::tool_window`, agreed with the user
so they can be found); their keys (or a click) expand them, and the bars hide with the
panel (`2`). `CLICK="x,y,frame;x,y,frame…"` clicks into the
1280×720 window, e.g. on the map. `CAM`/`CAMAT` env vars place the camera (see
`shot.rs`). `shot.sh` fetches Mesa and Xvfb from the pinned nixpkgs on first use.
A shot takes about 35 s.

For looking at cars: `CAR="chassis[,engine]"` starts in that car (its place in
`CHASSIS`); `LINEUP="first,count"` parks that many cars one behind the other on an open,
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

## Multiplayer (agreed with the user, 2026-11-19)

A multiplayer sandbox first; the career comes back later, built on it. Native and web
both offer **"Play alone"** and **"Join server"** (a start menu, chosen at runtime).
No hosting from the game: a dedicated server (`src/bin/server.rs`, native, headless:
`MinimalPlugins` and the world core) runs on the user's VPS.

- **Each player owns their car**: full local physics, engine and force feedback, as
  alone. ~30 snapshots/s (pose, velocities, wheels, rpm, throttle, gear, lights; ~100 B,
  unreliable datagrams). Other players' cars are **remote cars** (model, wheels, sound),
  dead reckoned like traffic (see "Every car from the server alike"); each client
  collides its own car against them and applies the impulse only to itself.
- A client sends what others **see and hear**: chassis, paint, wheels, ride height and a
  sound description (layout, cylinders, `ExhaustSound`, turbo), not the engine's parts;
  re-sent when it changes (e.g. a swap in the garage, 4).
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
  dropped after 30 s. The traffic density slider only exists alone.
- **Transport**: WebTransport (QUIC, one UDP port, default 4433) with a real
  certificate; `wtransport` for the server and native client, the browser's
  WebTransport for the web (maybe `xwt` for both clients). Datagrams for snapshots and
  traffic, streams for the handshake, looks and events. Handshake checks the protocol
  version and the terrain's fingerprint. Dev only: without a certificate the server makes
  a throwaway one and a native client can skip verification (browsers can't join).
- **Nix**: `packages.server`, `packages.web`, `nixosModules.server`:
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
  `MinimalPlugins` + `LogPlugin` + `WorldPlugin` at 60 updates/s; traffic populates
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
  local server and two headless games: both joined, saw each other, shared traffic); 6. shared traffic (done: `traffic::Source` (Own/Server, switched by the
  client on joining/leaving, own traffic despawned); since protocol 5 sent like any
  car, see below);
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
  protocol and map, prints the answer). "No answer" with the page loading over HTTPS
  means UDP is dropped before the server (seen 2026-11-19 on the user's IONOS VPS; QUIC
  out of the sandbox works, e.g. `curl --http3-only https://cloudflare.com`).
- **Updates** (discussed with the user): the server checks only `PROTOCOL` and the map
  fingerprint, so client-only changes (driving physics, engine, tires, FFB, aids, UI)
  need no server update, and old and new clients play together. Bump `PROTOCOL`
  whenever a message's content changes (`protocol.rs`: `CarState`, `Looks`, `Motion`,
  a new message). The server needs updating too for: catalogue changes
  (chassis, paints, gearboxes, clutches, suspensions, compounds, bar sizes: sent as
  indices and checked against the reader's catalogues; a car the server can't read gets
  "The server doesn't know your car…", a client skips a car it can't read (traffic's or
  another player's) with a warning), traffic behaviour
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
  `switch_source` removes traffic and remote cars when joining or leaving. Name tags
  follow `Tag`. Alone (and on the server): `TrafficCar` simulates as before,
  `traffic::show` gives it `Looks` (`FactoryLooks`, per chassis with its base engine,
  16 ms for all 13) and `Motion` (steer, spin from rolled, suspension at rest, idling standing, 2000 rpm
  at 20 % load moving)
  and a `Transform` at the centre of mass, and `visuals.rs` draws it like any car (shared
  materials, bodies cached per chassis and paint). Remaining tells: traffic's suspension
  doesn't move, its engine (heard since other cars have engine sound) only idles
  standing and holds 2000 rpm at 20 % load moving (blended over 0–3 m/s; asked for by
  the user, no gears), how it drives.
  Tried with a local server and two headless games: name tag over the other player,
  traffic shown, a leaving player's car gone at once.
- **Parallel worlds** (planned with the user, 2026-11-21, not built): groups of players
  in a challenge (race, hide and seek) separate from the normal world and each other.
  Built on the above: a world component on cars, peers and anchors; the sending loop
  sends only a player's world, `populate` counts density per world, traffic `step`
  collides only within a world; the client needs nothing (maybe a message for the UI).
  Per-world settings like `--map-players self` and no tags for hide and seek.
- Not done yet: the browser path tried in a real browser (no
  browser here; it builds and links), rate limits on what a player sends (chat has one).
- Later: sanity checks on snapshots, Safari, career.
- **Name tags** (agreed with the user, 2026-11-21: in the scene, covered like anything
  else; just on or off, in the players window, saved as `tags on|off` in online.cfg):
  `name_tags.rs`, each name rasterised once with ab_glyph in egui's Ubuntu-Light (no
  bevy_text in the build) into a texture on an unlit, blended quad facing the camera,
  scaled with distance (constant size on screen), within 150 m. Systems that move the
  camera after `Update` go before `name_tags::PlaceTags`.

## Wheel support

Wheel and pedals come through gamepad input (gilrs), bound in the panel (2, "Wheel and
pedals") and saved to a config file. `src/wheel_base.rs` sets the base's rotation range
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
  from sysfs (readable even when not writable) or given in the panel, the axis is
  scaled to the game's range, and force feedback pushes back past either end (full
  force 10° past; stays on with the tire forces switched off).
- Clutch pedal: engine can stall, ignition restarts it. Without a bound pedal the
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
  (otherwise neutral). Without a pedal, the automatic clutch shifts. Paddles shift
  with the automatic clutch even when a pedal is bound.
- Each H gate is a button held while the lever is in it; neutral is "no gate held".
- No hill hold (removed at the user's wish): the handbrake is a parking brake toggled
  by Space or a handbrake button (`Car::parking_brake`, on when a car spawns or resets,
  a red (P) under the gear); a handbrake lever (an axis) works directly on top of it.

### Still to check on the real wheel

- "Test: push left" in the panel: tick "Reverse direction" if it pushes right (the sign
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
  back to the soft lock (the panel says "Wheel base is at …°"). The `range` sysfs file is root's; the udev
  rule only does `chown -f :games` on it when the device is added. Needs a `games` group
  (NixOS doesn't create one; `users.groups.games = { };`), the user in it (log in
  again), and the rule to have run after that (replug, or
  `sudo udevadm trigger --action=change --subsystem-match=hid`). Check with
  `ls -l /sys/module/hid_fanatec/drivers/hid:fanatec/*/range`, `getent group games`
  and `id`. If the group owns it but can't write, the mode needs `g+w` too.
- The panel shows "Wheel base set to 540°" once the range was written, and names the
  force feedback device or what's wrong with each candidate.

## Cars

Agreed with the user: lots of variety; a chassis is independent of engines and the
other components, engines are shared between cars, and a bigger (or better) engine can
be dropped into another car. **Space is the only hard limit** for a swap. Front- and
all-wheel drive exist. One car at a time, picked from a menu. Later this becomes a game
with a campaign, upgrades and personal cars, so a car is an instance built from
catalogue entries, and things like swap subframes are named so they can be priced.

- **Catalogues** (statics, entries referred to by their place; engine versions also by
  code): `chassis.rs` has `CHASSIS` (13: Minim city car, Vireo hot hatch, Linnet
  roadster, Tarn coupé (the default), Halden saloon, Skarn rally car, Harrow muscle car,
  Calder mid-engined, Rheon rear-engined, Vespera supercar, Wisp track-day special,
  Drover pickup, Carrow van; names are made up, no real cars), `GEARBOXES` (8, of three
  kinds: transverse, inline, transaxle), clutches, suspension kits and paints (the tire
  compounds are in `physics/tire.rs`).
  `physics/engine/families.rs` has `FAMILIES` (12 families, 22 factory versions, codes
  like W20T; W is today's 2-litre four, R the high-revving four with switching cams).
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
  with vertex colours. The cockpit is placed by the chassis' eye point.
- **Showroom (6) and garage (4)** are both in `garage.rs`; both go through `start`
  and `fit`. A changed `CarBuild` respawns the car's entity (its meshes are children);
  an engine-only change swaps it in while driving.
- **Traction control** (my addition, on by default, a checkbox in the panel): without
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
- **Fuel** (`physics/fuel.rs`, agreed with the user: R and the panel's fuel load slider
  refill, no stations; with starvation): each chassis has a `Tank` (litres, centre
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
  physics needed, following the roads matters, crashes at junctions are fine, a
  "Traffic density" slider in the panel, 0–60 cars, default 20; no HUD or map changes).
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
  merge, about 0.1 ms per step for all). Not done: right of way, overtaking, sounds,
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
  are caught (asserted by `a_power_slide_can_be_caught`).
- Tire compounds (`tire::COMPOUNDS`, agreed with the user; touring added later: least
  grip, best at 55 °C, lasts longest, rolls easiest): street (grips from cold,
  best at 65 °C, lets go gently past the peak, overheats when driven hard), sport, race
  (needs heat, best at 95 °C, peaks early and drops off, wears fast). Each has grip,
  peak slip angle/ratio with the Magic Formula C (stiffness B follows from the peak), a
  temperature window and a tread life (sliding energy). Cars start on cold tires; R
  gives fresh ones. A car's tire state survives an engine swap. Tests about other things
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
  kit comes aligned to its extra camber and allows ± its reach (`SUSPENSIONS`). Set in 4
  (`garage::alignment`).
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
- ECU tuning window (3, `src/ecu_ui.rs`): VE, target λ and spark grids with the
  operating point, block selection and +/-/set, "Correct VE here" (scales the cells
  around the operating point by measured/target λ), rev limiter, idle, factory tune.
  Tunes are saved by name as text (`src/tunes.rs`: `~/.config/car-sim/tunes/*.tune`,
  local storage on the web) with the build they were made for; loading reads over the
  engine's factory tune and warns if the build differs. Swapping the engine build in
  the setup panel replaces the tune with the new build's factory tune.
- Garage (4, `src/garage.rs`): parts from a catalogue (`parts.rs`: a slot per kind of
  part, each part a function fitting it onto the engine's hardware, relative to it; a
  factory version is a block and a selection), cam-gear timing and wastegate spring. The car's `EngineSetup.parts` says
  what it's built from. Building bakes the maps if the fingerprint changed (native: a
  thread; wasm: 12 ms slices per frame), with the old engine running meanwhile, and caches
  them (`storage.rs` folders: `~/.cache/car-sim/engines/<fingerprint>.maps`, local storage
  on the web); bolt-on parts fit at once. The new engine is new (no damage) and gets a
  factory tune unless unticked. The redline follows from the hardware (`safe_rpm`: 95 %
  of the valve float speed, rods' pull under 70 % of their strength); the factory tune's
  rev limit (first cam profile's with the stock ECU) and the rev counter follow it.
- Engine dyno (5, `src/dyno.rs`): a copy of the active car's engine (as built, tuned
  and worn; a run doesn't wear the car's) held at rpm by the brake, full throttle,
  settling 3 s at 1000 rpm, then swept at a set rate (100–1000 rpm/s, slow lets a turbo
  spool) to 50 rpm below the rev limit, sampling crank torque every 100 rpm (1 s
  simulated per frame). Plots N·m (solid) and hp (dashed); keeps the last 4 runs. The
  curves are piecewise linear between the maps' 500 rpm grid points.
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
  with big boost breaks stock rods. The garage shows the condition and repairs; R
  (reset) also repairs. Exhaust temperatures run hot (no losses before the turbine),
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
   `Terrain::ground(x, z, y)` blends overlapping roads (weights fade at road ends) and
   has levels (a deck carries what's within a metre below its surface); `sample` is the
   level beneath any bridge, `land` the bare terrain. Bridges are drawn as slab and
   parapets; parapets are obstacles. Not done: tunnels, junction shapes, piers.
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
pins are `ele` (above sea level; baked format 13 stores `Terrain::base`, the lowest
point's height, to convert). egui input is **not** absorbed in the editor (bevy_egui
then resets the mouse buttons whenever egui is over the pointer, dropping drags):
systems check `EguiWantsInput` themselves. The bake is shared code-wise: `src/osm.rs`
is `#[path]`-included by `tools/bake` (std + quick-xml only). Map file ids: new
elements without `version` (JOSM rejects one on new elements).

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
with `Looks` and `Motion` that isn't a `Car` (remote cars, traffic alone), the nearest 6
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
