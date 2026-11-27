# Notes: Cars, handling and the engine model

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/physics/`; keep it up to date.

## Cars

Agreed with the user: lots of variety; a chassis is independent of engines and the
other components, engines are shared between cars, and a bigger (or better) engine can
be dropped into another car. **Space is the only hard limit** for a swap. Front- and
all-wheel drive exist. One car at a time, picked from a menu. Later this becomes a game
with a campaign, upgrades and personal cars, so a car is an instance built from
catalogue entries, and things like swap subframes are named so they can be priced.

- **Catalogues** (data in a `catalogue::Catalogue`, the server's, see "Server
  catalogue"; entries referred to by their place; engine versions also by code). The
  built-in one (`src/catalogue/`): `chassis.rs` has the chassis (only the Vireo hatch, B16 and B18, from 2026-11-26; the
  Merrow roadster with the R80 since 2026-11-27,
  see "Owning cars and parts"; the other twelve (Minim, Linnet, Tarn, Halden, Skarn,
  Harrow, Calder, Rheon, Vespera, Wisp, Drover, Carrow) and their engines are in git
  history before that; names are made up, no real cars), gearboxes (the two
  transverse ones and an inline 6-speed), clutches, suspension kits and paints (the tire
  compounds' rubber is `physics/tire.rs`'s `COMPOUNDS`, named in `catalogue/parts.rs`).
  `engines.rs` has the families (the B series, versions B16 and B18; the R-series V10,
  R80, the Merrow's; `EngineHardware::default`
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
- **Maximum throttle** (asked for by the user, 2026-11-27, for keyboard players: keys
  know only full or nothing; moved from the driving aids into the controls the same day):
  `Profile::max_throttle` (per control profile, 30–100 %, 100 = off, the default;
  `wheel.cfg` `max_throttle` after the profile's curves), a slider in Bindings under
  the throttle's and full power's rows. `read_wheel` scales the throttle (after the
  curve, before the linkage, so not exactly the torque share): the server just gets a
  smaller throttle (no protocol or server change). Held, `Action::FullThrottle` ("Full
  power", left Shift in the keyboard profile; `wheel.cfg` `keys 3` binds it in an older
  file's first profile if Shift is free) lifts the limit.
- **ABS** (asked for by the user, on by default, next to traction control): per wheel,
  eases the brake share to hold a slip ratio of 0.1, the same smooth way (`ABS_*` in
  `sim.rs`), off below 2 m/s. Without it, braking while steering locks the inside
  front wheel of most cars and they go straight on (`abs_keeps_the_wheels_turning`).
- **A clutch slips** when the engine makes more torque than it passes, so a swapped
  engine may need the sport or race clutch. Gearboxes have no torque limit yet.
- **The automatic clutch** (no clutch control bound; with one bound, none of this, asked
  for by the user, 2026-11-27): up a gear, the throttle stays shut after the gear goes in
  until the engine is down to the gear's speed (`CUT_RPM`), then fades back in
  (`CUT_FADE`), so it doesn't flare against the slipping clutch. Pulling away it bites
  from idle + `BITE_ABOVE_IDLE` (150) to full `BITE_SPAN` (750) higher (was idle + 700
  to + 2200, pulling away at 2500 rpm; now about 1450), and closes anyway over
  `FEED_TIME` (0.8 s) on the gas (`feed`, a driver's foot coming up), so in too high a
  gear it drags the engine down and stalls it: measured on flat ground, 1st and 2nd
  always pull away, 3rd stalls the Vireos on light gas and bogs to 400–550 rpm floored,
  4th always stalls. Under `FEED_GAS` (after traction control) `feed` goes back down
  (`FEED_BACK`): without that, traction control cutting the gas against wheelspin
  stalled every launch. Off the gas at a crawl it opens (braking to a stop in gear never
  stalls). It stalls only with a gear in (not hunting at idle), and while shifting the
  ECU's overrun cut does the firing cut, so a gear going in from idle doesn't stall it.
  No automatic restart: the ignition (I), with a hint.
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
  stops; it needs the ignition (I) once fuel comes back. Cars
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
  temperature, heat to coolant, peak pressure, all by rpm and load, per cam profile
  (unphased; `LoadMaps`, tables `mbt0`, `mbt1`, …, since 2026-11-27, the user's wish);
  friction by rpm (and oil
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
- **Valvetrain kinds** (2026-11-27, `docs/pushrod-v10.md` milestone A):
  `EngineHardware::valvetrain` (`ValveLayout`: `CamDrive` Ohv/Sohc/Dohc, 2–4 valves, the
  valves' diameters relative to the bore; in the fingerprint) is what `cycle.rs` breathes
  through; `ValveTrain` has `rocker_ratio`, `cam_side_mass` (lifter and pushrod, at the
  valve ÷ ratio², `moving_mass`) and `rollers`. With one cam (Ohv, Sohc) `Parts::hardware`
  retards the exhaust by minus the intake advance and the dyno shows one knob. The
  valvetrain friction is live (`spring_drag`: force × valves/4, rollers half), so the
  4-valve engines' maps stayed the same. Phasers and switching cams need
  `items::OVERHEAD_CAMS`, provided by long blocks not `Ohv`. A block's head is data
  (`catalogue::engines::Head`: layout, rocker ratio, cam side mass, rollers, lift
  relative to the bore; `TWIN_CAM` for the B series, `PUSHROD` 1.6:1, 0.2 kg, rollers,
  0.125). The factory idle speed follows the cams' overlap (duration − 2 × separation,
  past 20°: 12.5 rpm a degree), not the duration. Engine wear per cylinder covers 12
  (`damage::MAX_CYLINDERS`, protocol 50).
- **The R-series V10** (2026-11-27, milestone B): R80 "8.0 V10", 90°, two valves on
  pushrods, 101.6 × 98.6, CR 9.1, runners 400 mm, headers 500, flow 0.95, a 276° cam on
  a 130° lobe separation (the model's cams are abstract: a mild 250°/114° cam made ~760
  N·m, 95 N·m/l; only a late intake closing brings the low end down, and the wide
  separation keeps the overlap, so the idle, low), springs made for 6000 (398 N seat,
  float 6380 rpm), flywheel 0.35; 545 N·m at 1500, ~637 N·m at 3900, 295 kW at 4500,
  peak 305 kW at 5100, idle 850. 0.86 × 0.78 × 0.60 m, 300 kg (its long block item
  `r80`, 210 kg, comes from the family like every version's; `engine.r80` from the
  Merrow's trim). `the_pushrod_v10_pulls_low_and_broad`. The NA torque range in
  `every_factory_engine_idles_and_pulls` starts at 75 N·m/l for it. Its layout is
  `Layout::OddFireV10` (milestone D, protocol 52).
- ECUs are switchable (agreed with the user): `EcuModel` (Stock, Programmable) is chosen
  in the garage like a part; the stock one stays. One `Ecu` struct for all models, the
  model deciding which features it uses (cam control lives in every `Ecu` but only the
  programmable one drives it, and only it writes the cam settings and maps to tunes).
  Tunes record `ecu <model>`; loading keeps the fitted model and warns. Programmable:
  cam switch-over (rpm and MAP, hysteresis) and intake/exhaust phase maps; its factory
  tune switches where the second profile breathes better all the way to the redline,
  and phases for the most air from 45 to 75 kPa up (at rest below, for the idle).
  The second profile has fuel, λ and spark maps of its own (`CamControl::ve`, `lambda`,
  `spark`, tune tables `*_high`, `Ecu::maps_on`; the tuning's "Cam" choice); saved cars
  without them get the first profile's.
  Parts: a switching cam (236°/264°) and phasers (intake 40°, exhaust 30°); with the
  stock ECU they stay on the first profile, at rest. Phase axes have 3 points and the
  ratio axis is coarser with phasers (bake: intake phaser about 2 s, both 6 s, with the
  switching cam 13 s on 4 cores).
- Next ideas: closed loop (wideband sensor), knock sensor/control, boost control,
  launch/anti-lag; damage to the clutch and gearbox; misfire sound for dead cylinders.
