//! Fitting a part from the shelf to one of the player's cars. The part it replaces goes onto
//! the shelf (worn tires go to the bin). Fitting never touches the ECU's tune: after
//! changing the engine, the tune is still the one made for the old parts until it's
//! changed by hand.

use super::{Item, OwnedCar};
use crate::car::CarSetup;
use crate::chassis::{CLUTCHES, CarBuild, GEARBOXES, SUSPENSIONS};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::maps::{self, EngineMaps};
use crate::physics::engine::parts::{Parts, SLOTS};
use crate::physics::tire::{COMPOUNDS, Tread};
use crate::wheels::{Rim, TireSize, Wheel};

/// What a part is called on the shelf and in the shop.
pub fn name(item: Item) -> String {
    match item {
        Item::Engine { slot, part } => {
            format!("{}: {}", SLOTS[slot].name, SLOTS[slot].parts[part].name)
        }
        Item::Ecu(model) => format!("ECU: {}", model.name()),
        Item::Rims(rim) => format!("{} rims", rim.label()),
        Item::Tires(size, compound) => format!(
            "{} {} tires",
            size.label(),
            COMPOUNDS[compound].name.to_lowercase()
        ),
        Item::Clutch(i) => format!("{} clutch", CLUTCHES[i].0),
        Item::Suspension(i) => format!("{} suspension kit", SUSPENSIONS[i].0),
        Item::Gearbox(i) => format!("Gearbox: {}", GEARBOXES[i].name),
        Item::LimitedSlip => "Limited-slip differentials".to_string(),
    }
}

/// Fitting a part: what comes off (onto the shelf), and the engine's new parts if the
/// engine changes (its maps are needed before it's done, see `finish_engine`).
#[derive(Debug, PartialEq)]
pub struct Fitted {
    pub off: Option<Item>,
    pub engine: Option<Parts>,
}

/// Fits `item` to `car` (rims and tires to the front or rear `axle`, 0 or 1), or says why
/// it doesn't go on.
pub fn fit(car: &mut OwnedCar, item: Item, axle: usize) -> Result<Fitted, String> {
    let mut build = car.setup.build.clone();
    let mut parts = car.setup.engine.parts.clone();
    let mut new_tires = false;
    let off = match item {
        Item::Rims(rim) => {
            let wheel = &mut build.wheels[axle];
            if !wheel.tire.fits(&rim) {
                return Err(format!(
                    "Its {} tires don't go on {} rims: fit them with tires for them.",
                    wheel.tire.label(),
                    rim.label()
                ));
            }
            Some(Item::Rims(std::mem::replace(&mut wheel.rim, rim)))
        }
        Item::Tires(size, compound) => {
            let wheel = &mut build.wheels[axle];
            if !size.fits(&wheel.rim) {
                return Err(format!(
                    "{} tires don't go on its {} rims.",
                    size.label(),
                    wheel.rim.label()
                ));
            }
            (wheel.tire, wheel.compound) = (size, compound);
            new_tires = true;
            None
        }
        Item::Clutch(i) => Some(Item::Clutch(std::mem::replace(&mut build.clutch, i))),
        Item::Suspension(i) => {
            let old = build.suspension;
            build.set_suspension(i);
            Some(Item::Suspension(old))
        }
        Item::Gearbox(i) => {
            if !build.chassis().gearboxes().any(|g| g == i) {
                return Err(format!(
                    "The {} doesn't fit this car's layout.",
                    GEARBOXES[i].name
                ));
            }
            Some(Item::Gearbox(std::mem::replace(&mut build.gearbox, i)))
        }
        Item::LimitedSlip => {
            if build.limited_slip {
                return Err("It has limited-slip differentials already.".to_string());
            }
            build.limited_slip = true;
            None
        }
        Item::Ecu(model) => Some(Item::Ecu(std::mem::replace(&mut parts.ecu, model))),
        Item::Engine { slot, part } => {
            let old = std::mem::replace(&mut parts.chosen[slot], part);
            let off = Item::Engine { slot, part: old };
            // Taking a part off for nothing ("None") leaves nothing for the shelf.
            super::catalog::price(off).map(|_| off)
        }
    };
    if let Some(why) = build.wheel_misfit() {
        return Err(format!("{why}."));
    }
    let engine_changed = parts != car.setup.engine.parts;
    rebuild(car, build);
    if new_tires {
        fresh_treads(car, axle);
    }
    let engine = if !engine_changed {
        None
    } else if parts.ecu != car.setup.engine.parts.ecu && same_hardware(&parts, car) {
        // Only the ECU changed: same maps, same tune, another box.
        finish_engine(car, parts, car.setup.engine.build.maps.clone());
        None
    } else {
        Some(parts)
    };
    Ok(Fitted { off, engine })
}

/// Fits `rim`s with tires of `size` and `compound` on them to the front or rear `axle` at
/// once (when the tires on it don't go on the new rims): the old rims go on the shelf, the
/// old tires to the bin.
pub fn fit_wheels(
    car: &mut OwnedCar,
    axle: usize,
    rim: Rim,
    (size, compound): (TireSize, usize),
) -> Result<Fitted, String> {
    if !size.fits(&rim) {
        return Err(format!(
            "{} tires don't go on {} rims.",
            size.label(),
            rim.label()
        ));
    }
    let mut build = car.setup.build.clone();
    let wheel = Wheel {
        rim,
        tire: size,
        compound,
    };
    let old = std::mem::replace(&mut build.wheels[axle], wheel);
    if let Some(why) = build.wheel_misfit() {
        return Err(format!("{why}."));
    }
    rebuild(car, build);
    fresh_treads(car, axle);
    Ok(Fitted {
        off: Some(Item::Rims(old.rim)),
        engine: None,
    })
}

/// The car as `build`, with its engine as it is.
/// Rebuilds `car`'s setup with `build`, if that's another.
pub fn rebuild(car: &mut OwnedCar, build: CarBuild) {
    if build != car.setup.build {
        let engine = car.setup.engine.build.clone();
        car.setup = CarSetup::new(build, car.setup.engine.parts.clone(), engine);
    }
}

/// New treads on the front or rear `axle`.
fn fresh_treads(car: &mut OwnedCar, axle: usize) {
    for tread in &mut car.wear.treads[2 * axle..2 * axle + 2] {
        *tread = Tread::default();
    }
}

/// Whether `parts` make the same engine as the car's (apart from the ECU).
fn same_hardware(parts: &Parts, car: &OwnedCar) -> bool {
    let current = &car.setup.engine.build.hardware;
    maps::fingerprint(&parts.hardware()) == maps::fingerprint(current)
}

/// Puts the engine built from `parts` (with `maps` baked for it) into `car`. Its ECU keeps
/// its tune; only the box changes if the parts have another.
pub fn finish_engine(car: &mut OwnedCar, parts: Parts, maps: EngineMaps) {
    let old = &car.setup.engine.build;
    let mut ecu = old.ecu.clone();
    ecu.model = parts.ecu;
    let engine = EngineBuild {
        hardware: parts.hardware(),
        maps,
        ecu,
        octane: old.octane,
    };
    let build = car.setup.build.clone();
    car.setup = CarSetup::new(build, parts, engine);
    car.wear.fuel = car.wear.fuel.min(car.setup.tank.litres);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::career::Wear;
    use crate::physics::engine::ecu::EcuModel;
    use crate::physics::engine::parts::TURBO;
    use crate::physics::tire::{SPORT, STREET};

    fn coupe() -> OwnedCar {
        let setup = CarSetup::factory(3, 0);
        let wear = Wear::new(&setup);
        OwnedCar {
            setup,
            wear,
            age: 0.0,
        }
    }

    #[test]
    fn chassis_parts_swap_and_the_old_ones_go_on_the_shelf() {
        let mut car = coupe();
        let fitted = fit(&mut car, Item::Clutch(2), 0).unwrap();
        assert_eq!(fitted.off, Some(Item::Clutch(0)));
        assert_eq!(car.setup.build.clutch, 2);
        assert!(
            fit(&mut car, Item::Gearbox(0), 0).is_err(),
            "a transverse box"
        );
        // The coupé comes with open differentials.
        fit(&mut car, Item::LimitedSlip, 0).unwrap();
        assert!(car.setup.build.limited_slip);
        fit(&mut car, Item::LimitedSlip, 0).unwrap_err();
    }

    #[test]
    fn rims_and_tires_go_on_an_axle() {
        let mut car = coupe();
        let size = |width, aspect, rim| TireSize { width, aspect, rim };
        let rim = |diameter, width| Rim { diameter, width };
        car.wear.treads[0].set_wear(0.5);
        car.wear.treads[2].set_wear(0.5);
        // New tires at the front: fresh treads there, the worn ones go to the bin.
        let fitted = fit(&mut car, Item::Tires(size(225, 45, 17), SPORT), 0).unwrap();
        assert_eq!(fitted.off, None);
        let compounds = car.setup.build.wheels.map(|wheel| wheel.compound);
        assert_eq!(compounds, [SPORT, STREET]);
        assert_eq!(
            (car.wear.treads[0].wear(), car.wear.treads[2].wear()),
            (0.0, 0.5)
        );
        // Bigger rims need tires for them.
        assert!(fit(&mut car, Item::Rims(rim(18, 80)), 0).is_err());
        let fitted = fit_wheels(&mut car, 0, rim(18, 80), (size(235, 40, 18), SPORT)).unwrap();
        assert_eq!(fitted.off, Some(Item::Rims(rim(17, 75))));
        assert_eq!(car.setup.build.wheels[0].tire, size(235, 40, 18));
        assert_eq!(car.setup.front.tire.radius, size(235, 40, 18).radius());
        // Smaller ones don't clear the coupé's brakes, nor do tall tires fit the wells.
        assert!(fit_wheels(&mut car, 0, rim(15, 75), (size(225, 50, 15), SPORT)).is_err());
        assert!(fit(&mut car, Item::Tires(size(235, 55, 18), SPORT), 0).is_err());
        assert_eq!(car.setup.build.wheels[0].tire, size(235, 40, 18));
    }

    #[test]
    fn engine_parts_keep_the_tune() {
        let mut car = coupe();
        car.setup.engine.build.ecu.rev_limit = 5000.0;
        let fitted = fit(
            &mut car,
            Item::Engine {
                slot: TURBO,
                part: 2,
            },
            0,
        )
        .unwrap();
        // The turbo slot had none: nothing for the shelf.
        assert_eq!(fitted.off, None);
        let parts = fitted.engine.expect("a new engine");
        assert_eq!(parts.chosen[TURBO], 2);
        let maps = EngineMaps::for_hardware(&parts.hardware());
        finish_engine(&mut car, parts, maps);
        assert!(car.setup.engine.build.hardware.turbo.is_some());
        assert_eq!(car.setup.engine.build.ecu.rev_limit, 5000.0);
        // Another ECU box keeps the tune too, and the maps.
        let fitted = fit(&mut car, Item::Ecu(EcuModel::Programmable), 0).unwrap();
        assert_eq!(
            fitted,
            Fitted {
                off: Some(Item::Ecu(EcuModel::Stock)),
                engine: None
            }
        );
        assert_eq!(car.setup.engine.build.ecu.model, EcuModel::Programmable);
        assert_eq!(car.setup.engine.build.ecu.rev_limit, 5000.0);
    }
}
