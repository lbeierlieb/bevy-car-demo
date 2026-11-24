//! Our car against the server's word. The server drives our car from our inputs (see
//! `shadow`) and is in charge of it; our game drives it too, at once, so the driving never
//! waits for the server. Every step's input and our car's state after it are kept; when
//! the server's state of our car for a step comes and differs from ours, ours takes it up
//! and the steps since are driven again from the inputs kept. Driven alike, the two agree
//! to the bit, so that only happens where the server knew more (other cars it met).
//!
//! Whatever changes our car outside the simulation (put on the road, carried by the
//! rescue helicopter, built anew in a shop, refuelled) is noticed by comparing its state
//! with what the last step left, and told to the server as a placement (`Place`), which it
//! takes up (trusted, until the server does those things itself). The server says with
//! each state the latest placement it had taken up; a state from before ours is stale.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::Connection;
use super::client::Status;
use crate::car::{ActiveCar, Car, Correction, DriverInput, StepInput};
use crate::sim::{STEP, step_car};
use crate::snapshot;
use crate::terrain::Terrain;

/// Steps of inputs and states kept (two seconds: the server's word comes within a round
/// trip and its margin).
const KEPT: usize = 240;
/// How often corrections are logged (s).
const REPORT: f64 = 10.0;

/// Our car's prediction (see the module's docs).
#[derive(Resource, Default)]
pub struct Prediction {
    /// Our car's state as the last step left it, and as the simulation left it in the
    /// step under way (before what runs after it, the rescue helicopter).
    end: Option<Vec<u8>>,
    simulated: Option<Vec<u8>>,
    /// Each step's number, input and our car's state after it, the latest last.
    history: VecDeque<(u32, DriverInput, Vec<u8>)>,
    /// The latest step after which our car was put somewhere, and the placement to tell.
    placed: Option<u32>,
    pub(super) place: Option<(u32, Vec<u8>)>,
    /// The server's latest state of our car: its step, the latest placement it had taken
    /// up, and the state.
    pub(super) yours: Option<(u32, Option<u32>, Vec<u8>)>,
    report: Report,
}

#[derive(Default)]
struct Report {
    since: f64,
    /// States from the server that agreed with ours, and ones that didn't, and how far
    /// the farthest correction moved our car (m).
    agreed: usize,
    corrected: usize,
    farthest: f32,
}

impl Prediction {
    /// The latest step after which our car was put somewhere.
    pub fn placed(&self) -> Option<u32> {
        self.placed
    }

    /// How many of the server's states differed from ours, of how many compared, since
    /// the last log (for tests).
    #[cfg(test)]
    pub fn corrections(&self) -> (usize, usize) {
        let report = &self.report;
        (report.corrected, report.corrected + report.agreed)
    }
}

/// The active car.
fn active<'a>(
    cars: impl Iterator<Item = Mut<'a, Car>>,
    active: &ActiveCar,
) -> Option<Mut<'a, Car>> {
    cars.into_iter().find(|car| car.index == active.0)
}

/// At the start of a step: if our car isn't as the last step left it, something put it
/// somewhere between steps (after step `step - 1`).
pub fn notice_placing(
    mut prediction: ResMut<Prediction>,
    (step, chosen): (Res<StepInput>, Res<ActiveCar>),
    mut cars: Query<&mut Car>,
) {
    let Some(car) = active(cars.iter_mut(), &chosen) else {
        return;
    };
    let state = snapshot::save(&*car);
    if prediction.end.as_ref() != Some(&state) {
        let before = step.step.wrapping_sub(1);
        prediction.placed = Some(before);
        prediction.place = Some((before, state.clone()));
        prediction.end = Some(state);
    }
}

/// Right after the simulation in a step.
pub fn note_simulated(
    mut prediction: ResMut<Prediction>,
    chosen: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
) {
    prediction.simulated = active(cars.iter_mut(), &chosen).map(|car| snapshot::save(&*car));
}

/// At the end of a step: our car's state kept with the step's input; if something after
/// the simulation changed it (the rescue helicopter), it was put there.
pub fn note_step(
    mut prediction: ResMut<Prediction>,
    (step, chosen): (Res<StepInput>, Res<ActiveCar>),
    mut cars: Query<&mut Car>,
) {
    let Some(car) = active(cars.iter_mut(), &chosen) else {
        return;
    };
    let state = snapshot::save(&*car);
    let prediction = &mut *prediction;
    if prediction.simulated.as_ref() != Some(&state) {
        prediction.placed = Some(step.step);
        prediction.place = Some((step.step, state.clone()));
    }
    prediction
        .history
        .push_back((step.step, step.input, state.clone()));
    while prediction.history.len() > KEPT {
        prediction.history.pop_front();
    }
    prediction.end = Some(state);
}

/// Takes up the server's latest state of our car if it differs from ours for its step,
/// driving the steps since again. Not while nobody's at the wheel (our game moves the car
/// then, and tells the server where).
pub fn reconcile(
    mut prediction: ResMut<Prediction>,
    connection: Res<Connection>,
    (step, chosen): (Res<StepInput>, Res<ActiveCar>),
    terrain: Res<Terrain>,
    time: Res<Time<Real>>,
    mut correction: Option<ResMut<Correction>>,
    mut cars: Query<&mut Car>,
) {
    let prediction = &mut *prediction;
    log(&mut prediction.report, time.elapsed_secs_f64());
    let Some((at, placed, state)) = prediction.yours.take() else {
        return;
    };
    if !matches!(connection.status, Status::Joined { .. })
        || placed != prediction.placed
        || step.input.away
    {
        return;
    }
    // Ours for the same step.
    let Some(i) = (prediction.history.iter()).position(|(kept, ..)| *kept == at) else {
        return;
    };
    if prediction.history[i].2 == state {
        prediction.report.agreed += 1;
        return;
    }
    let Some(mut car) = active(cars.iter_mut(), &chosen) else {
        return;
    };
    let (was, turned) = (car.body.position, car.body.rotation);
    if !snapshot::load(&mut *car, &state) {
        warn!("The server sent a state that isn't our car's");
        return;
    }
    prediction.history[i].2 = state;
    for (_, input, kept) in prediction.history.iter_mut().skip(i + 1) {
        car.drive(input, STEP);
        step_car(&mut car, &terrain, STEP);
        *kept = snapshot::save(&*car);
    }
    prediction.end = prediction.history.back().map(|(.., state)| state.clone());
    if let Some(correction) = correction.as_deref_mut() {
        correction.moved(&car, was, turned);
    }
    let report = &mut prediction.report;
    report.corrected += 1;
    report.farthest = report.farthest.max(was.distance(car.body.position));
}

fn log(report: &mut Report, now: f64) {
    if now - report.since < REPORT {
        return;
    }
    if report.corrected > 0 {
        info!(
            "Last {REPORT} s: the server corrected our car {} times (agreed {} times), \
             moving it up to {:.2} m",
            report.corrected, report.agreed, report.farthest
        );
    }
    *report = Report {
        since: now,
        ..Report::default()
    };
}
