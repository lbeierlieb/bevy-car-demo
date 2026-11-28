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
   (its wild cam breathes better nowhere below its redline, `switch_rpm` 9500); left as
   it is.
2. **Bellhousings and K gearboxes**: compatibility from data, adapter plates as items
   (inline only), the five K gearboxes; the derived `needs_adapter` goes.
3. **The three chassis and six trims**: bodies (`body.rs` outlines: short hatch, saloon,
   tall box with a split rear hatch), bays (all transverse, front), masses (about
   1200, 1350, 1550 kg; AWD +70 kg), tanks, wheels, anti-roll bars, holds, body slots,
   showroom and traffic weights. The AWD Bothy comes in step 4. Until then it ships front
   drive only.
4. **Drivetrain parts**: rear subframe, rear differential, prop shaft, centre coupling,
   `Drive` derived, a differential per axle; the AWD Bothy trim; the tunnel kit and AWD
   conversions for the Tessel and Aldine; limited-slip swaps front and rear.
5. **Drive shafts** with torque ratings, wear and breaking.
6. **Rear-engine subframe** (aftermarket, planned in detail when we get there): a rear
   subframe that carries an engine and a transverse gearbox at the rear axle, like the
   real mid-engine Civic swaps. Engine bays then come from subframes instead of one
   `Chassis::bay`: the rear one takes the rear bench's place and part of the hold, the
   front axle goes undriven, and the masses, the engine's drawn place, the sound source,
   the exhaust exit and the radiator lines follow. One engine per car. It touches
   physics, looks, audio and fitting, so it is the largest step and is kept last.

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
