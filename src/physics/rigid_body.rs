//! A minimal rigid body: accumulate forces and torques, then integrate with semi-implicit Euler.
//! Its position is a reference point fixed in the body, which its centre of mass may be
//! offset from (a load that changes, like the fuel).

use bevy::prelude::*;

#[derive(Clone, Debug)]
pub struct RigidBody {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    /// World-space angular velocity.
    pub angular_velocity: Vec3,
    pub mass: f32,
    /// Principal moments of inertia about the centre of mass, in body space (the body axes
    /// are taken as the principal axes).
    pub inertia: Vec3,
    /// The centre of mass, from the reference point in body space.
    pub center: Vec3,
    /// Forces, and torques about the reference point.
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
            center: Vec3::ZERO,
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

    /// Adds a force acting at the centre of mass (like gravity).
    pub fn add_force_at_center(&mut self, force: Vec3) {
        self.force += force;
        self.torque += (self.rotation * self.center).cross(force);
    }

    /// Changes the mass by `mass` at `at` (body space, from the reference point), and moves
    /// the centre of mass and the inertia with it (only the principal moments).
    pub fn load(&mut self, mass: f32, at: Vec3) {
        // Inertia of a point mass about the axes through a point `d` away from it.
        let point = |mass: f32, d: Vec3| {
            let s = d * d;
            Vec3::new(s.y + s.z, s.x + s.z, s.x + s.y) * mass
        };
        let total = self.mass + mass;
        let center = (self.center * self.mass + at * mass) / total;
        self.inertia += point(self.mass, center - self.center) + point(mass, center - at);
        // Removing mass from beyond the centre takes off inertia; it can't go negative.
        self.inertia = self.inertia.max(Vec3::splat(1e-3));
        self.mass = total;
        self.center = center;
    }

    /// Where the centre of mass is in the world.
    pub fn center_of_mass(&self) -> Vec3 {
        self.position + self.rotation * self.center
    }

    /// Velocity of the body's point at `point` (world).
    pub fn velocity_at(&self, point: Vec3) -> Vec3 {
        self.velocity + self.angular_velocity.cross(point - self.position)
    }

    /// The inverse inertia applied to a world-space vector.
    fn inverse_inertia(&self, v: Vec3) -> Vec3 {
        self.rotation * ((self.rotation.inverse() * v) / self.inertia)
    }

    /// How much an impulse along `normal` (unit) at `point` changes the point's speed
    /// along it, per unit of impulse (1/kg).
    pub fn inverse_mass_at(&self, point: Vec3, normal: Vec3) -> f32 {
        let arm = point - self.center_of_mass();
        1.0 / self.mass + normal.dot(self.inverse_inertia(arm.cross(normal)).cross(arm))
    }

    /// Changes the velocities at once by an `impulse` (N·s) at `point` (world).
    pub fn apply_impulse(&mut self, impulse: Vec3, point: Vec3) {
        let center = self.center_of_mass();
        let center_velocity = self.velocity_at(center) + impulse / self.mass;
        self.angular_velocity += self.inverse_inertia((point - center).cross(impulse));
        self.velocity = center_velocity - self.angular_velocity.cross(center - self.position);
    }

    /// The pose this body will have after `dt` if its velocities stay constant.
    pub fn extrapolate(&self, dt: f32) -> (Vec3, Quat) {
        (
            self.position + self.velocity * dt,
            rotate_by(self.rotation, self.angular_velocity * dt),
        )
    }

    #[cfg(test)]
    pub fn integrate(&mut self, dt: f32) {
        self.integrate_velocity(dt);
        self.integrate_position(dt);
    }

    /// The first half of a step: the forces change the velocities (contacts are resolved
    /// after this, before the body moves).
    pub fn integrate_velocity(&mut self, dt: f32) {
        // The centre of mass moves with the forces; the torque about it turns the body.
        let center = self.rotation * self.center;
        let center_velocity =
            self.velocity + self.angular_velocity.cross(center) + self.force / self.mass * dt;

        // Euler's equations in body space, including the gyroscopic term ω × Iω.
        let w = self.rotation.inverse() * self.angular_velocity;
        let tau = self.rotation.inverse() * (self.torque - center.cross(self.force));
        let w_dot = (tau - w.cross(self.inertia * w)) / self.inertia;
        self.angular_velocity = self.rotation * (w + w_dot * dt);

        self.velocity = center_velocity - self.angular_velocity.cross(center);
        self.force = Vec3::ZERO;
        self.torque = Vec3::ZERO;
    }

    /// The second half of a step: the body moves with its velocities.
    pub fn integrate_position(&mut self, dt: f32) {
        self.position += self.velocity * dt;
        self.rotation = rotate_by(self.rotation, self.angular_velocity * dt);
    }
}

/// Applies a world-space rotation vector (axis × angle) to an orientation.
fn rotate_by(rotation: Quat, rotation_vector: Vec3) -> Quat {
    (Quat::from_scaled_axis(rotation_vector) * rotation).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_load_moves_the_centre_of_mass() {
        let mut body = RigidBody::new(Vec3::ZERO, Quat::IDENTITY, 900.0, Vec3::splat(500.0));
        body.load(100.0, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(body.mass, 1000.0);
        assert!((body.center.z - 0.1).abs() < 1e-6);
        // 900 kg 0.1 m and 100 kg 0.9 m from the new centre.
        assert!((body.inertia.x - (500.0 + 9.0 + 81.0)).abs() < 1e-3);
        assert_eq!(body.inertia.z, 500.0);
        body.load(-100.0, Vec3::new(0.0, 0.0, 1.0));
        assert!(body.center.length() < 1e-6 && (body.inertia.x - 500.0).abs() < 1e-2);
    }

    #[test]
    fn a_force_through_the_centre_of_mass_doesnt_turn_it() {
        let mut body = RigidBody::new(Vec3::ZERO, Quat::IDENTITY, 900.0, Vec3::splat(500.0));
        body.load(100.0, Vec3::new(0.0, 0.0, 1.0));
        for _ in 0..100 {
            body.add_force_at_center(Vec3::new(0.0, -9810.0, 0.0));
            body.integrate(0.01);
        }
        assert!(
            body.angular_velocity.length() < 1e-5,
            "{}",
            body.angular_velocity
        );
        assert!((body.velocity.y + 9.81).abs() < 1e-3);
        // Pushed at the reference point instead, it turns.
        body.add_force(Vec3::new(1000.0, 0.0, 0.0));
        body.integrate(0.01);
        assert!(body.angular_velocity.y.abs() > 1e-3);
    }
}
