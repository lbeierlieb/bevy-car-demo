# Pushrod engines and a V10 roadster: the plan

Asked for by the user on 2026-11-27: engines with the cam in the block (pushrods,
two valves per cylinder), and a car like an early-90s American V10 roadster with one: a
huge-displacement V10 that sits low (no overhead cams, low heads). Made up like the
Vireo: no real names, badges or exact styling. The real engine's numbers are only
tuning targets. The milestones go in order; each ends with clippy (native and wasm) and
tests passing, rustfmt clean, the game playable, and committed atomically.

The user's answers (2026-11-27): an early-90s open roadster with side pipes; I make up
the names (the car is the **Merrow**, the engine the **R-series V10**, version **R80**,
"8.0 V10"); the odd-fire sound is worth a protocol bump; the car appears in traffic,
rarely.

## What the engine model assumed before

- Four valves per cylinder are hard-coded in `cycle.rs` (`Cylinder::new`: two intake
  valves of 0.36 × bore, two exhaust valves of 0.30 × bore, `count: 2.0 * flow`).
- Intake and exhaust timing (phasers, cam gears) are independent: a cam each.
- `ValveTrain` (spring, moving mass) already gives valve float; the pushrod side only
  needs counting in the mass.
- Engine height is a number (`Family::height`) checked against the bay.
- Friction (`maps.rs`, the bake's `finish`) is a piston-speed correlation without a
  valvetrain term.

## Assumptions

- The Vireo stays a DOHC four; only the R-series is pushrod for now.
- Engine height stays hand-set data, chosen so a pushrod V comes out clearly lower than
  a DOHC one (not derived from deck height and head).
- No cam phasing and no displacement-on-demand for pushrod engines at first.
- Pushrod flex, lifter pump-up and pushrod-specific float aren't modelled; the heavier
  moving mass carries the effect.
- Gearboxes and clutches are matched by the chassis' kind (transverse or inline), as
  before the one-car cut.

## Milestone A: valvetrain kinds in the engine model

1. `EngineHardware::valvetrain` (cylinder side: in the maps' fingerprint): `kind` (`Ohv`,
   `Sohc`, `Dohc`), `valves` per cylinder (2, 3, 4) and the intake/exhaust valve
   diameters relative to the bore (4-valve as now, 0.36/0.30; 2-valve about 0.48/0.39,
   about ⅔ of the curtain area: breathes well low, runs out high). `cycle.rs` reads them
   instead of the hard-coded four.
2. Rocker ratio and pushrod mass: `ValveTrain::rocker_ratio` and `cam_side_mass` (lifter
   and pushrod); the mass the spring sees is `mass + cam_side_mass × ratio²` (OHC: no cam
   side). `made_for` then gives a pushrod engine's redline with sensible springs.
3. One cam: for `Ohv`, the cam gear timing moves both lobes together (advancing the cam
   advances intake and exhaust alike); the dyno's cam screen shows one knob.
4. Parts needing overhead cams (the switching cam, the phasers) need a tag
   `overhead-cams` that OHC long blocks provide (`PartDef::needs`/`provides`). Pushrod
   parts (roller rockers, light pushrods) may come later.
5. Friction gets a small valvetrain term (spring force × valves; roller lifters half of
   flat tappets); OHV comes out a little lower than DOHC.
6. The B16/B18 (and the test engines W20, B16T) maps must come out the same after the
   re-bake (compare before/after). Tests: pushrod float rpm, 2-valve breathing falling
   off above a 4-valve head of the same bore. Protocol bump (the catalogue's engine
   hardware on the wire).

**Built** (2026-11-27). Decided while building it:
- The struct is `hardware::ValveLayout` (`drive: CamDrive`, `valves`, `intake_diameter`,
  `exhaust_diameter`; `FOUR_VALVE` the old head, `PUSHROD` 2 valves of 0.48/0.39), next to
  the springs' `ValveTrain` (`valves`), which got `rocker_ratio`, `cam_side_mass` and
  `rollers`. A third valve is an intake. `cycle.rs` rounds the diameters to 1e-4 of the
  bore, so the f32 0.36 is exactly the old f64 literal (bit-identical maps).
- The cam side counts at the valve **divided** by the rocker ratio squared
  (`ValveTrain::moving_mass`): the lifter and pushrod move 1/ratio as far as the valve, so
  their kinetic energy is as from m/ratio² at the valve. The plan's "× ratio²" was the
  wrong way round. `block()` scales the cam side mass like the valve's (zero for now).
- One cam for **SOHC too** (`CamDrive::one_cam`): `Parts::hardware` sets
  `exhaust_timing = -intake_timing` (the cam's advance retards nothing), so the maps and
  the fingerprint see it without special cases in `cycle.rs`; the engine item's exhaust
  knob stays but does nothing. The dyno shows "Cam advance" alone (`Parts::one_cam`, from
  the version's block).
- The valvetrain friction is **live, not baked**: the existing `SPRING_FRICTION` term in
  `Engine::cycle` now takes `EngineHardware::spring_drag` (spring force at full lift ×
  valves/4, × 0.5 with rollers) instead of the plain spring force, so the maps needed no
  change and 4-valve flat-tappet engines get exactly the old value. A 2-valve roller OHV
  with the same springs drags a quarter of the force (less friction: about 5 kPa of FMEP
  at stock-like springs).
- The tag is `items::OVERHEAD_CAMS` ("head with overhead cams"): every long block whose
  `drive` isn't `Ohv` provides it; parts with a `Switching` effect or changing
  `IntakePhasing`/`ExhaustPhasing` need it (`catalogue/items.rs`).
- Wire: `ValveLayout` checked by `fine` (2–4 valves, diameters 0.15–0.6 of the bore),
  `ValveTrain` by mass > 0, cam side ≥ 0, rocker ratio 0.5–3. Protocol 49.
- Re-baked B16, B18, W20, B16T: every table identical, only the fingerprints changed.
- Tests: `hardware::tests` (pushrod float < 0.85 of OHC with 0.15 kg cam side at 1.6:1;
  spring drag), `cycle::tests::two_big_valves_breathe_well_low_and_run_out_high` (the
  `PUSHROD` head on the W20: over 93 % of the four valves' VE at 2500 rpm, at least 5
  points less of it at 7000), `parts::tests::one_cam_in_the_block_moves_both_rows_of_valves`,
  `fitting::tests::phasers_and_switching_cams_need_overhead_cams`.
- For milestone B: `block()` still takes the reference's valvetrain; it needs a
  `ValveLayout` (and rocker ratio, cam side mass, rollers) per `Block`, and `made_for`
  then sizes the springs against the heavier moving mass by itself. `port_flow` scales
  the valves' and ports' areas whatever the layout.

## Milestone B: the R-series V10

- 90° V10, two valves, one cam in the block, 8.0 L: bore 101.6 mm, stroke 98.6 mm, CR
  9.1; long intake runners (~450 mm), short cast manifolds (headers ~ 400–600 mm), a
  mild cam (~250°, separation 114), redline 6000, heavy flywheel (~0.35 kg·m²).
- Target about 630 N·m at 3600 rpm and 300 kW at 4600 rpm (tune `port_flow`, the cam).
- Size about 0.86 m long, 0.78 m wide, 0.60 m high (lower than the old DOHC 5.2 V10's
  0.64 for 2.8 L more), ~300 kg (aluminium block).
- `catalogue::engines::block()` takes the valvetrain (it scales from the 4-valve W20).
- Baked into `engine/baked/R80.maps`, a line in `BAKED`. Not yet in any chassis (it
  doesn't fit the Vireo); a test checks its curve against the targets.

**Built** (2026-11-27). Decided while building it:
- `catalogue::engines::Head` per `Block`: the valve layout, rocker ratio, cam side mass,
  rollers and the lift relative to the bore (0.11 for the B series, as before: their maps
  stayed the same). The valve's own mass scales with the intake valve's diameter too.
  The R80's: `ValveLayout::PUSHROD`, 1.6:1, 0.2 kg lifter and pushrod, hydraulic rollers,
  lift 0.125 × bore (12.7 mm). `made_for` gave 398 N seat force, 37.4 kN/m, float at
  6380 rpm; rods 43.5 kN.
- **The cam isn't the plan's.** With a mild 250° cam on 114° the model makes ~760 N·m
  (95 N·m/l) and its power falls off early: port flow cuts the top, not the low end, and
  a slower burn for two-valve heads (tried: 25 % longer) took only 2 %. Only a late intake
  closing lowers the low end, so the R80 has a 276° cam on a 130° lobe separation (the
  model's durations are abstract, at zero lift), runners 400 mm, headers 500, port flow
  0.95. The curve: 545 N·m at 1500, 573 at 2100, 631 at 3500, ~637 at 3900, 625 at 4500
  (295 kW), peak 305 kW at 5100, 473 N·m at 5900. Torque peaks at 3900, not 3600, and power
  at 5100 (but 295–300 kW from 4500).
- The factory idle speed followed the duration (850 + 12.5 per degree over 240), which
  put this cam at 1300 rpm; it follows the overlap now (duration − 2 × separation, 12.5
  rpm per degree over 20): the B16 and B18 idle as before, a regrind's idle rises a little
  more (sport cam +250 instead of +200, race +550 instead of +450), the R80 idles at 850.
- `every_factory_engine_idles_and_pulls` allows naturally aspirated engines from 75 N·m/l
  (the R80 makes 80).
- Engine wear was per cylinder for at most 8 cylinders (`damage::MAX_CYLINDERS`): now 12
  (and `items::MOST_WEAR` 12), protocol 50.
- The R80's long block item (`r80`, 210 kg) comes from the family like every version's;
  a used parts shop selling `*` may offer it. It doesn't fit the Vireo (`install` refuses
  the swap: too long across). No `engine.r80` assembly yet: assemblies come from trims,
  so it appears with the Merrow's trim (milestone C).
- For milestone C: the family is 0.86 long, 0.78 wide, 0.60 high, 300 kg, `Layout::V10`;
  it needs a longitudinal bay at least that big, and a clutch and gearbox for ~640 N·m.
  Its headers are 500 mm: the room `Chassis::fit` gives (the bay's `header_room` times the
  share the bay's width beside the engine leaves, `HEADER_SLACK`) must be at least 0.5 m,
  or the factory headers are cut and the test that factory offerings aren't capped fails.

## Milestone C: the Merrow

- Chassis: front-mid longitudinal engine (behind the front axle), rear-wheel drive,
  limited slip, about 1450 kg; wheelbase 2.44 m, 4.45 m long, 1.92 m wide, ~1.12 m tall;
  17" wheels, 275/40 R17 front and 335/35 R17 rear; two seats, no rear bench; a small
  trunk hold; a big tank.
- A longitudinal 6-speed (`GearboxKind::Inline`, from git history before cb6e578)
  taking ~700 N·m (the 6-speed's 360 would wear its gears in a few pulls); a clutch for
  650 N·m and more.
- Body: an early-90s open roadster: windscreen frame only (an open cabin), long low
  bonnet with a bulge, side pipes ahead of the rear wheels. The same pieces as the Vireo
  (bonnet, bumpers, doors, fenders), body items, a trim and an `engine.r80` assembly in
  `catalogue/items.rs`.
- `mechanicals.rs`: a longitudinal V (two banks, low heads without tall cam covers,
  intake in the valley), driveshaft and rear differential, exhausts out at the sides.

**Built** (2026-11-27). Decided while building it:
- The numbers: 780 kg chassis without seats at 1.42 m behind the front axle, engine at
  0.42 m, gearbox at 1.25 m; 1460 kg with a full 85 L tank (1428 empty), 49.8 % on the
  front; 4.8 kg per kW (305 kW); 0–100 km/h in 4.7 s flat out with traction control and
  the automatic gearbox (`zero_to_hundred`, ignored, prints every stock car; the Vireo
  1.6 8.4 s, 1.8 8.0 s). Track 1.52/1.54 m, CG 0.46 m, ride 1.45/1.6 Hz, bars 28/20 mm
  (rates 200/300 per 20 mm), camber −0.8/−1.0°, pressures 2.1/1.85 bar (what suits the
  tires, `factory_pressures_suit_the_tires`), drag area 0.82, max steer 0.5, final drive
  3.07, eye at 2.68 m, 0.98 m up. Rims 17×9.5 front, 17×11.5 rear (the widest the 1.25"
  rule allows round 275 and 335 mm tires lies under 12").
- Gearbox: one inline box, "6-speed heavy duty (inline)" (2.66/1.78/1.3/1.0/0.74/0.5,
  60 kg, 700 N·m, its stock clutch 720 N·m: the existing clutches are multipliers of
  that). Gearbox part definitions are now made for (`fits`) the chassis whose factory
  gearbox is of their kind, so a shop never offers the Merrow a transverse box.
- The open body uses the pieces as they were: an open car's **first opening is its
  cockpit** (`Body::cockpit`, `cockpit_half_width`): the deck's top is open there like
  under a greenhouse, sills run from the deck's edge in to the opening, and a second
  bulkhead stands behind the seats (trim towards them). So the bonnet (ahead of the
  windscreen's base) and the boot lid (behind the cockpit, `Panel::Hatch`) are pieces
  that come off, and details on them and on the fenders go with them (`piece_at`). A
  spoiler on an open car is a lip on the boot lid's rear edge.
- Two body fields (protocol 51): `bulges` (from x, to x, half-width, rise: a flat-topped
  ridge with the bonnet, rising over its first half) and `side_pipes` (the exhaust leaves
  at the sides ahead of the rear wheels; mechanicals draw it out of each bank's
  collector, past a catalyst each, under the sill to a pipe outside the body with the
  silencer in it).
- Slots follow the body (`catalogue/items.rs`, `layout`): the Merrow's lid is "Boot lid",
  its spoiler "Boot spoiler", and it has no "Rear bench" slot. Its hold: a trunk over the
  rear axle (0.9 × 1.0 × 0.4 m) and the passenger seat's place while it's out.
- Brakes per chassis: the Merrow has its own two (`merrow.brake.front-vented`, 1500 N·m,
  17" rims; `merrow.brake.rear-vented`, 720 N·m), none of the hatch's (the big front kit
  stays the Vireo's). Trims have wheels per axle (`TrimSpec::wheels`). The trim is
  `merrow.r80` ("Merrow R80"), its factory paint "Dull navy".
- Mechanicals: a longitudinal V's crank points forwards (belt at the front); its banks
  are tilted boxes (block, low alloy head, a flat cover, red on a reworked head), runners
  from the plenum in the valley, headers per bank to a collector each, the throttle and
  air box ahead. Longitudinal engines get a bell housing and gearbox case, and
  rear-wheel drive a propeller shaft, differential and half shafts. Turbos and
  superchargers on a V are drawn where the inline engine has them (a V's other bank
  crosses over to the turbo). Seats sit lower when the eyes are low (cushion at most
  0.68 m under them).
- Traffic: weight 0.05 against the Vireo's 3.0 (a placeholder for milestone D).
- Shots: `OFFLINE=1 TRIM="1,0"` drives any stock car (chassis, option); `TRIM=1` is still
  the Vireo 1.8. The Merrow is stock car 2 for `LINEUP`.
- For milestone D: no shop sells `merrow.r80`, its parts (`merrow.*`, the inline
  gearbox) or `engine.r80` yet; the showroom's floor places and `FactoryLooks` already
  take any number of chassis. Sound is still `Layout::V10`. The power slide and
  TC/ABS weren't tried with it; the sport and track springs and the alignment shop's
  ride height work as on the Vireo (the same kits).

## Milestone D: wiring it in

- Shops: a car dealer sells the Merrow's trim, used car dealers too, the showroom's
  places for two chassis, tuning parts that fit it (the shops file's `sells` lines).
- Traffic: rarely (`Chassis::traffic`).
- Sound: a new `Layout::OddFireV10` (90° V10 with a common-pin crank: firing intervals
  alternating 54°/90°), the R-series' layout; protocol bump.
- Handling: 630 N·m at the rear; bring back a power-slide test for this car
  (`a_power_slide_can_be_caught`), check traction control and ABS.
