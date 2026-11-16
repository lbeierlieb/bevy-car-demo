//! Engine temperatures: the coolant (with the block) and the oil (with the sump) as two
//! lumps. The cylinders heat the coolant, friction heats both, the oil gives heat to the
//! coolant, and the radiator takes it away once the thermostat opens, cooled by the air
//! the car drives through and by its fan.

use super::AMBIENT_TEMPERATURE;
use super::hardware::Cooling;

/// Heat passed from the oil to the coolant, and lost from the block and the sump to the
/// air around them (W/K).
const OIL_TO_COOLANT: f32 = 600.0;
const BLOCK_LOSS: f32 = 15.0;
const SUMP_LOSS: f32 = 10.0;
/// Share of the driving speed that goes through the radiator, and the least air through
/// it when stopped with the fan off (m/s).
const RAM_SHARE: f32 = 0.4;
const STILL_AIR: f32 = 0.3;
/// Coolant flow past a closed thermostat (its bypass), as a share of fully open.
const BYPASS: f32 = 0.02;
/// Air density (kg/m³) and specific heat (J/(kg·K)).
const AIR_DENSITY: f32 = 1.18;
const AIR_HEAT: f32 = 1005.0;

#[derive(Clone, Copy, Debug)]
pub struct Thermal {
    /// Coolant and oil temperatures (K).
    pub coolant: f32,
    pub oil: f32,
    pub fan: bool,
}

impl Thermal {
    /// At running temperature.
    pub fn warm() -> Self {
        Self {
            coolant: 363.0,
            oil: 368.0,
            fan: false,
        }
    }

    /// Advances by `dt` with heat flowing into the coolant and oil (W) and the car driving
    /// at `airspeed` (m/s).
    pub fn step(&mut self, cooling: &Cooling, to_coolant: f32, to_oil: f32, airspeed: f32, dt: f32) {
        if self.coolant > cooling.fan_on {
            self.fan = true;
        } else if self.coolant < cooling.fan_off {
            self.fan = false;
        }
        let air_speed =
            (RAM_SHARE * airspeed.abs() + if self.fan { cooling.fan_speed } else { 0.0 })
                .max(STILL_AIR);
        let air = AIR_DENSITY * cooling.radiator_area * air_speed * AIR_HEAT;
        let opening = ((self.coolant - cooling.thermostat_opens)
            / (cooling.thermostat_open - cooling.thermostat_opens))
            .clamp(BYPASS, 1.0);
        let radiator = cooling.radiator_effectiveness * air * opening;
        let exchange = OIL_TO_COOLANT * (self.oil - self.coolant);
        let coolant_out = (radiator + BLOCK_LOSS) * (self.coolant - AMBIENT_TEMPERATURE);
        let oil_out = SUMP_LOSS * (1.0 + 0.05 * airspeed.abs()) * (self.oil - AMBIENT_TEMPERATURE);
        self.coolant += (to_coolant + exchange - coolant_out) / cooling.coolant_capacity * dt;
        self.oil += (to_oil - exchange - oil_out) / cooling.oil_capacity * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermostat_holds_running_temperature() {
        let cooling = Cooling::default();
        // Hard driving (about 70 kW into the coolant) at 30 m/s, and idling (2 kW)
        // standing still: either way the coolant settles between the thermostat and the
        // fan.
        for (heat, speed) in [(70e3, 30.0), (2e3, 0.0)] {
            let mut thermal = Thermal::warm();
            for _ in 0..(1200.0 * 10.0) as usize {
                thermal.step(&cooling, heat, heat * 0.1, speed, 0.1);
            }
            let celsius = thermal.coolant - 273.15;
            assert!((85.0..=105.0).contains(&celsius), "{celsius} °C at {heat} W");
            // Working hard, the oil runs hotter than the coolant; idling, a little cooler.
            let oil = thermal.oil - thermal.coolant;
            assert!(if heat > 10e3 { oil > 0.0 } else { oil > -10.0 }, "oil {oil} K over");
        }
    }

    #[test]
    fn warms_up_from_cold() {
        let cooling = Cooling::default();
        let mut thermal = Thermal {
            coolant: AMBIENT_TEMPERATURE,
            oil: AMBIENT_TEMPERATURE,
            fan: false,
        };
        // Ten minutes of moderate driving (15 kW into the coolant).
        for _ in 0..6000 {
            thermal.step(&cooling, 15e3, 2e3, 20.0, 0.1);
        }
        let celsius = thermal.coolant - 273.15;
        assert!((70.0..=100.0).contains(&celsius), "{celsius} °C");
    }
}
