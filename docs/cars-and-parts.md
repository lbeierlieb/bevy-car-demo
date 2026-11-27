# Owning cars and parts: the plan

Agreed with the user in rounds of questions on 2026-11-26 (the decisions are also short
in `CLAUDE.md`, "Owning cars and parts"). Milestones 1–8 are built. The milestones are
in the order they're meant to be built; each ends with clippy and tests passing and the
game playable.

## The game

The game is about tuning every aspect of a car. A player owns cars and parts, kept in a
garage, carried in trunks, and fitted at shops on the map.

- **Money** is in the plan from the start, every price 0 € until the economy is worked
  out (earning: events, jobs, selling).
- **Garage park**: one place on the map, everyone their own garage there (a private
  scene, like a shop's). Storage only: cars and parts, up to a limit high enough never
  to be hit by playing. The only place to switch cars.
- **Trunks**: each car has a cargo grid; the game packs what's loaded (no placing by
  hand). Passenger seat and rear bench can come out, giving more room. What's carried
  adds its mass where it lies. Loading and unloading everywhere (shops, garage,
  marketplace); **shipping** is the paid alternative (free for now): to the garage, and
  from the garage to a shop (an engine waiting at the engine shop).
- **Shops** (unlimited stock, an assortment per shop from the server's shops file, no
  waiting, a cost for fitting and for parts not brought):
  - **car shop**: new stock parts for every chassis; fits any part you bring except a
    long block's insides; works on gearboxes; takes engines out and puts them in.
  - **tuning shop**: the same, selling standard tuning parts (exhausts, sport cats, body
    parts, …).
  - **tire shop**: rims and tires; the only place a tire goes onto a rim or off it.
  - **engine shop**: only long blocks, in the car or brought in a trunk; sells nothing;
    keeps one engine for you (bring it in the van, come back with the car, they swap);
    rebores (oversize pistons then).
  - **alignment shop**, **dyno** as now; **paint shop** paints any of the fitted body
    parts (each part its own colour; a new body part's colour is picked when buying).
  - Anything replaced goes into your trunk if it fits, else the shop discards it.
  - Repairs are new parts, except the engine shop's rebore.
- **Showroom**: sells trims; later some showrooms only some chassis. **Used car dealer**
  sells worn trims. Buying a car, the old one goes to the garage or is traded in.
- **Used parts shop**: buys anything (it disappears), sells worn parts the server makes
  up.
- **Marketplace**, a place on the map: offers are brought (or shipped) there, which
  locks them; others buy. Parts and whole cars; an offered car or engine shows what it's
  built of. Rare parts come only from multiplayer events (weekly races: later, not
  planned yet) and are what's traded.
- **Special rescue** for a car that can't drive: to a shop or a gas station.
- **Wear**: crashes don't damage cars. Engine, tires, brakes (fade, pads), clutch and
  gearbox (too much torque, synchros), each getting worse before it fails (milestone 8
  for brakes, clutch and gearbox).
- **Accounts**: username (the name in the game) and password, made by the server's admin;
  the admin resets a forgotten one. A server option allows guests (kept for the session
  only). Playing alone uses a default user.
- **Persistence**: SQLite playing alone, MySQL for a single server, PostgreSQL for
  several instances sharing everything (marketplace too: instances only spread the
  physics); a player is active on one instance at a time.
- **One car for now**: the hot hatch, looking like an early-90s 3-door hatchback (the
  user: an EG Civic; the name stays made up, no badges), in two trims. The other twelve
  chassis and their engines go; new ones come later.

## The model

### Definitions, items, places

- A **definition** is a catalogue entry with a **stable string id**
  (`vireo.bonnet.vented`, `b.pistons.forged`). The server defines them; a changed
  definition changes every item of it; definitions aren't meant to be removed (if one
  is, its items go, smoothly).
- An **item** is one owned piece: its definition, condition (wear, as each kind needs:
  pad thickness, synchro wear, the engine's per-cylinder damage…), colour if paintable,
  bore oversize for a block, knob settings (camber, ride height…), and where it is.
- **Where an item is**: in a slot of a car or of another item, in a trunk (with its
  place in the grid), in a garage, waiting at a shop, at the marketplace (locked).
- Items **nest**: a car has slots; an engine, a long block, a gearbox and a wheel are
  items with slots of their own.

### The hatchback's slots

Car (the chassis is the bare shell: floor, roof, rear quarters, pickup points, tank,
cargo grid):

| Group | Slots | Required |
|---|---|---|
| Body (painted) | front bumper; front lip (needs the bumper); bonnet; fender L, R; door L, R; mirror L, R; side skirt L, R; hatch; rear spoiler (needs the hatch); rear bumper; diffuser (needs the rear bumper) | none; a missing piece adds drag |
| Interior | driver's seat, passenger seat, rear bench | the driver's seat |
| Per corner (×4) | wheel, brake, spring, damper | all |
| Per axle (×2) | anti-roll bar | no |
| Drivetrain | gearbox, clutch | both |
| Exhaust | front pipe, catalyst, centre pipe/resonator, silencer | the pipes (a straight pipe is a pipe part) |
| Cooling | radiator; intercooler (with a turbo; needs a bumper with an opening) | the radiator |
| Electronics | ECU (stays with the car: a swapped engine needs a new tune) | yes |
| Engine | engine | yes: taking one out is a swap |

Items with slots:

| Item | Slots | Worked on by |
|---|---|---|
| Engine | long block; intake manifold, throttle body, air filter, exhaust manifold, turbo or supercharger, flywheel | bolt-ons at car and tuning shops; comes out of the car whole |
| Long block | block (family, version, bore oversize), crankshaft, rods, pistons (matching the bore), head gasket, cylinder head, camshafts, valve springs, cam phasers | engine shop only |
| Gearbox | gear set, final drive, differential | car and tuning shops |
| Wheel | rim, tire | tire shop only (rims and tires also loose in trunks and garages) |

Springs, dampers, brakes, wheels: each corner its own item (sold in pairs or fours).

### A definition holds

Id, name, slot kind, what it fits (chassis; engine family and version; bore size;
`needs` / `provides` tags such as a bonnet cut-out for a supercharger, a bumper opening
for an intercooler), size box (for the cargo grid), mass, price, whether it's painted,
the slots it has, its effects on the car or engine (engine fields as today's `Effect`;
car fields: drag, downforce front/rear and their ride-height sensitivity, mass, brake
torque, ride rate, damping, insulation, …), its knobs (ranges with effects across them),
how it wears, and its geometry (how it's drawn).

A **trim** is a stock car: an assembly of definitions with factory knob settings and
paint. The hatchback's two:

| | 1.6 | 1.8 |
|---|---|---|
| Engine | B16 | B18 (new version of the B family) |
| Wheels | 14" steel, 185/60 R14 | 15" alloys, 195/55 R15 |
| Brakes | solid front discs, rear drums | vented front discs, rear discs |
| Differential | open | limited slip |
| Body | | rear spoiler |

## Milestones

### 1. Parts as data (done 2026-11-26)

Built as below, with a bridge instead of rewriting the simulation: trees resolve to the
`CarBuild` + `Parts` it builds from, and shops' changes are assembled back into the tree
(see `CLAUDE.md`, "Owning cars and parts"). The chassis' own numbers (body, bay, mass…)
stay in `Chassis` for now. Done before it: the catalogue down to the hatch and the B16
(milestone 2's first step).

The item model replacing `CarBuild` + `Parts`, with today's Vireo B16 as its first
trim, coming out bit for bit as now (as the catalogue move was proved: dump and compare
setups; a car driving the same steps to the bit).

- `catalogue`: definitions with string ids, slot kinds, trims; car fields as effects
  (today's chassis numbers become the shell's plus its stock parts' effects).
- An `Item` tree (`CarSpec` becomes the car's item tree plus the ECU's tune); `CarSetup`
  composes it: shell + parts → masses, aero, suspension, brakes; engine items →
  `EngineHardware` as `Parts` does now.
- Wire: catalogue and item trees by string id once per catalogue, indices in messages.
- The existing shops (garage, paint, alignment, dyno, showroom) keep working on the new
  model, still the client's word as now.

### 2. One car (done 2026-11-26)

Built as below; the showroom still offers the chassis with an engine choice, which now
picks the trim (selling trims as such comes with the shops' rework in 6).

- The catalogue down to the hatchback (other chassis, the engine families but the B
  series, their baked maps go; traffic is hatchbacks in trims and paints; the showroom
  two cars; the hunt's "muscle vs rally" goes; tests that ran every car run the trims).
- New data: dimensions, mass, wheels, track like an early-90s 3-door; B18; the two trims.
- The body generator's outline redone for that look (the pieces come in 3).

### 3. The body in pieces (done 2026-11-26)

Built as below (see `CLAUDE.md`, "Owning cars and parts"). Side skirts are their own
shape, not a region; mirrors are drawn only with their door, the lip with the front
bumper, the spoiler with the hatch, the diffuser with the rear bumper. Lip, skirts and
diffuser have definitions but no trim or shop fits them yet. The parts shop's engine
stand still draws its own engine.

- `body.rs` split: the shell, and each body part's mesh made from the shell's outline
  regions (bumpers, bonnet, fenders, doors, hatch, mirrors, skirts) or its own geometry
  (lip, spoiler, diffuser), each its own colour; missing pieces leave the opening.
- Under the bonnet: engine (block, head, intake, throttle, filter, exhaust manifold,
  turbo), radiator, battery, from the fitted parts (the parts shop's engine stand is the
  start). Underneath: floor, tank, arms, the exhaust line part by part.
- `Looks` carries the fitted visible parts and colours (protocol bump); traffic,
  showroom, shops' scenes draw from it.

### 4. Accounts and saving (done 2026-11-26)

Built as below (see `CLAUDE.md`, "Owning cars and parts"). A server without
`--database` keeps nothing and lets everyone in by name, as before. Guests aren't in the
database at all (nothing of theirs is kept, so nothing needs erasing). The saved car is
text by string ids (`saved.rs`), not the wire format, so it survives catalogue changes.
Not done: limiting login attempts, a last save when a dedicated server is stopped
(SIGTERM; it saves every minute), logging in from a browser tried for real.

- A database layer (`sqlx`: SQLite, MySQL, PostgreSQL; migrations per dialect), server
  only (the web client needs none).
- Accounts: `server` subcommands to add users and reset passwords (argon2 hashes);
  login in the handshake (the name field becomes username + password); `--guests on|off`.
  Playing alone: SQLite in the user's data folder, the default user.
- The instance lock (a row per player, refreshed while playing, taken over once stale
  after ~30 s).
- Saved: the player's car (item tree, fuel, tires, wear), loaded on joining instead of
  the start car.

### 5. Garage park, several cars, trunks (done 2026-11-26)

Built as below (see `CLAUDE.md`, "Owning cars and parts"). Before the shops' rework (6)
loose parts come from the existing shops: what a shop's change takes off the car goes
into the trunk while there's room (else the shop keeps it), and a car bought in the
showroom puts the old one in the garage (no trade-in yet). Unloading and loading work at
every shop and the garage park; shipping to the garage only from a shop. Nothing yet takes
seats out, so the seats' areas of the hold stay closed in play. The special rescue only
works for a car that can't drive (engine wrecked, tank under 0.5 L), to the nearest gas
station or parts shop (a car workshop since 6).

- The garage park on the map (a kind in the shops file), the garage's scene: cars and
  parts listed, switch car, load and unload.
- Cargo grids per chassis (floor cells with a height each; the removable seats' areas
  join when empty), the server's packer (largest first, turning parts every way), the
  load's masses in `CarSetup` (a spec change, like the fuel).
- Shipping (free, instant for now): to the garage, from the garage to a shop.
- The special rescue (to a shop or a gas station) in the central menu.

### 6. Fitting at shops (done 2026-11-26)

Built as below (see `CLAUDE.md`, "Owning cars and parts"). The parts shops became car
workshops and tuning shops, tire shops are new; each shop's assortment is listed in the
shops file (`sells` lines, nothing by default). Long blocks' insides wait for the engine
shop (7): until then only whole engines (the factory ones, sold as assemblies) are swapped.
What comes off goes into the trunk, else the shop holds it (up to `SHOP_HOLD`), else it's
thrown away. Settings set in the game's scenes (alignment, paint, the dyno) go to the
server as works too. Not done: money (everything free), items' own condition (wear of a
part taken off isn't kept), the intercooler's bumper opening.

- One fitting mechanic for car, tuning and tire shops: the car's slots (and the trunk's
  items) listed, buy new or fit from the trunk, the old part to the trunk or discarded,
  engines swapped, wheels made and taken apart (tire shop). The prompt line and menu kit
  as every menu.
- **Car changes as requests**: the client asks, the server checks (fit rules, the shop's
  assortment, money) and answers with the car (ends trusting the client's builds).
- Alignment (knobs of fitted parts), paint (per body part) and dyno moved onto it; the
  showroom sells trims (old car to the garage or traded in).

### 7. Engine shop (done 2026-11-27)

Built as below while the user slept (see `CLAUDE.md`, "Owning cars and parts"; the
decisions I made are under "Decided while building milestone 7" at the end). Not done:
bore wear (the pistons carry the knock wear, so new pistons repair it; a rebore repairs
nothing by itself), loose internals for sale (only fitted), the engine stand in the
workshop showing the engine worked on, a lowered bake priority on servers other than
Linux, the dyno's cam changes still baking in the game rather than asking the server.

- Long block work in the car or on an engine from the trunk; rebores with oversize
  pistons; one engine kept per player; the server bakes maps for changed long blocks
  (cached by fingerprint, sent to the client; the old engine runs meanwhile).
- Items' own condition: the engine's damage on the long block's parts and the turbo
  (tires' treads on the tires too), kept by a part taken off; saved and sent.

### 8. New parts and physics (done 2026-11-27)

Built as below while the user slept (see `CLAUDE.md`, "Owning cars and parts"; my
decisions are under "Decided while building milestone 8" at the end). Not done: the dyno's
boost knob for superchargers (their pulleys are fixed), an intercooler with a
supercharger, superchargers' own wear, brakes set per corner (the left corner's part
counts for its axle, as every per-axle part), keeping front and rear brake parts apart,
re-tuning the ECU when forced induction is fitted (the tune stays as it was, as with turbo
kits: a dyno's factory tune is safe), the whine, fade and slipping clutch heard or felt on
a real wheel.

- Aero by ride height and pitch (splitter, diffuser), wing angle as a knob, drag from
  missing pieces; ride height as a knob of springs/dampers.
- Brakes as parts (torque, unsprung mass, smallest rim), pad wear and fade.
- Clutch wear and slip, gearbox torque limit, synchro wear (grinding, then refusing).
- Seats' masses (out and lighter seats save weight; insulation unchanged).
- The supercharger (new engine physics) with the bonnet cut-out it needs.

### 9. Trading

- Used parts shop (buys; sells made-up worn parts), used car dealer (worn trims).
- Marketplace: offers locked there, buying, an offer's parts inspected (the same view as
  the garage's).

### Later

Scheduled events with rewards (weekly races, special parts), prices and earning, more
chassis, trailers.

## Settled later (2026-11-26)

- The cargo grid has 10 cm cells; how the packer works is mine to decide (parts may be
  turned every way). The player only sees "fits" or "no space for this part".
- Any shop holds parts for a player (shipped there, or left), a reasonable amount each
  (`SHOP_HOLD`: 20 items per player per shop).
- A guest starts like any new player; everything is erased when they leave.
- A car traded in or sold goes with what's in its trunk, after a warning.
- Limits (large, only against endless growth): a garage holds 50 cars and 2000 items.

## Settled before milestones 7–9 (2026-11-27)

The user's answers:
- **The engine shop sells long-block internals** (pistons in bore sizes, rods, cams,
  heads, …) itself, by `sells` lines like any shop ("sells nothing" is dropped); it
  fits them, and those brought in a trunk or held there.
- **The supercharger comes as two kits**: a belt-driven Roots blower (boost from idle
  following rpm, belt drag, hot charge, whine, through a bonnet cut-out) and a
  centrifugal one (boost rising with rpm², under the bonnet).
- **Money in milestone 9**: every account gets a wallet (saved, shown in the menus,
  a start amount); shops keep charging 0 €; marketplace sellers set a price, and the
  used parts shop pays a small amount for what it buys.

Decided by me (the user asleep; to look over):
- Items get their own condition in milestone 7 (the engine's per-cylinder damage moves
  onto the long block's parts, so replacing pistons, rods, springs, gasket or bearings
  repairs that) and in 8 for brakes, clutch and gearbox; a part taken off keeps its
  wear. Milestone 9's worn parts and cars build on that.
- The server bakes changed long blocks' maps on a background thread at low priority
  (the VPS has one slow vCPU: the physics must not stutter); the car keeps its old
  engine until they're done.
- New shops (engine shop, used parts shop, used car dealer, marketplace) get places
  and made-up names near the existing towns.

## Decided while building milestone 7 (2026-11-27, the user asleep; to look over)

- **Where the wear sits**: pistons (each cylinder's knock wear), connecting rods (fatigue),
  the cylinder head (the valves bent when they float), the head gasket, the crankshaft (the
  bearings), the turbo; and each tire's tread on the tire (the same mechanism, so tires
  taken off keep their wear too). The "Internals" slot (forged rods and pistons, an
  upgrade on top of the rods and pistons) carries none. Crashes still damage nothing.
- **Bore sizes**: stock, +0.25, +0.50, +0.75 and +1.00 mm. A block is only ever bored
  bigger; afterwards it takes only pistons of its size, which the engine shop sells in
  every size (the same parts, a size each). The rebore and the new pistons are one request.
  A new long block from a shop comes with the stock bore. Bore wear isn't modelled: new
  pistons repair the knock wear without a rebore.
- **What the engine shop does**: a long block into an engine, anything inside a long block,
  a rebore, and whole engines swapped into the car (that's how the engine kept there goes
  in). It works on the car's engine and on an engine or long block brought in the trunk or
  kept there. It keeps **one engine and one long block** per player (a long block swapped
  into the kept engine has to go somewhere); other shops hold up to 20 parts as before. It
  sells what its shops file says; the repo's sells the internals and the B16 and B18 long
  blocks (a B18 block in a B16 engine is a classic swap).
- **The engine shop** is "Motorenbau Mühlbach" in Baunach (120, 2700), near the tire
  shop and the dyno there; it uses the workshop scene (the car on the lift).
- **Bakes on the server**: at nice 19 on Linux (the thread and its workers); the car
  drives on as it was until the maps are done, then the player gets the new car. The work's
  answer tells them the maps are being worked out (in the window's warning colour, for
  now). A change made meanwhile builds on the waiting one. Maps baked are kept for the
  server's lifetime (not on disk) and given to games that ask.
- **Saved cars** (format 3) write the wear on the parts; older saves' wear goes onto the
  parts when read. Protocol 40.

## Decided while building milestone 8 (2026-11-27, the user asleep; to look over)

- **Aero**: body parts carry drag, downforce over each axle and a ground effect (a share
  more per centimetre the body comes nearer the ground at that axle, from the axle's mean
  suspension travel, between 0.2 and 1.8 of its design value), so ride height and pitch
  matter. The lip (0.1 m² at the front), side skirts, diffuser (0.08 m² at the rear) have
  ground effect; the 1.8's roof spoiler gives a little downforce; a new **rear wing**
  (0.32 m² at 10°, drag 0.05 m²) has its angle as a knob (0–16°, from 6°): downforce with
  the angle from −4°, drag partly with its square. A missing piece adds drag: bumper 0.05,
  bonnet 0.04, hatch 0.04, a door 0.02, a fender 0.015 m²; without mirrors 0.008 m² less.
  The body's underside makes no lift or downforce of its own.
- **Ride height** is a knob of the springs (not the dampers): stock springs have none,
  the sport kit's go −40 to 0 mm (from −25), the track kit's coilovers −70 to +10 (from
  −40). Lowering shortens the spring's free length, so the body sits lower and the arms
  sit at a different angle (camber changes with it, the bump stops come sooner). Set at
  alignment shops, as is the wing's angle.
- **Brakes**: each part says its torque per wheel (solid front 950, vented 1100, the new
  big four-piston kit 1400 N·m; rear drum 190, rear disc 230), heat capacity, cooling,
  fade temperature and pad life; the car's brake torque and balance come from them (no
  longer from the tires' grip), the alignment shop's force and balance on top. The pads
  fade from their temperature over 200 K to 45 %; worn past 80 % they grip less, worn
  through 30 % is left; hot pads wear faster. Pad lives are long (about 2000 stops
  from 100 km/h on cool pads, a few times fewer hot): it shows after hard track days. Any brake
  part goes into any corner (a drum at the front is allowed, just weak).
- **Clutch**: wears by the work it does slipping (pulling away wears a little), lasting
  about 500 kJ per N·m it passes; past 60 % worn it passes less, down to half just
  before it's worn out (then 15 %): a worn clutch slips under the engine's full torque.
- **Gearbox**: a torque limit per gearbox (the stock 5-speed 200 N·m, above the B18's
  185; the 6-speed 360): more wears the gear in by how far over it is (twice the limit
  wears a gear out in ten minutes). Each forward gear's synchro wears by the work of
  bringing the input shaft to its speed (the shaft turns with the gear in gear, with the
  engine in neutral with the clutch up, and keeps its speed with the clutch pressed).
  Past half worn a synchro can't bring a gear in from as far off its speed (3000 rpm at
  half, down to 150 rpm worn out): quick shifts grind and the gear stays out, gentle ones
  go in; worn out, only rev-matched shifts (double-clutching) work. With the automatic
  clutch a refused gear leaves the box in neutral, grinding a moment. Reverse has no
  synchro. One wear value per gear covers its synchro and its teeth.
- **Seats**: the chassis' mass is now without seats (770 kg), the stock seats 16 kg, the
  rear bench 18, a new bucket seat 6 kg, each at its place (the balance changes when the
  rear bench comes out). Insulation unchanged.
- **Superchargers**: both kits go into the turbo's slot (renamed "Turbo or supercharger":
  a turbo and a supercharger together isn't possible). The Roots blower (pulleys 1.5:1)
  gives the 1.6 about 0.5–0.6 bar from 2000 rpm up, the centrifugal one (impeller geared
  19:1) 0.1 bar at 3000 and 0.4 at 6000; with the stock compression the factory tune is
  knock-limited, so the gain is about a fifth to a quarter in torque. Both have a bypass
  valve for part throttle, no intercooler (the Roots blower's charge reaches about 80 °C), and
  don't wear. They don't change the cylinder side's maps (no bake). The tune isn't
  changed when one is fitted (as with turbo kits): a dyno's factory tune is safe on it.
  The Roots blower needs a **bonnet with a cut-out** (a new body part, a little more
  drag), the first use of parts' `needs`/`provides` tags (an engine part needing a body
  part). They whine in the engine sound (pitch following the crank, louder with the
  revs), never listened to.
- **Shops**: car workshops sell the stock brakes; tuning shops the wing, the bucket seat,
  the big brake kit, the cut-out bonnet and both superchargers.
- **Shown**: wear of brakes, clutch and gearbox in every workshop's window (as engine
  parts'), ride height and wing angle in the alignment window, brake temperatures and
  pads, clutch and gear wear, downforce and boost in the telemetry panel (2), hints for
  fading brakes, worn pads, a slipping clutch and a grinding synchro.
