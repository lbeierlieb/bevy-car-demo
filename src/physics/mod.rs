//! Vehicle physics, independent of Bevy's ECS: a rigid-body chassis, double A-arm corners,
//! a Magic Formula tire, and an engine with clutch and gearbox.

pub mod drivetrain;
pub mod rigid_body;
pub mod suspension;
pub mod tire;
