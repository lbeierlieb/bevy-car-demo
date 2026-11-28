# Exhaust manifolds and turbo mounts: the plan

Asked for by the user on 2026-11-27: the exhaust was too simple. Any engine had one
"Exhaust manifold" slot and, beside it, one "Turbo or supercharger" slot, independent of
each other, so long-tube headers took a turbo, and a V10 had one header and one turbo.
Wanted: V10 long-tube headers (one per bank, ending at the cat), turbo manifolds per bank
with a turbo on each, and one big manifold joining all ten into a single turbo; on an
inline six, NA long tubes, a single-turbo manifold and a twin-turbo manifold (3 + 3).
Sides may be required to match. The milestones go in order; each ends with clippy
(native and wasm) and tests passing, rustfmt clean, the game playable, and committed
atomically (see CLAUDE.md, "Working conventions").

The user's answers (2026-11-27): no inline-six test engine (the layout must allow
one, but nothing is built for it until a real I6 family comes); the plan as below. All
three milestones were built on 2026-11-27 (see the "Decided while building" sections at
the end).

## What the model assumed before

- `EngineHardware::exhaust.header_length` is one primary length (the cycle simulation's
  header pipe, reflecting inverted off a collector at constant pressure, `cycle.rs`
  `HEADER_REFLECTION`), and `turbo: Option<Turbo>` one turbo taking all the exhaust:
  one turbine and wastegate in `exhaust_pressure`, one shaft (`Engine::turbo_speed`,
  `step_turbo`), one compressor feeding the tract (`tract_inflow`).
- Header parts are generic multipliers on the engine's header length (`catalogue/parts.rs`:
  stock, short ×0.56, long-tube ×1.375) and fit every engine; turbo kits
  (`parts::TurboKit::fit`) are scaled by the whole engine's displacement.
- Item tree: the engine item's slots are the long block, then `ENGINE_SLOTS` (intake
  manifold, exhaust manifold, air filter, throttle, turbo or supercharger, flywheel),
  flat. The intercooler, catalyst, silencer and exhaust pipe are the car's
  (`CAR_SLOTS`).
- The chassis' `header_room` caps the primaries' length (`Parts::header_room`).

## The idea

**The exhaust manifold decides the layout; turbos sit in mounts on it.** A manifold
without mounts (long tubes, a stock NA manifold) can't carry a turbo.

```
Engine
├ Long block …
├ Intake manifold, air filter, throttle, flywheel
├ Supercharger            (what's left of "Turbo or supercharger")
└ Exhaust manifold        (made for one engine family)
   ├ Turbo, left bank     (only on turbo manifolds; as many mounts as branches)
   └ Turbo, right bank
```

A manifold definition: primary length (m, absolute, for its family), collector kind
(`Tuned`: a collector into the pipe; `Turbine`: a turbine inlet), branches (groups of
cylinders, each to one collector or turbo: all the same size), and whether each branch
has a turbo mount. Examples:

| Engine | Manifold | Branches → mounts |
|---|---|---|
| R80 V10 | Stock cast manifolds (pair) | 2 × 5 → 0 |
| R80 V10 | Long-tube headers (pair, ending at the cat) | 2 × 5 → 0 |
| R80 V10 | Turbo manifolds (pair) | 2 × 5 → 2 |
| R80 V10 | 10-into-1 turbo manifold (crossover) | 1 × 10 → 1 |
| B16/B18 | Stock cast 4-into-1, short 4-1, long-tube 4-2-1 | 1 × 4 → 0 |
| B16/B18 | Turbo manifold | 1 × 4 → 1 |
| (a future I6) | long-tube 6-into-1 / single turbo / twin 3 + 3 | 1×6→0, 1×6→1, 2×3→2 |

Rules (`Item::fine` and `fitting`): a manifold fits only its engine family; a turbo
manifold's mounts are all empty or all hold the **same turbo definition** with the same
knob values (wastegate); turbos and a supercharger exclude each other (no twin-charging
for now); a shop fitting a turbo fills every mount at once (like "both on the axle").
An empty-mount turbo manifold runs as an NA engine with that manifold.

## Different turbos per side

Not built, but the model allows it later: two turbo states sharing the boost pipe (the
intake solve stays its 2×2 implicit step; turbo speeds are explicit), each bank with its
own exhaust pressure, torque the mean of two map lookups (about twice the lookup cost).
The hard part is two unequal compressors on one pipe (the stronger stalls the weaker:
surge and backflow, which the compressor model doesn't cover), and every display and the
ECU's view. With matching sides, N turbos are exactly one turbo state with the flow split
N ways, at no extra cost: that's what's built.

## Assumptions

1. A V engine's pair of headers (or manifolds) is **one item**, sold as a set; matching
   sides come for free. The 10-into-1 is one item too.
2. Turbos stay relative sizes (small, medium, large), sized for the displacement behind
   their mount (a "Medium" on a V10 bank is sized for 4 L).
3. Everything after the manifold stays the car's (pipe, cat, silencer, intercooler), as
   agreed before: twin turbos share one intercooler; a pair's outlets go into whatever
   exhaust the chassis has. No dual-exhaust parts.
4. The cycle simulation stays one cylinder: grouping changes the collector and the
   turbine's flow, not interference between cylinders sharing a collector.
5. `header_room` still caps the primaries' length (long tubes are what it shortens); no
   box check for turbos in the bay yet.
6. Saved cars are converted (format 4), not dropped.
7. Turbo wear is per turbo item, all mounts strained alike; any one failing kills the
   boost (an approximation).

## Milestone 1: data and catalogue (no physics change)

1. `items::Kind::Manifold` and `Kind::Turbo`; a family restriction on `PartDef` (like
   `fits` for chassis, by family or version code), checked in `Item::fine` and by
   `fitting`. The engine item's slots: long block, intake manifold, exhaust manifold
   (`Kind::Manifold`, required), air filter, throttle, supercharger (the old turbo slot,
   superchargers and "None"), flywheel.
2. Manifold definitions per family in the catalogue (`catalogue/engines.rs` data on the
   family, generated into defs in `catalogue/items.rs`): ids like `b.manifold.stock`,
   `b.headers.long-tube`, `b.manifold.turbo`, `r.manifolds.stock`, `r.headers.long-tube`,
   `r.manifolds.turbo`, `r.manifold.turbo-crossover`. Each a `Does::Manifold` with the
   layout above; turbo mounts are `SlotDef`s of `Kind::Turbo`, not required. Primary
   lengths match today's results (B series stock = today's stock header length after the
   family's preset; short/long as today's multipliers; R80 likewise).
3. Turbo definitions (`Kind::Turbo`, the old `turbocharger.small|medium|large` ids kept,
   wear `Turbo`) go only into mounts.
4. The bridge: `Item::resolve` gives `Parts` the manifold (which one, with its layout) and
   the turbo (which, how many: the mount count when filled); `items::assemble` puts them
   back. Rules: all mounts the same turbo and knobs, no turbo with a supercharger.
5. Trims: the B16/B18/R80 trims get their stock manifolds. Shops file: car shops the
   stock manifolds, tuning shops headers, turbo manifolds and turbos; the engine shop as
   before. `what_the_shops_sell_is_in_the_catalogue` passes.
6. Saved cars, format 4 (`saved.rs`): an old exhaust manifold part becomes the family's
   matching one; an old turbo in the old slot moves onto the family's turbo manifold
   (replacing the header); old supercharger ids stay in the supercharger slot. Tested.
7. Until milestone 2 the hardware keeps working as now: the manifold sets
   `header_length`, a filled mount set gives `turbo` (sized for the whole engine as
   before, so nothing measured changes yet; all baked maps unchanged).
8. Protocol bump (catalogue, items on the wire). Fitting rules tested in
   `fitting::tests` (long tubes have no mount, a turbo fills every mount, mixed turbos
   refused, turbo + supercharger refused, a B-series manifold refused on the R80).

## Milestone 2: physics (built 2026-11-27)

1. `hardware::Exhaust`: `header_length` becomes `manifold: Manifold { primary_length,
   collector: Collector, branches: u32 }`; `EngineHardware::turbos: u32` (0 without a
   turbo; the `Turbo` describes one of them). Maps' fingerprint follows.
2. Runtime (`engine/mod.rs`): each turbine and wastegate passes flow / turbos
   (`exhaust_pressure`, `step_turbo`), the compressor flow × turbos goes into the tract
   (`tract_inflow`); one shaft state. Telemetry, damage strain unchanged per turbo.
3. Turbo kits sized for the displacement behind a mount (`TurboKit::fit` with the
   branch's share; a manifold with one branch: the whole engine as now, so the B series'
   turbo results stay).
4. `cycle.rs`: the collector kind sets the header's reflection (`Tuned` as now; a
   turbine inlet reflects much less, so less scavenging). Re-bake every stock engine
   (`bake_stock_maps`).
5. `Ecu::calibrated` and factory tunes work for twin turbos (the turbine-inlet margin
   per turbo).
6. Tests: long tubes beat a turbo manifold with empty mounts on the R80 low down or
   through the band as the model says; on the R80 twin turbos (pair) spool earlier than
   the 10-into-1 with the same turbo kit (smaller per turbo), and the single big turbo
   holds boost high up; the B-series turbo results unchanged within tolerance;
   `every_factory_engine_idles_and_pulls` passes.

## Milestone 3: the game (built 2026-11-27)

1. `garage/fit.rs`: the manifold's screen with its mounts; fitting a turbo fills all
   mounts; readouts (primary length, branches, mounts).
2. `mechanicals.rs`: headers per bank as the manifold says (long tubes, cast manifolds,
   turbo manifolds), a turbo at each collector, the crossover pipe for the 10-into-1.
3. `Looks`/sound: the turbo count (`ExhaustSound`, the whistle per turbo) — protocol
   bump; telemetry (2) shows boost and turbo speed (and the count).
4. Notes (CLAUDE.md) and this plan as built.

## Later (not in this plan)

Different turbos per side; sequential turbos, twin-scroll; pulse interference by firing
order within a collector (the odd-fire V10, 3-cylinder groups); an inline six family;
dual-exhaust parts; turbos checked against the bay's room.

## Decided while building milestone 1

Built 2026-11-27. Where the plan was silent:

1. **Manifolds are family data, not engine-slot parts.** `Family::manifolds` holds
   `ManifoldDef { id, name, manifold: hardware::Manifold { primary_length, collector,
   branches }, mounts }`; `Parts::manifold` and `Preset::manifold` are its place in the
   family's list (every factory version's is 0). `Parts::hardware` sets
   `header_length` from it (then `header_room` cuts it, as before). The catalogue's old
   "Exhaust manifold" engine slot with its three generic multipliers is gone from
   `Catalogue::slots`. `hardware::Manifold` and `Collector` exist already, unused by the
   physics until milestone 2.
2. **The old turbo slot split in two at the `Parts` level**: "Turbo" (`TURBO`, 11: None,
   Small, Medium, Large; `Slot::mounted`, so its parts are `Kind::Turbo` items and its
   first part stands for an empty mount) and "Supercharger" (`SUPERCHARGER`, 12: None,
   Roots, centrifugal), next to each other so the effects apply in the same order. With
   the manifold slot gone there are still 20 slots; air filter and throttle moved up
   (9, 10), the intercooler and the rest kept their places. The engine item's slots:
   long block, intake manifold, exhaust manifold, air filter, throttle, supercharger,
   flywheel (`ENGINE_SLOTS` = 8, 9, 10, 12, 18 with the manifold put in after the
   intake).
3. **Ids**: turbos keep `turbocharger.small|medium|large`, superchargers
   `turbocharger.roots-blower`, `turbocharger.centrifugal-supercharger` and the absent
   `turbocharger.none`; there's no "None" turbo item (an empty mount is none).
4. **The manifolds**: the B series' `b.manifold.stock` (700 mm), `b.headers.short` (390),
   `b.headers.long-tube` (960), `b.manifold.turbo` (700, one mount); the R80's
   `r.manifolds.stock` (500, two branches), `r.headers.long-tube` (690),
   `r.manifolds.turbo` (500, two mounts), `r.manifold.turbo-crossover` (900 mm, made up:
   the far bank's primaries cross the valley; one branch, one mount). Turbo manifolds
   have the cast ones' primaries, so a turbo engine is the same hardware as with the
   old slot (the B16T's baked maps still match). The R80 had short headers through the
   generic part (280 mm); the plan's table has none, so old saves get its stock pair.
   Collector `Turbine` for manifolds with mounts, else `Tuned`.
5. **Family restriction**: `PartDef::engines` (families' names or versions' codes,
   none: any); manifolds name their family. `items::engine_misfit` checks it, the mounts
   (all empty, or all the same definition with the same knobs) and the exclusion of a
   turbo and a supercharger anywhere in an engine. `Item::fine` calls it on every tree
   (cars, loose parts, the wire), `resolve` refuses such a car, `Bench::finish` says why
   for the car, the trunk and what's held. Turbos have no knobs: the wastegate's spring
   stays the engine item's "Boost" knob, set at dynos as before.
6. **Fitting**: a new turbo goes on several mounts in one `Work::Fit` (as "both on the
   axle"); a turbo brought fits one mount, so a pair needs two in one request (the rules
   are checked on the result). `fitting::mounts` gives every mount of the manifold a
   mount's path is on, `engine_on` the engine family on a path, `made_for_engine` whether
   a definition suits it. The workshop window (a little of milestone 3, to keep the game
   playable) offers only the engine's family's manifolds and fits a new turbo with "Fit
   one on every mount" where there are several. Engine shops don't fit turbos (they're
   not inside a long block).
7. **`assemble`** keeps the manifold item when it's the same, puts the turbo wanted on
   every mount (keeping turbo items of the same definition) or clears them, and refuses a
   turbo on a manifold without mounts rather than swapping the manifold. `Parts::turbo`
   (tests) fits the B series' turbo manifold.
8. **Items**: a manifold weighs 1.5 kg per cylinder (the B series' 6 kg as before), ×1.3
   with turbine flanges; mounts are "Turbo" (one), "Turbo, left bank"/"right bank" (a V's
   two), "Turbo, front"/"rear" (an inline engine's two), else "Turbo N". Used-part values
   are 150 € like other engine parts (×2.5 for ids with "turbo").
9. **Wear**: every turbo gets the engine's turbo wear (`Condition::put_on`); read back,
   the worst turbo's counts (any one failing kills the boost).
10. **Shops file**: car workshops sell `b.manifold.stock r.manifolds.stock`, tuning shops
    `b.* r.*` (they sold every exhaust manifold before, stock ones too) and
    `turbocharger.*` (turbos and superchargers); the engine shop as before.
11. **Saved cars, format 4**, converted by ids (loose parts and marketplace offers carry
    no version): an old `exhaust-manifold.*` becomes its engine's family's manifold
    without mounts whose primaries are nearest what the old part made of the block's
    headers (loose: the first family's); a turbo in the old "Turbo or supercharger" slot
    goes on every mount of the family's turbo manifold nearest the manifold it had
    (replacing it if it has no mounts: on the R80 long tubes, the pair, not the
    crossover), and the empty supercharger slot gets "None"; superchargers land in the
    supercharger slot by their kind.
12. `mechanicals` draws short or long headers by the primaries against the family's
    first manifold, and a turbo when one is on a mount (one turbo, as before, for a pair
    too, until milestone 3). Protocol 53.

## Decided while building milestone 2

Built 2026-11-27. Where the plan was silent:

1. **`Exhaust::manifold`** (`hardware::Manifold`) replaced `header_length`; a block's is
   its factory primaries with a `Tuned` collector and one branch, and `Parts::hardware`
   puts the fitted manifold in *before* the parts' effects, so a turbo kit knows the
   branches it goes on. `EngineHardware::turbos` is set by `TurboKit::fit` to the
   manifold's branches (a turbo chosen on a manifold without mounts, possible only in
   tests, still gets one per branch); `turbo_count()` is the factor (1 at least with a
   turbo). Both are on the wire (catalogue blocks): protocol 54.
2. **What is shared**: the air filter, the ducts, the intercooler (sized for the whole
   engine, `3e-3 × size`) and the wastegate's opening (one state: the turbos see the same
   boost). Per turbo: compressor, turbine, wastegate valve, diverter valve (all sized for
   the displacement behind the branch). The mass of the turbos is 12 kg each plus 6 kg ×
   the engine's relative size (one turbo: as before).
3. **Turbine inlet reflection 0.3** (tuned collectors 0.8, as before). The model's waves
   come back inverted; a turbine's nozzle chokes the collector, so less comes back. The
   fingerprint has the primary length and the collector kind, not the branches (one
   cylinder is simulated, so the pair and a 10-into-1 of the same primaries would share
   maps). Every stock engine re-baked.
4. **Scale efficiency**: with kits scaled exactly by displacement, two half-size turbos
   and one big one are the same engine in steady state (the model is homogeneous in the
   size); only the shafts' inertia (∝ wheel⁵) differed. Real small turbos are a little
   less efficient (tip clearances, blade edges), so the compressor's and turbine's best
   efficiencies gain `TURBO_SCALE_EFFICIENCY` (0.02) per doubling of the displacement
   behind them (±0.05 at most). The B series' turbo (sized for 1.6 l) loses 0.006.
   This makes the big single hold boost high up; the twins still spool quicker.
5. **No exhaust heat loss before the turbine** was added (it would have slowed the
   crossover's 900 mm primaries further, but changes every turbo engine's temperatures,
   the turbine limit and the factory tunes' richening). Steady state, the big single is a
   little ahead low down too (its efficiency); its lag is only in transients.
6. **Oil cooling** (`thermal.rs`): the oil passed heat to the coolant at the 2-litre
   four's 600 W/K whatever the engine, so the R80 with twin turbos (~1000 N·m) overheated
   its oil (434 K, bearings 17 % worn) in a minute flat out at 5500 rpm, and the NA R80
   reached 412 K. Now ×(oil heat capacity / the 2-litre's) (2.8× on the R80, 0.88× on
   the B16): NA R80 391 K; the twin turbo engine lasts the minute (a separate commit).
7. **`Ecu::calibrated`** needed no change: it goes through `exhaust_pressure` (which
   splits the flow) and the turbine inlet temperature is the cylinders' exhaust, the same
   for every turbo, so the margin per turbo is the one margin.
8. A test changed: `traction_control_keeps_a_big_engine_from_spinning_its_wheels` (the
   B16T at 1.2 bar floored in first without traction control) spins its wheels at 1.4×
   the road speed now (asserted > 1.3×, was > 1.5×): the turbine inlet's weaker
   reflection changed the low-rpm torque.

### Measured (flat out after 4 s at each speed; boost bar, torque N·m)

The R80 with low compression pistons and the medium turbo kit on every mount, wastegate
at 0.6 bar (compressor 71 mm on each of the pair, 100 mm on the 10-into-1):

| rpm | pair (2 × 4 l) | 10-into-1 (1 × 8 l) |
|---|---|---|
| 2000 | 0.09, 596 | 0.09, 599 |
| 2500 | 0.18, 663 | 0.19, 674 |
| 3000 | 0.36, 800 | 0.40, 831 |
| 3500 | 0.62, 974 | 0.62, 982 |
| 4000 | 0.65, 968 | 0.65, 979 |
| 5000 | 0.69, 885 | 0.69, 906 |
| 5800 | 0.70, 763 | 0.70, 788 |

Time to 0.4 bar after flooring it from a light throttle: pair 0.64 s at 3500 rpm, 0.43 s
at 4000; the 10-into-1 0.83 s and 0.56 s (the small kit: 0.28/0.23 s against 0.35/0.28
s; the large: 1.48 s against 1.78 s at 4000). The small kit held to 1.5 bar at 5800 rpm
spins the pair's shafts at 96 % of their limit, the big one's at 87 %; held to 1.8 bar
the pair burst at 5000 rpm while the big one holds 1.8 bar (it bursts at 5800).

The R80 naturally aspirated, 3500–5500 rpm in 500 rpm steps: stock cast pair 631, 638,
625, 585, 520; turbo manifolds with empty mounts (same 500 mm primaries, turbine inlets)
628, 634, 622, 581, 518; long tubes (690 mm) 632, 637, 616, 570, 496 (a little ahead of
the empty turbo manifolds at 3500 and 4000, behind higher up: the longer primaries tune
lower; `the_r80s_manifolds_tune_it_without_turbos`). The header's tuning is a small effect in this model on the R80 (under 1 %).

The B16T (medium turbo, 0.6 bar): 2000 rpm 0.10 bar 158 N·m, 3000 0.42/209, 4000
0.65/245, 4500 0.67/250, 5000 0.69/248, 5800 0.71/242; before (reflection 0.8): 3000
0.40/202, 4000 0.65/254, 5000 0.69/241, 5800 0.70/230 (within 6 %:
`the_b_series_turbo_is_as_before`).

Not done: exhaust heat loss before the turbine; pulse interference within a collector
(the twins' whistle, telemetry and drawing came with milestone 3).

## Decided while building milestone 3

Built 2026-11-27. Where the plan was silent:

1. **The workshop** (`garage/fit.rs`): the exhaust manifold's slot screen and a new
   manifold's screen show its readouts as info rows (not focusable, so the six buttons
   pass over them): primaries (mm, "cast" or "tubes"), branches ("each bank's 5 into its
   own", "all 10 cylinders into one"), what they end in (tuned collectors or turbine
   inlets), turbo mounts, and on the fitted one the turbos ("2 × Medium (turbo)"). The
   mounts are rows as before; a mount and a new turbo say how much engine a turbo there
   is sized for ("4.0 l behind each mount"). Turbos come off all together: "Take the
   turbos off" on the manifold, "Take it off every mount" on a mount (several
   `Work::Remove` in one request). Turbos brought fit only as many alike as there are
   mounts, as one request of a `Work::Fit` per mount (the highest places in the trunk
   and held list taken first, so the others don't move); fewer say how many are needed.
2. **Changing the manifold** (`fitting::Bench::fit`, a server rule): the turbos move to
   the new manifold only when it has as many mounts as there are turbos; else they stay
   on the old one, which comes off with them (before, one of a pair moved to the
   10-into-1 and the pair went into the trunk with one mount filled, which the rules
   refuse). A new manifold's screen says which will happen.
3. **`ManifoldDef::cast`** (on the wire, protocol 55): how it's drawn. The stock and
   turbo manifolds are cast; headers and the 10-into-1 (900 mm primaries, fabricated)
   are tubes.
4. **Drawing** (`mechanicals.rs`, `Headers`): cast manifolds are an iron log along the
   head (each bank's on a V) with a stub from each port, its outlet down to the
   collector or, on a turbo manifold, the turbine on its flange (on the R80 at the
   bank's rear end, low and outboard; on the B series in the log's middle, towards the
   bulkhead). Tubes go out and down to a collector behind the engine, lower and further
   back the longer they are against the family's own manifold (short under 0.8, long
   over 1.2); the B series' long tubes are drawn as a 4-2-1 (the outer and the inner
   two paired). The 10-into-1: the right bank's primaries into a collector at its rear,
   high, with the one turbo; the left bank's into one at its rear and a pipe from it
   behind the engine, over the bell housing, across to the right (the plan's "far
   bank's primaries cross the valley", drawn behind the engine rather than over it). A
   turbo at each turbine inlet: an iron snail and an alloy compressor towards the
   intake's end, its pipe to the throttle (the intercooler between isn't drawn), each
   compressor's own pipe to the air filter (twin cones with the open filter). The snail's
   radius is 62/75/88 mm for small/medium/large × the cube root of the displacement
   behind the mount over 1.6 l (0.8 to 1.6), so the R80's pair is bigger than the B
   series' and the 10-into-1's biggest. Each turbine's outlet starts an exhaust line (the
   pair's two side pipes from their own turbo, the 10-into-1's one turbo feeding both).
5. **Sound** (protocol 56): `ExhaustSound::turbos` (the hardware's `turbo_count`, a byte
   on the wire with the engine's looks). Two or more turbos whistle as two bands, the
   second `TWIN_DETUNE` (2 %) higher (no two shafts turn at quite the same speed: they
   beat), together as loud as one turbo; on a V (dual exhaust) each side gets its own
   bank's at one turbo's level. More than two turbos are still two bands. The pitch
   still follows the one shaft state (smaller twins spin faster, so they whistle higher
   by themselves). Never listened to.
6. **Telemetry** (2): the boost line names the turbos ("twin turbos 98k rpm (60 % of the
   limit)"); boost, shaft speed and wastegate were there already.
7. **Shots**: `FITIDS` puts a turbo on every mount of the first manifold with mounts
   (the manifold first in the list); `CARCAM` takes a fourth number, how far ahead of
   the car's middle to look (1.0 at the engine bay).

Not done: the turbos' second shaft (different turbos per side), an intercooler drawn in
the pipes, the 10-into-1's crossover over the valley, turbos checked against the bay's
room (the R80's large crossover turbo stays under the bonnet only by its placing).

## How a manifold is made (2026-11-28, agreed with the user)

The header's reflection depended on the collector alone (`Tuned` 0.8, `Turbine` 0.3), so
a cast manifold tuned exactly like tube headers. Now `hardware::Manifold::made`
(`Made::Tubes`, `Cast`, `CastPaired`) says how it's made; it is in the maps' fingerprint
and on the wire (protocol 73), and `ManifoldDef::cast()` is derived from it.

1. **Reflection** (`cycle.rs`): tubes 0.8 (each primary smooth and on its own until it
   opens into a big collector cone, nearly an open end), a cast 4-2-1 0.6 (each pair a
   longer run of its own into a Y), a cast log 0.45 (the stubs merge early into a small
   volume: the step in area is small, rough walls and the bends at the ports take more),
   any turbine inlet 0.3 as before.
2. **Measured, the weaker reflection alone made cast manifolds better**: their 650–700 mm
   primaries' wave hurt at the power peak, so the B16, B20, K20 and K24 gained 2–5 kW and
   headers still bought almost nothing. What a real cast manifold loses is mostly at its
   merge, so the merge is a live restriction (`Exhaust::resistance`: an effective area of
   the pipe's × `Made::CAST_FLOW` 1.0, about a dynamic head lost there, as much as the pipe
   loses over its length and bends; a 4-2-1's two Ys `PAIRED_FLOW` 1.5; none while it
   feeds a turbine, whose map takes it in; an empty turbo manifold has it). It acts live
   (no re-bake), more the more flow: more up high.
3. **Tried and left out**: the other cylinders' blowdown crossing over at a cast log's
   merge (each a firing interval behind this one's, after the same echo delay, 0.2 of it
   on a log and 0.3 from the 360° partner on a 4-2-1): with these primaries the pulses
   arrive while the valve is shut, it raised the cast manifolds' torque a little, so it
   isn't worth its code. Smearing of the wave by rough walls neither.
4. **Re-tuned**: the B16's head flows 0.93 (was 0.95; back to 66 kW). The K20R's wild cam
   now pays on cast manifolds too, so it gets a cast 4-2-1 from the factory like the real
   one (`k.manifold.4-2-1`, 850 mm, appended last so the places stay; car workshops sell
   it) and 400 mm runners (440 put a VE cliff at 7500 that peaked it at 7000); long tubes
   are an upgrade for it now (+2 %). Saved K20Rs keep the long tubes fitted, saved B16Rs
   the 700 mm manifold (no conversion: it's what's fitted; their maps bake on the server).
   A version's block takes its factory manifold (`engines::with_factory_manifolds`), so the
   locked ECUs' tunes are made for it.
5. Tools: `factory_engines_on_their_manifolds` (ignored, `MEASURE=codes`) prints every
   factory engine with its own ECU and on each of its family's manifolds (programmable
   ECU), with the gain over the factory one; `full_load_ve_by_cam` takes `VE_MADE`
   (`tubes`, `cast`, `paired`). Test: `tube_headers_beat_the_cast_manifold` (the K20: long
   tubes 3–9 % more peak power than the cast 4-into-1, the 4-2-1 between, more at 6500
   than at 4500).

Peaks with the engine's own ECU (torque N·m, power kW at rpm; limits unchanged):

| Engine | before | after |
|---|---|---|
| B16 | 154, 66 at 5000 | 153, 66 at 5000 (flow 0.93) |
| B20 | 189, 78 at 4500 | 188, 79 at 4500 |
| B16R | 172, 131 at 8000, switch 5700 | 171, 129 at 8000, switch 5500 |
| R80 | 638, 306 at 5000 | 633, 303 at 5000 |
| K20 | 211, 115 at 6500 | 211, 119 at 6500 |
| K20R | 229, 158 at 8500 (long tubes) | 225, 160 at 7500, switch 5900 (cast 4-2-1) |
| K24 | 245, 118 at 5500 | 242, 122 at 5500 |
| W20 | 206, 115 at 6500 | the same (tubes) |
| B16T | 230, 108 at 5000 | 229, 106 at 5000 (the B16's head) |

Peak power on each manifold against the factory one (programmable ECU), before → after:
B16 short +2.3 → +0.6 %, long +2.1 → +0.4 %; B20 short +1.6 → +0.7 %, long −0.3 → −1.2 %;
B16R (on the 4-2-1) cast 4-into-1 −0.7 → −2.1 %, long +1.1 → +2.7 %; R80 long −2.6 →
−1.6 %; K20 short +5.2 → +1.6 %, long +9.3 → +5.6 %, 4-2-1 +4.0 %; K24 short +4.1 →
+1.1 %, long +8.2 → +5.1 %, 4-2-1 +2.1 %; K20R (now on its 4-2-1) cast 4-into-1 −3.7 %,
long +1.8 % (before, on long tubes: cast −4.0 %). The "before" gains of the short headers
came from the cast manifolds' harmful wave, not from tuning. Open: the 8-valve B16/B20
gain under 1 % from any header (little overlap to scavenge, power peaking at 4500–5000
where the flow is small); more would take a restriction larger than plausible.
