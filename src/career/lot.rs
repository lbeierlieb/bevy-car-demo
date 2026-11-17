//! The dealer's lot: a few cars for sale, new from the factory or used (older, worn, and a
//! lot cheaper), made up again after every job. Buying one puts the player in it at the
//! dealer, and the car they came in goes home into the garage; the dealer also buys the
//! car the player came in.

use bevy::prelude::*;

use super::jobs::Rng;
use super::{Career, OwnedCar, PLAYER, Ride, Wear, catalog, spawn_ride};
use crate::car::{ActiveCar, Car, CarSetup};
use crate::chassis::{CHASSIS, CarBuild, PAINTS};
use crate::physics::engine::EngineBuild;
use crate::terrain::Terrain;

/// How many cars are for sale, how many of them used, and of those how many are cheap
/// runabouts (city car, van, pickup, saloon).
const FOR_SALE: usize = 7;
const USED: usize = 5;
const CHEAP: usize = 2;
const RUNABOUTS: [usize; 4] = [0, 12, 11, 4];
/// The dealer sells for this much more than a car is worth, and buys for this share.
const MARGIN: f64 = 1.2;
const BUYS_FOR: f64 = 0.8;

pub struct ForSale {
    pub car: OwnedCar,
    pub price: f64,
}

impl ForSale {
    pub fn used(&self) -> bool {
        self.car.age > 0.0
    }
}

/// The cars for sale, and the number of jobs done when they were put out.
#[derive(Resource, Default)]
pub struct Lot {
    pub cars: Vec<ForSale>,
    jobs: Option<u32>,
}

impl Lot {
    /// Puts out new cars if there were jobs done since the last time.
    pub fn restock(&mut self, career: &Career, rng: &mut Rng) {
        let jobs = career.pizzas + career.deliveries;
        if self.jobs == Some(jobs) {
            return;
        }
        self.jobs = Some(jobs);
        self.cars = (0..FOR_SALE)
            .map(|i| {
                let chassis = if i < CHEAP {
                    RUNABOUTS[rng.below(RUNABOUTS.len())]
                } else {
                    rng.below(CHASSIS.len())
                };
                let used = i < USED;
                offer(rng, chassis, used)
            })
            .collect();
        self.cars.sort_by(|a, b| a.price.total_cmp(&b.price));
    }
}

/// A car of `chassis` for sale: new, or used (old and worn).
fn offer(rng: &mut Rng, chassis: usize, used: bool) -> ForSale {
    let option = rng.below(CHASSIS[chassis].engines.len());
    let build = CarBuild {
        paint: rng.below(PAINTS.len()),
        ..CarBuild::factory(chassis)
    };
    let parts = build.factory_engine(option);
    let engine = EngineBuild::new(parts.hardware(), parts.ecu);
    let setup = CarSetup::new(build, parts, engine);
    let mut wear = Wear::new(&setup);
    let mut age = 0.0;
    if used {
        age = f64::from(0.3 + 0.65 * rng.next());
        let tired = 0.25 * rng.next() * age as f32;
        for cylinder in &mut wear.damage.cylinders {
            cylinder.piston = tired * (0.5 + rng.next());
            cylinder.valves = 0.5 * tired * rng.next();
            cylinder.rod = 0.5 * tired * rng.next();
        }
        wear.damage.head_gasket = tired * rng.next();
        wear.damage.bearings = tired * rng.next();
        if setup.engine.build.hardware.turbo.is_some() {
            wear.damage.turbo = tired * rng.next();
        }
        for tread in &mut wear.treads {
            tread.wear = 0.2 + 0.5 * rng.next();
        }
        wear.fuel = setup.tank.litres * (0.1 + 0.3 * rng.next());
    }
    let price = if used {
        catalog::worth(&setup, &wear, age) * MARGIN
    } else {
        catalog::new_price(chassis, option)
    };
    ForSale {
        car: OwnedCar { setup, wear, age },
        price: price.round(),
    }
}

/// What the dealer pays for a car.
pub fn trade_in(car: &OwnedCar) -> f64 {
    (catalog::worth(&car.setup, &car.wear, car.age) * BUYS_FOR).round()
}

/// Buys the car for sale at `index` and puts the player in it in place of `player` (the
/// car they came in, which goes home). Gives the bought car's name, or nothing if it
/// can't be bought.
pub fn buy(
    lot: &mut Lot,
    index: usize,
    career: &mut Career,
    player: (Entity, &Car),
    commands: &mut Commands,
    terrain: &Terrain,
    active: &mut ActiveCar,
) -> Option<&'static str> {
    let price = lot.cars.get(index)?.price;
    if !career.has_room() || !career.pay(price) {
        return None;
    }
    let bought = lot.cars.remove(index).car;
    let name = bought.setup.name;
    let (entity, car) = player;
    career.bring_home(car);
    let (at, yaw) = (
        car.body.position.xz(),
        crate::car::heading(car.body.rotation),
    );
    commands.entity(entity).despawn();
    career.cars.push(bought);
    career.ride = Ride::Own(career.cars.len() - 1);
    spawn_ride(commands, career, terrain, at, yaw);
    active.0 = PLAYER;
    career.save();
    Some(name)
}

/// Sells the car the player came in (one of theirs) and gives them mom's to get home in.
/// Gives what it fetched.
pub fn sell(
    career: &mut Career,
    player: (Entity, &Car),
    commands: &mut Commands,
    terrain: &Terrain,
    active: &mut ActiveCar,
) -> Option<f64> {
    let Ride::Own(i) = career.ride else {
        return None;
    };
    let (entity, car) = player;
    career.bring_home(car);
    let price = trade_in(career.cars.get(i)?);
    career.cars.remove(i);
    career.money += price;
    career.ride = Ride::Mom;
    let (at, yaw) = (
        car.body.position.xz(),
        crate::car::heading(car.body.rotation),
    );
    commands.entity(entity).despawn();
    spawn_ride(commands, career, terrain, at, yaw);
    active.0 = PLAYER;
    career.save();
    Some(price)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lot_has_cheap_cars_and_trading_loses_money() {
        let career = Career::new();
        let mut rng = Rng(12345);
        let mut lot = Lot::default();
        lot.restock(&career, &mut rng);
        assert_eq!(lot.cars.len(), FOR_SALE);
        assert!(lot.cars.windows(2).all(|w| w[0].price <= w[1].price));
        assert!(
            lot.cars[0].price < 3_500.0,
            "cheapest {}",
            lot.cars[0].price
        );
        for sale in &lot.cars {
            // Bought and sold straight back, it's always a loss.
            assert!(trade_in(&sale.car) < sale.price, "{}", sale.car.setup.name);
        }
        // The same jobs done, the same cars.
        let first = lot.cars[0].price;
        lot.restock(&career, &mut rng);
        assert_eq!(lot.cars[0].price, first);
    }
}
