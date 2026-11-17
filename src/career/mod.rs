//! The career: start with nothing but mom's car, earn money with deliveries, buy cars and
//! parts. Its places (home, gas station, pizzeria, car dealer, parts shop) are on the map;
//! drive onto one's marker and stop to go in.
//!
//! Everything the player has is in `Career`, saved as text (`~/.config/car-sim/career/`,
//! local storage on the web) whenever something changes hands and in the garage.

pub mod catalog;
mod garage;
mod hud;
mod menus;
mod places;
mod save;

use bevy::prelude::*;

use crate::car::{ActiveCar, Car, CarSetup};
use crate::chassis::CarBuild;
use crate::physics::engine::EngineBuild;
use crate::physics::engine::damage::Damage;
use crate::physics::fuel::Fuel;
use crate::physics::tire::Tread;
use crate::terrain::Terrain;

pub use places::{PlaceKind, Places};

/// How many of their own cars fit into the player's garage (mom's has its own bay).
pub const GARAGE_BAYS: usize = 3;
/// What a new career starts with (€).
const START_MONEY: f64 = 500.0;
/// Mom's car: the saloon with its base engine, in silver.
const MOM_CHASSIS: usize = 4;
const MOM_PAINT: usize = 5;
/// The car the player drives has this index (see `ActiveCar`); a car being delivered for
/// the dealer has the other.
pub const PLAYER: usize = 0;
pub const DELIVERY: usize = 1;

pub struct CareerPlugin;

impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Career::load())
            .init_state::<Scene>()
            .add_plugins((
                places::PlacesPlugin,
                menus::MenusPlugin,
                hud::HudPlugin,
                garage::GaragePlugin,
            ))
            .add_systems(OnEnter(Scene::Driving), drive_out)
            .add_systems(OnExit(Scene::Driving), park);
    }
}

/// Out of the garage, on the road in front of home.
fn drive_out(
    mut commands: Commands,
    career: Res<Career>,
    places: Res<Places>,
    terrain: Res<Terrain>,
    mut active: ResMut<ActiveCar>,
) {
    let home = places.of(PlaceKind::Home);
    spawn_ride(&mut commands, &career, &terrain, home.at, home.yaw);
    active.0 = PLAYER;
}

/// Back into the garage: the car's wear is kept, every car on the road goes, and the
/// career is saved.
fn park(mut commands: Commands, mut career: ResMut<Career>, cars: Query<(Entity, &Car)>) {
    for (entity, car) in &cars {
        if car.index == PLAYER {
            career.bring_home(car);
        }
        commands.entity(entity).despawn();
    }
    career.save();
}

/// Where the player is: in the garage at home (where a career starts), or out driving.
#[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Scene {
    #[default]
    Garage,
    Driving,
}

/// Which car the player takes out of the garage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ride {
    Mom,
    Own(usize),
}

/// What wears on a car between drives: the engine, the fuel in the tank and the treads.
#[derive(Clone, Copy, Debug)]
pub struct Wear {
    pub damage: Damage,
    pub fuel: f32,
    pub treads: [Tread; 4],
}

impl Wear {
    /// A car as new, with a full tank.
    pub fn new(setup: &CarSetup) -> Self {
        Self {
            damage: Damage::default(),
            fuel: setup.tank.litres,
            treads: [Tread::default(); 4],
        }
    }

    pub fn of(car: &Car) -> Self {
        Self {
            damage: car.drivetrain.engine.damage,
            fuel: car.fuel.litres(),
            treads: car.corners.each_ref().map(|corner| corner.tread),
        }
    }

    pub fn put_on(&self, car: &mut Car) {
        car.drivetrain.engine.damage = self.damage;
        car.fuel = Fuel::new(self.fuel.min(car.setup.tank.litres));
        car.load_fuel();
        for (corner, tread) in car.corners.iter_mut().zip(self.treads) {
            corner.tread = tread;
        }
    }
}

/// One of the player's cars: how it's built (with its engine's tune) and its wear.
#[derive(Clone)]
pub struct OwnedCar {
    pub setup: CarSetup,
    pub wear: Wear,
}

/// A part bought at the parts shop, waiting in the garage to be fitted (or taken off a car).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    /// An engine part: a slot of `parts::SLOTS` and the part in it.
    Engine {
        slot: usize,
        part: usize,
    },
    Ecu(crate::physics::engine::ecu::EcuModel),
    /// A set of tires of a compound in `tire::COMPOUNDS`.
    Tires(usize),
    Clutch(usize),
    Suspension(usize),
    Gearbox(usize),
    LimitedSlip,
}

/// Everything the player has and has done.
#[derive(Resource)]
pub struct Career {
    pub money: f64,
    /// How much the car dealer trusts the player with its cars, 0 to 100.
    pub trust: f32,
    pub cars: Vec<OwnedCar>,
    pub items: Vec<Item>,
    /// The car taken out of the garage (or to be).
    pub ride: Ride,
    pub pizzas: u32,
    pub deliveries: u32,
    /// Mom's car as it always is: full and in one piece.
    pub mom: CarSetup,
}

impl Career {
    fn new() -> Self {
        Self {
            money: START_MONEY,
            trust: 0.0,
            cars: Vec::new(),
            items: Vec::new(),
            ride: Ride::Mom,
            pizzas: 0,
            deliveries: 0,
            mom: mom_car(),
        }
    }

    /// The saved career, or a new one.
    fn load() -> Self {
        let mut career = Self::new();
        if let Some(text) = save::load() {
            save::read(&mut career, &text);
        }
        career
    }

    pub fn save(&self) {
        if let Err(problem) = save::store(&save::write(self)) {
            warn!("Couldn't save the career: {problem}");
        }
    }

    /// Takes `price` if there's enough money.
    pub fn pay(&mut self, price: f64) -> bool {
        let enough = self.money >= price;
        if enough {
            self.money -= price;
        }
        enough
    }

    /// The setup and wear of the car the player takes out.
    pub fn ride_car(&self) -> (CarSetup, Wear) {
        match self.ride {
            Ride::Own(i) if i < self.cars.len() => {
                let car = &self.cars[i];
                (car.setup.clone(), car.wear)
            }
            _ => (self.mom.clone(), Wear::new(&self.mom)),
        }
    }

    /// Keeps what wore on the player's car while it was out (mom's is looked after by mom).
    pub fn bring_home(&mut self, car: &Car) {
        if let Ride::Own(i) = self.ride
            && let Some(owned) = self.cars.get_mut(i)
        {
            owned.wear = Wear::of(car);
        }
    }

    /// Whether there's a free bay in the garage for another car.
    pub fn has_room(&self) -> bool {
        self.cars.len() < GARAGE_BAYS
    }
}

fn mom_car() -> CarSetup {
    let build = CarBuild {
        paint: MOM_PAINT,
        ..CarBuild::factory(MOM_CHASSIS)
    };
    let parts = build.factory_engine(0);
    let engine = EngineBuild::new(parts.hardware(), parts.ecu);
    CarSetup::new(build, parts, engine)
}

/// The player's car, as it comes out, put on the road at `at` facing along `yaw`.
pub fn spawn_ride(commands: &mut Commands, career: &Career, terrain: &Terrain, at: Vec2, yaw: f32) {
    let (setup, wear) = career.ride_car();
    let mut car = Car::spawn_at(PLAYER, setup, terrain, at.x, at.y, yaw);
    wear.put_on(&mut car);
    car.drivetrain.automatic = true;
    commands.spawn((Transform::from_translation(car.body.position), car));
}

/// The player's car's entity and the car (the one they own or mom's, not a car being
/// delivered).
pub fn player_car<'a>(cars: &'a mut Query<(Entity, &mut Car)>) -> Option<(Entity, Mut<'a, Car>)> {
    cars.iter_mut().find(|(_, car)| car.index == PLAYER)
}

/// The car being driven, whichever it is.
pub fn driven<'a>(cars: &'a Query<&Car>, active: &ActiveCar) -> Option<&'a Car> {
    cars.iter().find(|car| car.index == active.0)
}

/// Money as shown: whole euros with thousands separators.
pub fn euros(amount: f64) -> String {
    let whole = amount.round() as i64;
    let digits = whole.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if whole < 0 { "-" } else { "" };
    format!("{sign}€{grouped}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_well() {
        assert_eq!(euros(0.0), "€0");
        assert_eq!(euros(999.4), "€999");
        assert_eq!(euros(1234.0), "€1,234");
        assert_eq!(euros(-1234567.0), "-€1,234,567");
    }
}
