//! What a car is built of, whole, as it goes to the server: the chassis and what's fitted
//! to it (alignment and brakes too), the engine's parts and its ECU's tune. The server
//! builds the same `CarSetup` from it and simulates the car exactly as the player's game
//! does. Everything is checked when read: catalogue places, settings within what the shops
//! offer, and that the engine fits the car.

use crate::car::CarSetup;
use crate::chassis::{BRAKE_BALANCES, BRAKE_FORCES, CarBuild, PRESSURES, TOES};
use crate::items::Item;
use crate::net::wire::{Reader, Wire};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::ecu::{CamControl, Ecu, EcuModel};
use crate::physics::engine::maps::EngineMaps;
use crate::physics::engine::parts::Parts;
use crate::physics::engine::table::Table;
use crate::physics::fuel::PUMP_OCTANE;

/// A car as built: from it, `setup` makes the car the simulation runs. On the wire it's
/// the car's items and the tune; `build` and `parts` are what the items resolve to.
#[derive(Clone, Debug, PartialEq)]
pub struct CarSpec {
    pub car: Item,
    pub build: CarBuild,
    /// As installed in the car (`CarBuild::install`).
    pub parts: Parts,
    pub ecu: Ecu,
    /// The loose parts it carries (see `cargo`).
    pub cargo: Vec<Item>,
}

/// Slack on settings' ranges for values a slider left a hair outside them.
const SLACK: f32 = 1e-3;

/// The most values in a table read (the ECU's are a few hundred).
const MOST_VALUES: usize = 4096;

impl CarSpec {
    pub fn of(setup: &CarSetup) -> Self {
        Self {
            car: setup.car.clone(),
            build: setup.build.clone(),
            parts: setup.engine.parts.clone(),
            ecu: setup.engine.build.ecu.clone(),
            cargo: setup.cargo.clone(),
        }
    }

    /// The car's setup with its engine's `maps` (see `maps`).
    pub fn setup_with(&self, maps: EngineMaps) -> CarSetup {
        let engine = EngineBuild {
            hardware: self.parts.hardware(),
            maps,
            ecu: self.ecu.clone(),
            octane: PUMP_OCTANE,
        };
        let mut setup = CarSetup::with_car(
            self.build.clone(),
            self.parts.clone(),
            engine,
            self.car.clone(),
        );
        setup.set_cargo(self.cargo.clone());
        setup
    }

    /// The engine's maps, if they come with the game (a factory engine on the cylinder
    /// side); otherwise they have to be baked (`EngineMaps::bake`, seconds).
    pub fn stock_maps(&self) -> Option<EngineMaps> {
        EngineMaps::stock(&self.parts.hardware())
    }

    /// The car's setup, baking its engine's maps if they don't come with the game.
    pub fn setup(&self) -> CarSetup {
        self.setup_with(EngineMaps::for_hardware(&self.parts.hardware()))
    }
}

/// Within `range`, give or take `SLACK`.
fn within(x: f32, range: &std::ops::RangeInclusive<f32>) -> bool {
    (range.start() - SLACK..=range.end() + SLACK).contains(&x)
}

impl Wire for CarSpec {
    fn put(&self, out: &mut Vec<u8>) {
        self.car.put(out);
        self.ecu.put(out);
        self.cargo.put(out);
    }

    fn get(r: &mut Reader) -> Option<Self> {
        let catalogue = r.catalogue()?;
        // Items read only as they fit the catalogue (`Item::fine`).
        let car: Item = r.get()?;
        let ecu: Ecu = r.get()?;
        // Loose parts (no cars), all with room in the hold.
        let cargo: Vec<Item> = r.get()?;
        let loose =
            (cargo.iter()).all(|item| item.def(catalogue).kind != crate::items::Kind::Chassis);
        (loose && crate::cargo::stow(catalogue, &car, &cargo).is_ok()).then_some(())?;
        let (build, parts) = car.resolve(catalogue).ok()?;
        let settings = (0..2).all(|axle| {
            within(build.toe[axle], &TOES)
                && within(build.pressure[axle], &PRESSURES)
                && within(build.camber[axle], &build.camber_range(axle))
        }) && within(build.brake_force, &BRAKE_FORCES)
            && within(build.brake_balance, &BRAKE_BALANCES)
            && within(build.final_drive, &(2.0..=6.0));
        let tune = ecu.model == parts.ecu
            && ecu.cylinders == parts.hardware().cylinders
            && ecu.cylinders > 0;
        let parts_fine = within(parts.intake_timing, &(-10.0..=10.0))
            && within(parts.exhaust_timing, &(-10.0..=10.0))
            && within(parts.boost, &(0.3..=1.5));
        (settings && tune && parts_fine).then_some(Self {
            car,
            build,
            parts,
            ecu,
            cargo,
        })
    }
}

impl Wire for Parts {
    fn put(&self, out: &mut Vec<u8>) {
        self.family.put(out);
        self.version.put(out);
        // As many as the catalogue has slots.
        for chosen in &self.chosen {
            chosen.put(out);
        }
        self.manifold.put(out);
        self.head.put(out);
        self.bore.put(out);
        self.intake_timing.put(out);
        self.exhaust_timing.put(out);
        self.boost.put(out);
        self.ecu.put(out);
    }

    /// Their header room is the car's to say (`CarBuild::install`).
    fn get(r: &mut Reader) -> Option<Self> {
        let catalogue = r.catalogue()?;
        let family: usize = r.get()?;
        let version: usize = r.get()?;
        let chosen = (0..catalogue.slots.len())
            .map(|_| r.get())
            .collect::<Option<_>>()?;
        let parts = Self {
            catalogue: catalogue.clone(),
            family,
            version,
            chosen,
            manifold: r.get()?,
            head: r.get()?,
            bore: r.get()?,
            intake_timing: r.get()?,
            exhaust_timing: r.get()?,
            boost: r.get()?,
            ecu: r.get()?,
            header_room: 0.0,
        };
        let fine = (catalogue.families.get(family)).is_some_and(|f| {
            version < f.versions.len()
                && parts.manifold < f.manifolds.len()
                && parts.head < f.heads.len()
        }) && crate::physics::engine::parts::fits(catalogue, &parts.chosen)
            && parts.bore < crate::items::OVERSIZES.len()
            && within(parts.intake_timing, &(-10.0..=10.0))
            && within(parts.exhaust_timing, &(-10.0..=10.0))
            && within(parts.boost, &(0.3..=1.5));
        fine.then_some(parts)
    }
}

impl Wire for EcuModel {
    fn put(&self, out: &mut Vec<u8>) {
        let i = Self::ALL.iter().position(|m| m == self).expect("a model");
        i.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Self::ALL.get(r.get::<usize>()?).copied()
    }
}

impl Wire for Ecu {
    fn put(&self, out: &mut Vec<u8>) {
        self.model.put(out);
        self.displacement.put(out);
        self.cylinders.put(out);
        self.ve.put(out);
        self.lambda.put(out);
        self.spark.put(out);
        self.idle_rpm.put(out);
        self.rev_limit.put(out);
        self.limiter_cut.put(out);
        self.limiter_fuel.put(out);
        self.cams.switch_rpm.put(out);
        self.cams.switch_map.put(out);
        self.cams.intake_phase.put(out);
        self.cams.exhaust_phase.put(out);
        self.cams.ve.put(out);
        self.cams.lambda.put(out);
        self.cams.spark.put(out);
    }

    fn get(r: &mut Reader) -> Option<Self> {
        let ecu = Self {
            model: r.get()?,
            displacement: r.get()?,
            cylinders: r.get()?,
            ve: r.get()?,
            lambda: r.get()?,
            spark: r.get()?,
            idle_rpm: r.get()?,
            rev_limit: r.get()?,
            limiter_cut: r.get()?,
            limiter_fuel: r.get()?,
            cams: CamControl {
                switch_rpm: r.get()?,
                switch_map: r.get()?,
                intake_phase: r.get()?,
                exhaust_phase: r.get()?,
                ve: r.get()?,
                lambda: r.get()?,
                spark: r.get()?,
            },
        };
        // The tables' axes are rpm and pressure, so more than two is nonsense.
        let tables = [
            &ecu.ve,
            &ecu.lambda,
            &ecu.spark,
            &ecu.cams.intake_phase,
            &ecu.cams.exhaust_phase,
            &ecu.cams.ve,
            &ecu.cams.lambda,
            &ecu.cams.spark,
        ];
        let fine = tables.iter().all(|table| table.axes().len() <= 2)
            && ecu.displacement > 0.0
            && ecu.idle_rpm > 0.0
            && ecu.rev_limit > ecu.idle_rpm;
        fine.then_some(ecu)
    }
}

impl Wire for Table {
    fn put(&self, out: &mut Vec<u8>) {
        (self.axes().len() as u8).put(out);
        for axis in self.axes() {
            axis.put(out);
        }
        self.values().to_vec().put(out);
    }

    fn get(r: &mut Reader) -> Option<Self> {
        let count = r.get::<u8>()?;
        let axes: Vec<Vec<f32>> = (0..count).map(|_| r.get()).collect::<Option<_>>()?;
        let values: Vec<f32> = r.get()?;
        if values.len() > MOST_VALUES {
            return None;
        }
        Table::from_parts(axes, values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::encode;

    /// Reads a message with cars of the built-in catalogue.
    fn decode<T: Wire>(bytes: &[u8]) -> Option<T> {
        crate::net::wire::decode_with(bytes, Some(&crate::catalogue::builtin()))
    }
    use crate::snapshot::save;
    use crate::terrain::Terrain;

    #[test]
    fn every_factory_car_reads_back_as_the_same_car() {
        let terrain = Terrain::flat();
        for (chassis, model) in crate::catalogue::builtin().chassis.iter().enumerate() {
            for option in 0..model.engines.len() {
                let setup = CarSetup::factory(chassis, option);
                let spec = CarSpec::of(&setup);
                let read: CarSpec = decode(&encode(&spec)).expect("reads back");
                let again = read.stock_maps().map(|maps| read.setup_with(maps));
                let again = again.expect("a factory engine's maps come with the game");
                // Driven alike, they stay alike to the bit.
                let mut cars = [setup, again]
                    .map(|setup| crate::car::Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0));
                for car in &mut cars {
                    car.drivetrain.automatic = true;
                    for _ in 0..240 {
                        car.controls.throttle = 1.0;
                        crate::sim::step_car(car, &terrain, 1.0 / 120.0);
                    }
                }
                assert_eq!(
                    save(&cars[0]),
                    save(&cars[1]),
                    "chassis {chassis}, option {option}"
                );
            }
        }
    }

    #[test]
    fn a_spec_out_of_range_is_refused() {
        // A knob past its range, a part that isn't in the catalogue, a part in a slot of
        // another kind, a slot that must have a part left empty.
        let mut spec = CarSpec::of(&CarSetup::default());
        spec.car.knobs[0] = 5.0;
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        let defs = CarSetup::default().build.catalogue.defs.len();
        let mut spec = CarSpec::of(&CarSetup::default());
        spec.car.slots.last_mut().unwrap().as_mut().unwrap().def = defs;
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        let mut spec = CarSpec::of(&CarSetup::default());
        let seat = spec
            .car
            .all()
            .iter()
            .find(|i| i.slots.is_empty())
            .unwrap()
            .def;
        spec.car.slots.last_mut().unwrap().as_mut().unwrap().def = seat;
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        let mut spec = CarSpec::of(&CarSetup::default());
        *spec.car.slots.last_mut().unwrap() = None;
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        let mut spec = CarSpec::of(&CarSetup::default());
        spec.ecu.cylinders += 1;
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        // More in the trunk than there's room for, or a car in it.
        let mut spec = CarSpec::of(&CarSetup::default());
        let wheel = spec
            .car
            .fitted(&spec.build.catalogue, crate::items::Kind::Wheel)
            .next()
            .unwrap()
            .clone();
        spec.cargo = vec![wheel.clone()];
        assert_eq!(decode::<CarSpec>(&encode(&spec)), Some(spec.clone()));
        spec.cargo = vec![wheel.clone(), wheel];
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
        spec.cargo = vec![spec.car.clone()];
        assert!(decode::<CarSpec>(&encode(&spec)).is_none());
    }

    #[test]
    fn what_a_car_carries_weighs() {
        let setup = CarSetup::default();
        let mut spec = CarSpec::of(&setup);
        let wheel = spec
            .car
            .fitted(&spec.build.catalogue, crate::items::Kind::Wheel)
            .next()
            .unwrap()
            .clone();
        spec.cargo = vec![wheel];
        let laden = spec.setup_with(setup.engine.build.maps.clone());
        let terrain = Terrain::flat();
        let [empty, laden] = [setup, laden]
            .map(|setup| crate::car::Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0));
        assert!((laden.body.mass - empty.body.mass - 18.0).abs() < 1e-3);
        // In the trunk, behind the centre of mass.
        assert!(laden.body.center.z > empty.body.center.z);
    }
}
