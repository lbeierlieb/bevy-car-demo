# Notes: Networking

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/net/`; keep it up to date.

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
  `packages.windows` (see "Windows build"), `packages.server`, `packages.web`, `nixosModules.server`:
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
