# Notes: Catalogue, owned cars and parts

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/catalogue/`; keep it up to date.

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
block (`LONG_BLOCK_SLOTS`), on the engine (`ENGINE_SLOTS`, and its exhaust manifold:
`Kind::Manifold`, turbos on its mounts, see "Exhaust manifolds") or in the car (`CAR_SLOTS`);
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
forest now, `OFFLINE=1 CARCAM="-6,1.3,0"` shows the car side on in the village
(`TRIM="chassis,option"` any stock car: `TRIM=1,0` the Merrow).

**The Merrow** (2026-11-27, `docs/pushrod-v10.md` milestone C; the second chassis, at
place 1): an early-90s open V10 roadster (made up), the R80 at 0.42 m behind the front
axle, rear-wheel drive, limited slip, 1460 kg full (49.8 % front), 2.44 m wheelbase,
4.45 × 1.92 m, 275/40 R17 front and 335/35 R17 rear on 9.5"/11.5" alloys, its own vented
discs (`merrow.brake.front-vented`/`rear-vented`, 17" rims), an 85 L tank, two seats (no
"Rear bench" slot; the lid is "Boot lid", `catalogue/items.rs`'s `layout`), a trunk over
the rear axle, side pipes; trim `merrow.r80` (so `engine.r80`), paint "Dull navy",
gearbox "6-speed heavy duty (inline)" (700 N·m, clutch 720; gearbox definitions are made
for the chassis whose gearbox is of their kind). 0–100 in 4.7 s (`zero_to_hundred`,
ignored). An open car's first opening is its cockpit (`Body::cockpit`): the deck's open
there, sills round it, a bulkhead behind the seats, so bonnet and boot lid come off like
the hatch's; `Body::bulges` (bonnet ridges) and `side_pipes` (protocol 51). Mechanicals:
a longitudinal V (banks at 90°, low heads, plenum in the valley), gearbox case,
propeller shaft, differential and half shafts, exhausts to side pipes. **Wired in**
(milestone D, 2026-11-27): the car dealers in Reckendorf and by the Gewerbepark and both
used car dealers sell `merrow.r80` (the other showrooms show it only to look at), the car
workshops its stock body parts, brakes, springs, dampers, bars, the inline gearbox, the
limited slip and `engine.r80`, the tuning shops its lip, skirts, spoiler, wing,
diffuser, cut-out bonnet, sport/track springs and dampers and bars, the engine shop the
`r80` long block. Traffic weight 0.06 (one car in ~50, a test holds 40–60;
`drivers_follow_the_roads` makes every fifth car the rarest chassis). Its sound is
`Layout::OddFireV10` (see "Audio"). Handling: `a_power_slide_can_be_caught` is back for
it, floored on the circle in second and third **with traction control** (6.5° and 9.9°,
caught); without, the bot spins it in both gears (the rear at 40° before full opposite
lock), in fourth it doesn't slide (`power_slides` prints gears 2 and 3; `trace_power_slide`
reads `SLIDE_GEAR`, `SLIDE_TC`). `traction_control_launches_the_roadster` (first gear,
8.7 m/s after 1.5 s, rear wheels 1.1× the road speed; without TC 3.9×); ABS stops it
from 90 km/h in 28.1 m (33.0 without). Springs, dampers and bars unchanged (load moved
56 % front on 51 % weight). Shots: the showroom's second car is `ArrowRight` on its first
row.

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
rib, trailing zeros left out, at most `MOST_WEAR` 12 (8 before protocol 50); on the wire, protocol 40).
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
turbo kit); in the turbo's slot, renamed "Turbo or supercharger", since exhaust
manifolds' milestone 1 a slot of their own, `catalogue::SUPERCHARGER_SLOT` "Supercharger"
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
provides, and so does no bonnet at all (`unmet_need`; the user, 2026-11-27; protocol 59, as
whole cars an older client would refuse can come). Drawn in `mechanicals.rs` (the blower sticking up through the bonnet with the
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
  twelve chassis go (others come later). The second, since 2026-11-27: the Merrow, an
  early-90s V10 roadster (`docs/pushrod-v10.md`).
- Engines come out whole (bolt-ons on); the ECU, exhaust after the manifold and cooling
  stay with the car. Rims and tires are items of their own too (loose in trunks and
  garages), put together only at the tire shop. Springs, dampers, brakes, wheels per
  corner. Doors are parts. Gearboxes (gear set, final drive, differential) are worked on
  by car and tuning shops, not the engine shop.

## Exhaust manifolds (planned with the user, 2026-11-27; built the same day)

The plan, with milestones: `docs/exhaust-manifolds.md` (keep it up to date; its last
section lists what was decided while building). **The manifold decides the layout,
turbos sit in its mounts.** Milestone 1 (data, no physics change): `Family::manifolds`
(`ManifoldDef`: id, name, `hardware::Manifold` (primary length, `Collector`, branches),
`mounts`), `Parts::manifold`/`Preset::manifold` (its place in the family's; sets
`Exhaust::manifold`, before the parts' effects), items `Kind::Manifold` (`Does::Manifold`, the engine's "Exhaust
manifold" slot, `PartDef::engines` = the family's name) and `Kind::Turbo` (the old
`turbocharger.small|medium|large`, `Does::EnginePart` of the catalogue's `mounted` slot
`TURBO`, 11) on the manifold's mounts; the old turbo slot is `SUPERCHARGER` (12). Rules
in `items::engine_misfit` (family, every mount the same turbo and knobs or none, no turbo
with a supercharger), used by `Item::fine`, `resolve` and `Bench::finish`;
`fitting::mounts`, `engine_on`, `made_for_engine` for the workshop window ("Fit one on
every mount"). B series: `b.manifold.stock`, `b.headers.short`, `b.headers.long-tube`,
`b.manifold.turbo` (1 mount); R80: `r.manifolds.stock`, `r.headers.long-tube`,
`r.manifolds.turbo` (2), `r.manifold.turbo-crossover` (1). Saved cars format 4 (old ids
converted, `saved::mount_old_turbo`). Protocol 53. Every factory engine and old part
combination comes out the same hardware (no re-bake). Milestone 2 (physics, protocol 54):
`EngineHardware::turbos` alike (`turbo_count`), each turbine and wastegate passing its
share (`exhaust_pressure`), compressors and diverters × turbos into the ducts
(`tract_inflow`), one shaft state, one shared intercooler (sized for the whole engine);
`TurboKit::fit` sizes a turbo for the displacement behind its branch (efficiency +2 % per
doubling, `TURBO_SCALE_EFFICIENCY`, so a big single holds boost high up; the twins spool
quicker by their lighter shafts); `cycle.rs` reflects 0.8 of the header's wave off a
`Tuned` collector, `TURBINE_REFLECTION` 0.3 off a turbine's inlet; the fingerprint has
the primaries and the collector, not the branches. The oil→coolant exchange grows with
the engine (`thermal.rs`, by the oil's heat capacity). Numbers: the plan's "Decided
while building milestone 2"; `turbo_layouts` (ignored) prints them. Milestone 3 (the
game, protocols 55–56): the workshop's manifold screen (readouts as info rows: primaries,
cast or tubes, branches, collectors, mounts, the turbos; a row per mount; turbos go on and
come off every mount at once, new or as many alike brought; another manifold takes the
turbos over only onto as many mounts, else they stay on the old one); `mechanicals`
draws it from the tree (`Headers`: `ManifoldDef::cast` logs with stubs or tubes, long
ones low and far back, the B series' long tubes a 4-2-1, the 10-into-1's left bank piped
behind the engine across to the right bank's turbo; a snail and compressor per filled
turbine inlet, sized by the displacement behind the mount, each with its exhaust line and
filter pipe); `ExhaustSound::turbos` in the looks (twins whistle as two bands
`TWIN_DETUNE` apart, together as loud as one, a V's on its own side); telemetry names
twin turbos. Shots: `FITIDS` fills every mount (manifold first), `CARCAM`'s fourth number
looks that far ahead of the car's middle; the R80's bay from behind over the cockpit
(`OFFLINE=1 TRIM=1,0 UNFIT=Bonnet FITIDS="r.manifolds.turbo,turbocharger.medium"
CARCAM="0.4,2.6,-1.6,1.0"`), from the side (`UNFIT=Bonnet,Fender CARCAM="-1.6,0.8,-0.5,1.0"`),
the Vireo's from above (`TRIM=0,0 UNFIT=Bonnet FITIDS="b.manifold.turbo,turbocharger.medium"
CARCAM="0.5,2.5,0.1,1.3"`); the manifold screen at the tuning shop: `CAR=1
PLACE="-1950,-3160,150" TAPS="Enter@200;ArrowDown*2@206;Enter@214;ArrowDown*2@222;Enter@230"`.
