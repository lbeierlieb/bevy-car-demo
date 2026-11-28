# A second engine family, three new cars, and drivetrains made of parts (planned with the user, 2026-11-28)

The user asked for counterparts of Honda's EP Civic, 7th-generation Accord and Element,
on a new engine family that works like Honda's K series. The car names below are made
up. The engine codes copy Honda's style, as the B series already does. The user also
wants:

- the hot 2.0's head on the 2.4 block (the "Frankenstein" swap),
- the rear axle and its differential (all-wheel drive) to be swappable into the hatch
  and the saloon,
- drive shafts that can limit high-power builds,
- an aftermarket rear subframe that carries the engine at the rear axle,
- gearboxes that don't cross between engine families where that isn't realistic.

## The lineup

| Trim | Chassis | Engine | Gearbox | Drive, differentials |
|---|---|---|---|---|
| `tessel.2-0` | Tessel, 3-door hatch (EP Civic) | K20 | K 5-speed | front, open |
| `tessel.2-0r` | Tessel | K20R | K 6-speed close ratio | front, limited slip |
| `aldine.2-0` | Aldine, saloon (7th-gen Accord) | K20 | K 5-speed (long) | front, open |
| `aldine.2-4` | Aldine | K24 | K 5-speed (long) | front, open |
| `bothy.2-4` | Bothy, tall box (Element) | K24 | K 5-speed (short) | front, open |
| `bothy.2-4-awd` | Bothy | K24 | K 5-speed AWD (transfer output) | all four; front and rear open, a reactive coupling in the centre |

Engines, the K family (a new `Family` in `catalogue/engines.rs`, DOHC 16 valves). Only
the K20R switches cams and phases its intake cam; the K20 and K24 have fixed cams and no
phaser (the user's decision):

| Code | Like | Bore × stroke | Compression | Cams | Power, limit (about) |
|---|---|---|---|---|---|
| K20 | K20A3 | 86 × 86 | 9.8 | fixed, mild | 118 kW at 6500, 6900 rpm |
| K20R | K20A (Type R) | 86 × 86 | 11.5 | switching on both cams, intake phaser, wild high cam, head that flows | 158 kW at 8000, 8600 rpm |
| K24 | K24A | 87 × 99 | 9.7 | fixed, mild | 120 kW at 5800, 6500 rpm, much more torque |

Frankenstein: the K20R head on the K24 block makes more power than either, as the B16R
head does on the B20. The K24's rods (long stroke) cap the revs until forged rods go in,
and the engine needs a programmable ECU that switches cams and phases them. All of this already exists
for the B series, so it is data plus baked maps.

## Gearboxes: bellhousings instead of "adapter plates for anything"

Today any gearbox of the chassis' kind goes on any engine, and a pairing no factory car
has only shows "needs an adapter plate". Plan:

- `Family::bellhousing` and `Gearbox::bellhousing` (a pattern name). A gearbox bolts to
  an engine of the same pattern.
- Transverse gearboxes never cross families. Older Hondas turn the "wrong way"
  (anticlockwise); the K series turns the usual way. A B gearbox on a K engine would
  give five reverse gears and one forward, so no adapter is offered.
- Inline gearboxes (longitudinal) are more lenient: adapter plates are items for listed
  pairs (`AdapterDef { gearbox pattern, engine pattern }`), bought at the engine shop.
  For now only the R80's inline 6-speed has one, to the K series, for a K in the Merrow.
- The K gearboxes: 5-speed (Tessel), 5-speed long (Aldine), 5-speed short (Bothy),
  5-speed AWD (Bothy, with a transfer output to a prop shaft), 6-speed close ratio
  (Tessel R). Ratios will be close to the real cars' and tuned in that step. The two B
  gearboxes become B only. Saved cars keep working, because every factory pairing stays
  valid.

## Drivetrains made of parts

Today `Chassis::drive` and one `CarBuild::limited_slip` are fixed per chassis. They
become something worked out from what's fitted:

- **Gearbox**: transverse FWD, transverse AWD (transfer output), inline, transaxle. Its
  slots stay as they are (gear set, final drive, differential, which is the front one for
  a transverse gearbox). The AWD gearbox also has a **centre coupling** slot: the
  reactive coupling is stock (front drive until the fronts slip, then torque goes to the
  rear, like the real car's pump coupling); an open centre and a viscous one are parts.
- **Rear subframe** (new required chassis slot): a plain one (no driven axle; what the
  Tessel, Aldine, front-drive Bothy and Vireo have from the factory), or one with a
  **differential mount** (the AWD Bothy's) that has its own slots: rear differential
  (open, limited slip) and rear drive shafts.
- **Prop shaft** (chassis slot, needs a tunnel): links the transfer output to the rear
  differential. Only the Bothy's floor has a tunnel from the factory. The Tessel and Aldine
  take a **tunnel kit** (an aftermarket part, fitted at the workshop, that gives the
  floor room for a prop shaft). The Vireo can't take one (the user's decision).
- **Drive** follows from that: front if the gearbox drives the front; rear too if a
  transfer output, prop shaft and rear differential are all fitted; the split from the
  centre coupling. Fitting refuses half-done combinations, or shows "rear axle not
  driven" (fitting's existing rules for required slots).
- Physics: `EngineSetup::differential` becomes one per axle (`[Differential; 2]`).
  `limited_slip` goes from `CarBuild` (each differential item says what it is). No new
  sim state is expected. Protocol bump, and saved cars are converted: they get their
  chassis' factory rear subframe; the old `limited_slip` becomes the gearbox's
  differential item, as now.
- AWD conversion for the Tessel and Aldine (aftermarket): the tunnel kit, the AWD gearbox,
  and the Bothy's rear subframe re-made for their pickup points (`tessel.rear-subframe.awd`, …), prop
  shaft and rear differential. Limited-slip swaps for every differential at tuning
  shops.

### Drive shafts

Recommended, as a later step: a **pair of drive shafts** per driven axle (front: in the
car's slots; rear: in the rear subframe), and the prop shaft, each with a torque rating
like `Gearbox::torque`. Over it they wear, and a worn-out one breaks: that axle no longer
drives (sim state, in `snapshot!`), and a hint says so. Stock shafts hold the factory
engine with margin; stronger ones come from tuning shops. Until this step, shafts are
items with no limit (or don't exist yet), so steps 4 and 5 can ship apart.

## Steps

1. **K family**: versions K20, K20R, K24; their heads (K20 / K24 fixed cams, K20R
   switching with an intake phaser); stock ECUs (locked; the K20R's switches and phases); baked maps (new files in
   `engine/baked/`, `BAKED`); stock engines tested against the table above, and the
   Frankenstein (more power, rods' rev cap, refused by a stock ECU). Engine shop items.

   **Done** (2026-11-28). Heads carry their own cam phasers now (`HeadDef::intake_phasing`,
   `exhaust_phasing`, from `Block::phaser`; protocol 66), so the K20R's phaser goes with
   its head onto the K24. The stock ECU model needed nothing more: `Ecu::calibrated`
   already phased for a model with `phasers`. As measured (`the_k_series_is_what_the_table_says`,
   `the_k20r_head_on_the_k24_block_pays`, each with its own locked ECU):

   | Code | Torque | Power | Limit | Made of |
   |---|---|---|---|---|
   | K20 | 211 N·m | 115 kW at 6500 | 6900 | 236°/112° cams, runners 430 mm, flow 1.1, `Head::TWIN_CAM` |
   | K20R | 229 N·m | 158 kW at 8500 | 8600 | 224°/112° (lift 0.095 bore, `Head::K_R`) and 236°/110° × 1.3 lift, switched at about 5500; intake phaser 50°; runners 440, flow 1.4; long-tube headers (960 mm) from the factory (`preset.manifold`) |
   | K24 | 245 N·m | 118 kW at 5500 | 6500 | as the K20, runners 450 |

   The K24 block with the K20R head: 271 N·m and 145 kW at 6500, held to 7000 by the
   K24's rods; with forged rods, short runners and long-tube headers 248 N·m, 164 kW at
   7500 (limit 8500); with race pistons (back to 11.7:1) 174 kW. The K20R's stock ECU
   runs it lean (2-litre maps), the K24's never switches cams: a programmable ECU it is.
   Decided while building: in the model a wilder high cam pays only with long headers
   (with the 650 mm cast manifold the mild cam breathes better at every speed), hence
   the K20R's factory 4-2-1; and the swap's gain over the K20R is small in power (the
   same valves choke a bigger engine sooner, and the long stroke's friction grows), so
   the K20R's runners are long enough to keep it at the table's power. The family's
   items (`k20`, `k20r.head`, `ecu.k24`, `k.headers.long-tube`, …) come from the family
   like the B series'; the shops sell them (`assets/shops.cfg`), but no `engine.k*`
   until a trim has one (step 3). Found on the way: the B16R's stock ECU never switches
   (its wild cam breathes better nowhere below its redline, `switch_rpm` 9500); fixed
   afterwards (2026-11-28, see `docs/vireo-lineup.md`): it switches at 5700.

   **Since cast manifolds reflect less and lose at their merge** (2026-11-28, see "How a
   manifold is made" in `docs/exhaust-manifolds.md`): K20 211 N·m, 119 kW at 6500; K24
   242 N·m, 122 kW at 5500; the K20R's wild cam pays on cast manifolds too, so it has a
   cast 4-2-1 from the factory like the real one (`k.manifold.4-2-1`, 850 mm, place 4)
   and 400 mm runners: 225 N·m, 160 kW at 7500, switching at 5900 (long tubes 163 kW).
   The K24 with the K20R head: 260 N·m, 152 kW at 6500; built as above 164 kW, race
   pistons 174. The table above is as it was built.
2. **Bellhousings and K gearboxes**: compatibility from data, adapter plates as items
   (inline only), the five K gearboxes; the derived `needs_adapter` goes.

   **Done** (2026-11-28; protocol 67, 68). `Family::bellhousing` and `Gearbox::bellhousing`
   are pattern names (`B-series`, `K-series`, `R-series`); `Catalogue::bolting` gives
   `Direct`, `Adapter(a)` or `Never`. Adapter plates are `Catalogue::adapters`
   (`AdapterDef { id, name, gearbox, engine, mass }`), items of `Kind::Adapter` that go
   into an optional "Adapter plate" slot on the gearbox (only gearboxes that aren't
   transverse have it): the plate bolts to the gearbox's face and stays with it when the
   engine is swapped, the usual case for a swap. The one plate, `adapter-plate.k-to-r`, is
   sold at the engine shop, which now fits plates (as it swaps engines). The Merrow's bay
   went from 0.62 to 0.70 m tall so a K (0.68 m) fits at all. Fitting refuses a gearbox
   that doesn't bolt (`items::drivetrain_misfit`, with a `Fix`: fit plate `a`, or take
   the plate off when the two bolt straight together); the shop window shows the reason
   on a new part and offers it together with the plate (or the plate off) in one request,
   since either alone is refused. No saved car needed converting: every factory pairing is
   direct, and no other pairing could be built before (the B didn't fit the Merrow's bay,
   the V10 not the Vireo's, and no K engine is sold whole yet).

   The K gearboxes (transverse, `k_gearbox` in `catalogue/chassis.rs`; reverse 3.58;
   final drives stay the chassis', for step 3: about 4.29 Tessel, 4.76 Tessel R, 4.44
   Aldine, 4.76 Bothy):

   | Gearbox | Gears | Mass | Takes (N·m) |
   |---|---|---|---|
   | K 5-speed (Tessel; EP3 Si) | 3.27 1.88 1.21 0.92 0.74 | 38 kg | 250 |
   | K 5-speed long (Aldine; Accord 2.4) | 3.27 1.77 1.21 0.92 0.71 | 40 kg | 290 |
   | K 5-speed short (Bothy; Element) | 3.53 2.04 1.39 1.03 0.78 | 40 kg | 290 |
   | K 5-speed AWD (Bothy AWD) | as the short one, `transfer: true` | 52 kg | 290 |
   | K 6-speed close-ratio (Tessel R; EP3 Type R) | 3.27 2.13 1.52 1.15 0.92 0.74 | 44 kg | 320 (clutch 300), shifts 0.22 s |

   The AWD one is only marked: `Gearbox::transfer` is a flag nothing reads until step 4
   (which may make it a gearbox kind of its own and add the centre coupling slot). Car
   workshops sell the 5-speeds, tuning shops the 6-speed. The two B gearboxes keep their
   names and ids (saved cars); the shop window shows a gearbox's bellhousing.
3. **The three chassis and six trims**: bodies (`body.rs` outlines: short hatch, saloon,
   tall box with a split rear hatch), bays (all transverse, front), masses (about
   1200, 1350, 1550 kg; AWD +70 kg), tanks, wheels, anti-roll bars, holds, body slots,
   showroom and traffic weights. The AWD Bothy comes in step 4. Until then it ships front
   drive only.

   **Done** (2026-11-28; protocol 69). Tessel (3-door, 4.15 m), Aldine (saloon, 4.80 m, rear
   doors, boot), Bothy (tall box, 1.79 m, rear doors, a tailgate split at the belt); details
   and numbers in `src/catalogue/CLAUDE.md`, "The K cars". As built:

   | Trim | Mass full | Front | 0–100 | Top |
   |---|---|---|---|---|
   | `tessel.2-0` | 1194 kg | 62 % | 7.4 s | 223 km/h |
   | `tessel.2-0r` | 1208 kg | 63 % | 6.2 s | 244 km/h |
   | `aldine.2-0` | 1356 kg | 62 % | 7.5 s | 222 km/h |
   | `aldine.2-4` | 1361 kg | 62 % | 7.6 s | 226 km/h |
   | `bothy.2-4` | 1562 kg | 60 % | 8.5 s | 187 km/h |

   Decided while building: bodies got rear doors (`Panel::RearDoor`, `Body::rear_door`) as
   parts of their own, and a saloon's rear window belongs to the shell; the Bothy's split
   tailgate is one part (drawn split); trims may have their own gearbox and final drive
   (the Tessel R's 6-speed and 4.76); the final drives as suggested (the 2.4 Aldine's 4.44
   too: a 4.25 made it no quicker); the three's brakes are the hatch's kinds scaled for their
   weight, with a large vented front disc for the Tessel R; the Vireo's bay is 0.68 m tall
   now so a K swaps in (only what fits changed). The sim is optimistic about acceleration
   for every car (the K20 cars run 0–100 a second or two quicker than the real ones). The
   Bothy's tunnel isn't modelled until step 4.
4. **Drivetrain parts**: rear subframe, rear differential, prop shaft, centre coupling,
   `Drive` derived, a differential per axle; the AWD Bothy trim; the tunnel kit and AWD
   conversions for the Tessel and Aldine; limited-slip swaps front and rear.

   **Done** (2026-11-28; protocol 70). As planned, with these decisions (details in
   `src/catalogue/CLAUDE.md`, "Drivetrains made of parts"):
   - Half-done builds: a piece missing on the way to the rear (centre coupling, prop
     shaft, the mount's differential or drive shafts) leaves the rear axle undriven, as it
     honestly is; a prop shaft fitted with nothing to turn it or nothing to drive, or a car
     that drives no wheels, is refused with the reason. A conversion goes in any order
     ending with the prop shaft (or in one request); undone by taking the prop shaft off
     first. A new AWD gearbox comes without a coupling.
   - The Merrow: its inline gearbox lost its differential slot; its factory rear subframe
     (`merrow.rear-subframe`) has the mount with the limited slip and drive shafts, and a
     prop shaft runs along its factory tunnel. It drives exactly as before (all stock cars
     were checked to the bit against the commit before: the factory rear parts' masses are
     in the chassis' mass, other parts count their difference).
   - The tunnel: `Chassis::tunnel` (`Never` Vireo, `Kit` Tessel and Aldine, `Factory`
     Bothy and Merrow); the prop shaft needs what a factory tunnel or the tunnel kit
     provides; tunnel kits are fitted only at car workshops.
   - The reactive coupling fits `Differential { coupling, limit }` with `Drive::All {
     front: 1.0 }`; no physics change was needed beyond a differential per axle (one
     driven only through the coupling has its differential working too). Tuned to 250
     N·m·s, 1500 N·m: cruising the rears pull about a tenth, floored on a meadow without
     traction control about half. Open centre 50/50; viscous 50/50, 60 / 600.
   - Showrooms list the chassis' trims (two Bothys share the K24).

   | | `bothy.2-4` | `bothy.2-4-awd` |
   |---|---|---|
   | Mass full | 1562 kg | 1630 kg |
   | 0–100 | 8.5 s | 8.4 s |
   | 60 s flat out | 186 km/h | 186 km/h |
   | 4 s from standing, dry | 16.1 m/s | 16.6 m/s |
   | … on a meadow | 7.1 m/s | 11.3 m/s |
   | … one side on grass | 7.2 m/s | 11.0 m/s |

   (With traction control. The Aldine 2.4 converted with limited slips front and rear:
   11.3 m/s on the meadow against 7.8.) Rear drive shafts were plain items until step 5.
5. **Drive shafts** with torque ratings, wear and breaking.

   **Done** (2026-11-28; protocol 71). Details in `src/catalogue/CLAUDE.md`, "Drive shafts".
   Front drive shafts became a required car slot on the transverse chassis (saved cars get
   new stock ones); rear drive shafts (in the mount) and prop shafts got ratings. Each
   comes stock, uprated (1.5×) and race (2.2×; prop shafts stock and uprated), per
   chassis. The sim takes the torque through each shaft (drive torque less what spins the
   locked engine's inertia), so with the clutch locked a shaft carries about what its tire
   grips, and the big loads are clutch dumps (the clutch's capacity through first gear).
   Decided: wear goes with the **square** of the share over the rating (`SHAFT_WEAR` 3/s),
   not linearly like the gears: the shocks last hundredths of a second, linear wear made
   even a race clutch's 170 % overload harmless. Broken: that axle goes undriven (an
   all-wheel-drive car drives the other one, all of the torque, a simplification of the
   coupling slipping), nothing driven leaves the gearbox turning freely and the car
   stranded (the special rescue takes it); R keeps it broken, like engine damage; a new
   shaft at a workshop mends it. Ratings (N·m per shaft; prop shaft before the final
   drive) against the most measured floored from standing, with and without traction
   control, asphalt and meadow, 8 s through the gears:

   | Chassis | Front | Rear | Prop | Most measured (trim) | Clutch dumped |
   |---|---|---|---|---|---|
   | Vireo | 1400 | – | – | 1134 (2.0) | 1512 |
   | Merrow | – | 2600 | 1700 | rear 2133, prop 1389 | 2947, 1915 |
   | Tessel | 1850 | 1850 | 925 | 1524 (2.0R) | 2336 |
   | Aldine | 1800 | 1800 | 900 | 1462 (2.4) | 2105 |
   | Bothy | 2100 | 2100 | 1050 | 1705 (2.4); AWD rear 750, prop 315 | 2436 (AWD 3186) |

   A stock clutch dumped wears its shafts at most about 1 % (Tessel R); the turbo Vireo
   with race tires and limited slip breaks its stock fronts in 5 dumps with a race clutch
   (a sport clutch ~5 % each), the AWD Bothy with a race clutch in 4, then drives on its
   rears. Floored with the clutch locked even the power build only reaches about 1.6 kN·m
   (tire-limited), so launches break shafts through the clutch, not the engine's torque.
6. **Rear-engine subframe** (aftermarket, planned in detail when we get there): a rear
   subframe that carries an engine and a transverse gearbox at the rear axle, like the
   real mid-engine Civic swaps. Engine bays then come from subframes instead of one
   `Chassis::bay`: the rear one takes the rear bench's place and part of the hold, the
   front axle goes undriven, and the masses, the engine's drawn place, the sound source,
   the exhaust exit and the radiator lines follow. One engine per car. It touches
   physics, looks, audio and fitting, so it is the largest step and is kept last.

   **The plan** (2026-11-28, before building):
   - **The part**: `<chassis>.rear-subframe.engine`, "Rear subframe with an engine bay",
     for the Tessel, the Aldine and the Vireo (the classic mid-engine swap is on the
     hatch the Vireo stands for; it needs no tunnel). Not for the Bothy (its rear is the
     flat load floor under the tall tailgate, and it has all-wheel drive for traction) nor
     the Merrow (no rear bench, its engine already sits behind the front axle). It goes in
     the existing "Rear subframe" slot: a new `Does::EngineSubframe(EngineBay)` of
     `Kind::RearSubframe` with three optional slots of its own: "Engine", "Gearbox" and
     "Drive shafts". Mass about 45 kg (frame, engine mounts, a firewall behind the seats,
     the shift cables and the coolant lines with their coolant: the radiator stays at the
     front), counted at the rear axle like every rear subframe's difference.
   - **Engine bays from where the engine is**: the chassis' `bay` stays the front one;
     the subframe's bay is the rear one ("middle, transverse": the same transverse
     orientation, the engine and gearbox as far ahead of the rear axle as the factory's
     sit ahead of the front axle, since the whole front-drive unit is moved back as it is:
     a gearbox turned round would drive the car backwards; about 0.95 m long, the front
     bay's width, header room 1.1 m so the K20R's long tubes still fit and no factory
     engine needs a bake there). `CarBuild::rear_bay` (the subframe's definition, when the
     engine is in it; on the wire) and `CarBuild::bay()` give the bay; `Chassis::fit_in`
     checks an engine against any bay (`install` uses the car's).
   - **Where the engine is** (`items::powertrain`): in the subframe if it holds an
     engine, else the car. `resolve`, `driveline`, `drivetrain_misfit`, `assemble`, the
     masses, the looks and the shops look there. Exactly one engine: rules
     (`items::bay_problem`, checked by `resolve`, so fitting and reading cars refuse
     them): a second engine (front and rear), the gearbox not beside the engine, front
     drive shafts or a prop shaft with the engine at the rear (nothing turns them), the
     rear bench with an engine subframe fitted (its place is the bay's, even empty). An
     empty engine subframe with the engine still at the front is allowed (the car drives
     as before, heavier, without its rear bench). With the engine at the rear the car's
     own "Engine", "Gearbox" and "Front drive shafts" slots aren't required
     (`Item::fine`); the subframe's three are optional, and a rear engine without drive
     shafts drives nothing (refused, like the Merrow without its prop shaft).
   - **Drive**: a transverse gearbox in the engine subframe drives the rear axle through
     its own differential and the subframe's drive shafts (`Drive::Rear`, the rear
     differential the gearbox's); the front axle is undriven, a transfer output unused.
     The front drive shafts are the ones that move to the rear (as in the real swaps,
     which use the front-drive unit's own shafts), so no new shaft parts; their rating is
     the rear axle's then.
   - **Fitting**: a tuning shop does the conversion (they sell the subframe, and whole
     engines are swapped at car and tuning shops in this game, the engine shop does long
     blocks): fitting an engine subframe in place of one without moves the car's engine,
     gearbox and front drive shafts into it and takes the rear bench out (into the trunk
     or held, as anything taken off); fitting a subframe without a bay in place of an
     engine subframe moves them back. One request, as a shop does it
     (`fitting::convert`, also in `with_fitted` for the window's preview). Car workshops
     and engine shops refuse engine subframes in or out; everything else on the engine
     and gearbox in the subframe works as in the front (engines swapped, gearboxes, the
     long block at the engine shop, the dyno). The window lists the subframe's engine,
     gearbox and drive shafts with the car's slots and hides the car's empty ones and the
     rear bench; the subframe's ware says what moves and warns if the car it makes is
     refused. Sold at tuning shops (`assets/shops.cfg`), with the factory rear subframes
     for going back; car workshops' `*.rear-subframe*` lines become exact ids.
   - **Masses**: the engine and gearbox at the rear bay's places (`CarSetup::with_car`:
     the balance and the inertia follow), the subframe's difference at the rear axle, the
     front drive shafts' mass leaves the front axle (−12 kg there, +12 in the subframe),
     the rear bench's mass goes with it. Tank, radiator, battery stay. Expected for the
     Tessel 2.0R: about 45/55 (the engine and gearbox ahead of the rear axle, as in the
     real kits; the shell's own mass is front-heavy).
   - **Hold**: the bay's length takes the hold's columns there out (`cargo::grid`): the
     rear bench's place and the front of the trunk (Tessel, Vireo); the Aldine keeps its
     boot.
   - **Looks and sound**: `mechanicals` draws the engine, intake, headers and gearbox in
     the rear bay (exhaust side to the back, as at the front), a firewall behind the
     seats, coolant lines along the floor from the radiator, and a short exhaust from the
     collector under the rear bumper to the tip (no front pipe past the tank). Seen with
     the hatch off (`UNFIT=Hatch`) and through the rear windows. The cabin sound
     (`CabinShape`) takes the car's bay, so the engine right behind the driver comes
     through brighter. The exhaust exit stays at the rear (it was already); night lights,
     spray and the body don't change. Traffic and showrooms only use factory cars.
   - **Saved cars**: nothing saved has one; loading a rear-engined car mustn't give it
     new front drive shafts (`fill_front_shafts` skips it). Protocol 72 (a new `Does`,
     `CarBuild::rear_bay`).
   - **Tests**: the Tessel 2.0R converted at a tuning shop (fits; rear drive; balance
     rearward; refused: a second engine, the gearbox left at the front, front shafts at
     the front, a rear bench, a car workshop doing it; the conversion undone at a
     tuning shop with the factory subframe; a saved round trip; 0–100 and pulling
     away), stock cars bit for bit as before (fingerprint compared), the hold smaller,
     drawn at the rear; the power-slide bot on it.

   **Done** (2026-11-28; protocol 72), as planned. Details in `src/catalogue/CLAUDE.md`,
   "The rear-engine subframe". Measured (`the_hatch_gets_its_engine_behind_the_seats`,
   `the_mid_engined_hatch_pulls_away`, `power_slides`):

   | | `tessel.2-0r` | mid-engined |
   |---|---|---|
   | Mass full | 1208 kg | 1217 kg |
   | Front | 62.7 % | 47.8 % |
   | 0–100 | 6.2 s | 5.9 s |
   | Power slide, 2nd / 3rd, no TC | none (front drive) | spun / 13° caught (snaps back 11°) |
   | … with TC | none | 8.4° / 10.5°, caught |

   Decided while building: the balance comes out about 48/52, not the 40/60 the real
   kits are quoted at: the front-drive unit keeps its place ahead of the axle (turned
   round it would drive backwards), the shell's own mass stays front-heavy (radiator,
   battery, front structure), and nothing else moves (real builds also move the battery
   and lighten the front); left as the model says. The rear bench comes out when the
   subframe goes in (into the trunk if there's room, else held at the shop); the Tessel's
   shrunken hold rarely has room. `bay_problem` is checked before `Item::fine` in
   `Bench::finish`, so a refused build says why. Shots: `FITIDS="tessel.rear-subframe.engine"`
   fits it as a shop does (`OFFLINE=1 TRIM=2,1`, `UNFIT=Hatch CARCAM="-1.4,2.5,-3.4,-1.0"`
   looks into the bay). Not done: the tank under the bay isn't moved (drawn overlapping
   the engine's sump a little), traffic never has one, the dyno untried with it by hand
   (it goes by the drive's shares, so the rear rollers turn).

## Assumptions (agreed with the user, 2026-11-28)

- Names: Tessel (hatch), Aldine (saloon), Bothy (box). Engine codes K20, K20R, K24, in
  the style of B16/B16R.
- The Accord's 2.0 is the Civic's K20, so there's one engine (fewer maps to bake), not a
  separate K20A6.
- The AWD Bothy's stock centre is the reactive coupling (it fits `Differential {
  coupling, limit }` with the whole share at the front; still to check in the sim). Front
  and rear axle differentials are open.
- Any K head fits any K block; compression stays the block's (as with the B series).
- The same double-A-arm suspension stays for every car and subframe: a subframe changes
  what is driven and where mass sits, not the geometry.
- Swap subframes for engines stay as they are now (derived, only shown).
- Gearbox ratios and masses are close to the real cars', tuned by measurement, not
  copied exactly.
- All six steps are built in order, each by its own subagent, one after the other.
