# Owning cars and parts: the plan

Agreed with the user in rounds of questions on 2026-11-26 (the decisions are also short
in `CLAUDE.md`, "Owning cars and parts"). Nothing of it is built yet. The milestones are
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
- **Wear**: crashes don't damage cars. Engine and tires as now; brakes, clutch and
  gearbox (too much torque, synchros) to come, each getting worse before it fails.
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

### 1. Parts as data

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

### 2. One car

- The catalogue down to the hatchback (other chassis, the engine families but the B
  series, their baked maps go; traffic is hatchbacks in trims and paints; the showroom
  two cars; the hunt's "muscle vs rally" goes; tests that ran every car run the trims).
- New data: dimensions, mass, wheels, track like an early-90s 3-door; B18; the two trims.
- The body generator's outline redone for that look (the pieces come in 3).

### 3. The body in pieces

- `body.rs` split: the shell, and each body part's mesh made from the shell's outline
  regions (bumpers, bonnet, fenders, doors, hatch, mirrors, skirts) or its own geometry
  (lip, spoiler, diffuser), each its own colour; missing pieces leave the opening.
- Under the bonnet: engine (block, head, intake, throttle, filter, exhaust manifold,
  turbo), radiator, battery, from the fitted parts (the parts shop's engine stand is the
  start). Underneath: floor, tank, arms, the exhaust line part by part.
- `Looks` carries the fitted visible parts and colours (protocol bump); traffic,
  showroom, shops' scenes draw from it.

### 4. Accounts and saving

- A database layer (`sqlx`: SQLite, MySQL, PostgreSQL; migrations per dialect), server
  only (the web client needs none).
- Accounts: `server` subcommands to add users and reset passwords (argon2 hashes);
  login in the handshake (the name field becomes username + password); `--guests on|off`.
  Playing alone: SQLite in the user's data folder, the default user.
- The instance lock (a row per player, refreshed while playing, taken over once stale
  after ~30 s).
- Saved: the player's car (item tree, fuel, tires, wear), loaded on joining instead of
  the start car.

### 5. Garage park, several cars, trunks

- The garage park on the map (a kind in the shops file), the garage's scene: cars and
  parts listed, switch car, load and unload.
- Cargo grids per chassis (floor cells with a height each; the removable seats' areas
  join when empty), the server's packer (largest first, turning parts every way), the
  load's masses in `CarSetup` (a spec change, like the fuel).
- Shipping (free, instant for now): to the garage, from the garage to a shop.
- The special rescue (to a shop or a gas station) in the central menu.

### 6. Fitting at shops

- One fitting mechanic for car, tuning and tire shops: the car's slots (and the trunk's
  items) listed, buy new or fit from the trunk, the old part to the trunk or discarded,
  engines swapped, wheels made and taken apart (tire shop). The prompt line and menu kit
  as every menu.
- **Car changes as requests**: the client asks, the server checks (fit rules, the shop's
  assortment, money) and answers with the car (ends trusting the client's builds).
- Alignment (knobs of fitted parts), paint (per body part) and dyno moved onto it; the
  showroom sells trims (old car to the garage or traded in).

### 7. Engine shop

- Long block work in the car or on an engine from the trunk; rebores with oversize
  pistons; one engine kept per player; the server bakes maps for changed long blocks
  (cached by fingerprint, sent to the client; the old engine runs meanwhile).

### 8. New parts and physics

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

## Still to settle (before the milestone that needs it)

- How fine the cargo grid is (10 cm cells?), and whether parts may lie on their side.
- Whether "keeps one engine for you" generalises to any shop holding items shipped
  there for you, and how many.
- What a guest gets (the start car, gone when they leave?).
- Items in a car's trunk when it's traded in or sold.
- Limits: how many cars and items a garage holds.
