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

## Sandbox workflow

- Build inside the dev shell with a separate target dir (the shared `target/` broke):
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo clippy --all-targets -j 4 -- -D warnings'`.
  Touch `src/main.rs` first to make clippy actually re-check. Wasm:
  `cargo clippy --target wasm32-unknown-unknown -j 4 -- -D warnings`.
- `cargo run` in the dev shell is a wrapper for `--profile release-dev`; build that
  profile, not `dev`, before screenshots.
- No python/perl installed: `nix shell nixpkgs#python3 -c python3 -I`.
- 4 cores, 7.8 GB RAM, no swap: a full `nix flake check` got OOM-killed once.
- Anything that copies the repo flake into the Nix store (`nix build .#…`,
  `builtins.getFlake`) fails with "No space left on device": `target/` is over 20 GB.
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
- Terrain bake tool: `cd tools/bake; nix develop /repo -c cargo build --release; ./target/release/bake …`.
- Engine maps of every factory engine (22, see `families.rs`) are baked into
  `src/physics/engine/baked/<code>.maps`. After changing `cycle.rs`, `maps.rs` (outside
  their tests; any edit counts, their text is in the fingerprint) or a factory engine's
  cylinder side, re-bake all of them (about 40 s, 1.5 MB of text changes) or
  `stock_maps_are_current` and the tests that expect baked maps fail:
  `nix develop -c bash -c 'CARGO_TARGET_DIR=/repo/target/sandbox cargo test --profile release-dev bake_stock_maps -- --ignored'`.
  A new factory version needs an (empty) file there first, for `include_str!`.

### Screenshots (`tools/dev`)

`tools/dev/unstrip.sh` copies `tools/dev/shot.rs` into `src/` and hooks it into
`main.rs`; `tools/dev/shot.sh out.png [frames] [keys]` renders headless and saves a
screenshot; `tools/dev/strip.sh` removes the hook. Always run `strip.sh` and check that
`grep -c shot src/main.rs` prints 0 before committing. Keys are pressed from a third of
the frames until the shot (`k` opens the map, `2` hides the panel, `3` the ECU window,
`4` the garage, `5` the dyno, `6` the showroom, `f` the gizmos, `c` the driver's view),
so clicks on a window must come after that. `CLICK="x,y,frame;x,y,frame…"` clicks into the
1280×720 window, e.g. on the map. `CAM`/`CAMAT` env vars place the camera (see
`shot.rs`). `shot.sh` fetches Mesa and Xvfb from the pinned nixpkgs on first use.
A shot takes about 35 s.

For looking at cars: `CAR="chassis[,engine]"` starts in that car (its place in
`CHASSIS`); `LINEUP="first,count"` parks that many cars one behind the other on an open,
flat straight (the spawn is in a village, where walls and slopes block the view);
`CARCAM="side,height,along"` looks at them from that far to their right (negative:
left, the sunlit side), up, and ahead. Side views of five:
`LINEUP=0,5 CARCAM="-21,2,0" bash tools/dev/shot.sh out.png 60 f2`; one car close up:
`LINEUP=3,1 CARCAM="-3.6,1.9,4.6"`; from the driver's seat: keys `f2c`.

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

## Wheel support

Wheel and pedals come through gamepad input (gilrs), bound in the panel (F2, "Wheel and
pedals") and saved to a config file. `src/wheel_base.rs` sets the base's rotation range
and plays force feedback, both through hid-fanatecff on Linux.

### Design decisions (agreed with the user)

- Target: Linux with hid-fanatecff. Force feedback only there; input works everywhere.
- Force feedback: self-aligning torque only. Damping/friction from the base's tuning
  menu (evdev 0.13 can't send damper parameters). Rate-limited, fades in with speed.
- The game sets the base's rotation range (default 540°) and restores it on exit;
  either end is full lock. If it can't, soft lock like BeamNG: the base's range is read
  from sysfs (readable even when not writable) or given in the panel, the axis is
  scaled to the game's range, and force feedback pushes back past either end (full
  force 10° past; stays on with the tire forces switched off).
- Clutch pedal: engine can stall, ignition restarts it. Without a bound pedal the
  clutch is automatic and the engine never stalls.
- The automatic gearbox ignores the clutch pedal (automatic clutch, no stalling).
- H-shifter: with a clutch pedal bound, a gear only goes in with the clutch pressed
  (otherwise neutral). Without a pedal, the automatic clutch shifts. Paddles shift
  with the automatic clutch even when a pedal is bound.
- Each H gate is a button held while the lever is in it; neutral is "no gate held".

### Still to check on the real wheel

- "Test: push left" in the panel: tick "Reverse direction" if it pushes right (the sign
  convention of hid-fanatecff couldn't be verified without hardware).
- Tune the default force strength (0.5, with 400 N·m of steering torque as full scale;
  tires peak around 300 N·m in the coupé, with 30 mm of caster trail).
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
  suspension, tires) plus its engine's `Parts` (family, version, a part per slot, ECU).
  `CarSetup::new` works everything the simulation needs out of them: the engine's and
  the gearbox's masses sit where the chassis' bay has them, which gives the balance and
  the inertia; springs and dampers come from ride frequencies and the weight on each
  axle (`AxleSetup::tune`); brake torque is a share of the stock tires' grip (about
  0.8, so a floored pedal doesn't lock wheels on a dry road) and its balance follows the
  weight shift. The linkage is the same double A-arm for every car, moved to its track
  and wheel size.
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
- **Showroom (F6) and garage (F4)** are both in `garage.rs`; both go through `start`
  and `fit`. A changed `CarBuild` respawns the car's entity (its meshes are children);
  an engine-only change swaps it in while driving.
- **Traction control** (my addition, on by default, a checkbox in the panel): without
  it full throttle on the keyboard just spins the wheels of anything strong. It eases
  the throttle to hold the driven wheels at a slip ratio of 0.12, at a rate that follows
  the slip smoothly (no chopping; turbos still hunt a little with the boost lag).
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
- Load sensitivity (`TireParams::load_sensitivity`, 0.1) made slides slightly deeper,
  not easier, but makes axle balance matter. Relaxation length wasn't added: at speed it
  lags by only ~20 ms, so it matters only at low speed.
- With the bot, mid- and rear-engined cars still spin (plausible), as does the 6.2 V8
  Harrow floored in second, and the 5-litre V8s snap back; the other front-engined cars
  are caught (asserted by `a_power_slide_can_be_caught`).
- Tire compounds (`tire::COMPOUNDS`, agreed with the user): street (grips from cold,
  best at 65 °C, lets go gently past the peak, overheats when driven hard), sport, race
  (needs heat, best at 95 °C, peaks early and drops off, wears fast). Each has grip,
  peak slip angle/ratio with the Magic Formula C (stiffness B follows from the peak), a
  temperature window and a tread life (sliding energy). One tread lump per corner
  (`Corner::tread`): heated by 0.8 of the sliding power plus rolling hysteresis, cooled
  by the air (time constant about a minute); wear with sliding, faster overheated, a
  quarter less grip worn through. Cars start on cold tires; R gives fresh ones. Near the
  limit on a circle the outside treads reach about 55 °C in 30 s and 95 °C in 2 min
  (`tire_temperatures`, ignored, prints it). A car's tire state survives an engine swap.
  Tests about other things call `Car::warm_tires`. A burnout overheats street tires
  within seconds, and then they can't stop spinning.
- Not tuned by driving yet: the heating rates, the windows, how much grip overheating
  costs, wear rates. Possible next steps: tread and carcass as two lumps, tire pressure,
  camber spreading heat across the tread.

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
- ECU tuning window (F3, `src/ecu_ui.rs`): VE, target λ and spark grids with the
  operating point, block selection and +/-/set, "Correct VE here" (scales the cells
  around the operating point by measured/target λ), rev limiter, idle, factory tune.
  Tunes are saved by name as text (`src/tunes.rs`: `~/.config/car-sim/tunes/*.tune`,
  local storage on the web) with the build they were made for; loading reads over the
  engine's factory tune and warns if the build differs. Swapping the engine build in
  the setup panel replaces the tune with the new build's factory tune.
- Garage (F4, `src/garage.rs`): parts from a catalogue (`parts.rs`: a slot per kind of
  part, each part a function fitting it onto the engine's hardware, relative to it; a
  factory version is a block and a selection), cam-gear timing and wastegate spring. The car's `EngineSetup.parts` says
  what it's built from. Building bakes the maps if the fingerprint changed (native: a
  thread; wasm: 12 ms slices per frame), with the old engine running meanwhile, and caches
  them (`storage.rs` folders: `~/.cache/car-sim/engines/<fingerprint>.maps`, local storage
  on the web); bolt-on parts fit at once. The new engine is new (no damage) and gets a
  factory tune unless unticked. The redline follows from the hardware (`safe_rpm`: 95 %
  of the valve float speed, rods' pull under 70 % of their strength); the factory tune's
  rev limit (first cam profile's with the stock ECU) and the rev counter follow it.
- Engine dyno (F5, `src/dyno.rs`): a copy of the active car's engine (as built, tuned
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

The engine sound goes through bevy_audio, which can't set the output buffer size. An
attempt to open the output with rodio/cpal directly (fixed 4096-frame buffer against
underruns) gave no sound at all in the wasm build and was dropped. If it's tried
again, keep the bevy_audio path on wasm.
