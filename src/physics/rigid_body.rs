//! A minimal rigid body: accumulate forces and torques, then integrate with semi-implicit Euler.

use bevy::prelude::*;

#[derive(Clone, Debug)]
pub struct RigidBody {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    /// World-space angular velocity.
    pub angular_velocity: Vec3,
    pub mass: f32,
    /// Principal moments of inertia in body space (the body axes are the principal axes).
    pub inertia: Vec3,
    force: Vec3,
    torque: Vec3,
}

impl RigidBody {
    pub fn new(position: Vec3, rotation: Quat, mass: f32, inertia: Vec3) -> Self {
        Self {
            position,
            rotation,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            mass,
            inertia,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        }
    }

    /// Inertia of a solid box with the given full extents.
    pub fn box_inertia(mass: f32, size: Vec3) -> Vec3 {
        let s = size * size;
        Vec3::new(s.y + s.z, s.x + s.z, s.x + s.y) * (mass / 12.0)
    }

    pub fn add_force(&mut self, force: Vec3) {
        self.force += force;
    }

    pub fn add_torque(&mut self, torque: Vec3) {
        self.torque += torque;
    }

    /// The pose this body will have after `dt` if its velocities stay constant.
    pub fn extrapolate(&self, dt: f32) -> (Vec3, Quat) {
        (
            self.position + self.velocity * dt,
            rotate_by(self.rotation, self.angular_velocity * dt),
        )
    }

    pub fn integrate(&mut self, dt: f32) {
        self.velocity += self.force / self.mass * dt;

        // Euler's equations in body space, including the gyroscopic term ω × Iω.
        let w = self.rotation.inverse() * self.angular_velocity;
        let tau = self.rotation.inverse() * self.torque;
        let w_dot = (tau - w.cross(self.inertia * w)) / self.inertia;
        self.angular_velocity = self.rotation * (w + w_dot * dt);

        self.position += self.velocity * dt;
        self.rotation = rotate_by(self.rotation, self.angular_velocity * dt);
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }
}

/// Applies a world-space rotation vector (axis × angle) to an orientation.
fn rotate_by(rotation: Quat, rotation_vector: Vec3) -> Quat {
    (Quat::from_scaled_axis(rotation_vector) * rotation).normalize()
}
