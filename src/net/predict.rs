//! Our car against the server's word. The server drives our car from our inputs (see
//! `shadow`) and is in charge of it; our game drives it too, at once, so the driving never
//! waits for the server. Every step's input and our car's state after it are kept; when
//! the server's state of our car for a step comes and differs from ours, ours takes it up
//! and the steps since are driven again from the inputs kept, meeting the other cars as
//! they are as of each step (as the first time: see `client::meet_time`). Driven alike,
//! the two agree to the bit, so that only happens where the server knew more (other cars
//! it met, where they really were).
//!
//! While the server has our car (the rescue helicopter carrying it: `Yours::notice`), we
//! neither drive nor predict it: it's shown as the server sends it (`client::spectate`),
//! until the server gives it back as a move. Where the server moves our car (on joining, to an event's
//! spots and back), it says so with the state (`Yours::moved`), which is then taken up
//! whatever we had (our game moves the car alike, but the server's word goes). Whatever
//! else changes our car between steps (built anew in a shop, refuelled) is noticed by
//! comparing its state with what the last step left, and told to the server as a
//! placement (`Place`), which it takes up (trusted, until the server does those things
//! itself, but not where it would move the car). The server says with each state the
//! latest placement it had taken up; a state from before ours is stale.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::Connection;
use super::client::{SolidCars, Status, meet_again, others_at};
use super::protocol::Yours;
use crate::car::{ActiveCar, Car, Correction, DriverInput, Spectating, StepInput};
use crate::looks::Ghost;
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
    /// Our car's state as the last step left it.
    end: Option<Vec<u8>>,
    /// Each step's number, input and our car's state after it, the latest last.
    history: VecDeque<(u32, DriverInput, Vec<u8>)>,
    /// The latest step after which our car was put somewhere, and the placement to tell.
    placed: Option<u32>,
    pub(super) place: Option<(u32, Vec<u8>)>,
    /// The server's latest state of our car, and how many of its moves we took up.
    pub(super) yours: Option<Yours>,
    moved: u32,
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
    spectating: Option<Res<Spectating>>,
    mut cars: Query<&mut Car>,
) {
    if spectating.is_some_and(|s| s.0.is_some()) {
        return;
    }
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

/// At the end of a step: our car's state kept with the step's input.
pub fn note_step(
    mut prediction: ResMut<Prediction>,
    (step, chosen): (Res<StepInput>, Res<ActiveCar>),
    spectating: Option<Res<Spectating>>,
    mut cars: Query<&mut Car>,
) {
    if spectating.is_some_and(|s| s.0.is_some()) {
        return;
    }
    let Some(car) = active(cars.iter_mut(), &chosen) else {
        return;
    };
    let state = snapshot::save(&*car);
    let prediction = &mut *prediction;
    prediction
        .history
        .push_back((step.step, step.input, state.clone()));
    while prediction.history.len() > KEPT {
        prediction.history.pop_front();
    }
    prediction.end = Some(state);
}

/// Takes up the server's latest state of our car if it differs from ours for its step,
/// driving the steps since again. Not while nobody's at the wheel (our game changes the
/// car then, and tells the server), unless the server moved it. While the server has the
/// car, only notes that (and what it says about it).
pub fn reconcile(
    mut prediction: ResMut<Prediction>,
    connection: Res<Connection>,
    (step, chosen, mut spectating): (Res<StepInput>, Res<ActiveCar>, Option<ResMut<Spectating>>),
    (terrain, time, fixed): (Res<Terrain>, Res<Time<Real>>, Res<Time<Fixed>>),
    mut correction: Option<ResMut<Correction>>,
    (mut cars, remote): (Query<(&mut Car, Has<Ghost>)>, SolidCars),
) {
    let prediction = &mut *prediction;
    log(&mut prediction.report, time.elapsed_secs_f64());
    let Some(yours) = prediction.yours.take() else {
        return;
    };
    if !matches!(connection.status, Status::Joined { .. }) {
        return;
    }
    if let Some(spectating) = spectating.as_deref_mut()
        && spectating.0 != yours.notice
    {
        spectating.0.clone_from(&yours.notice);
    }
    if yours.notice.is_some() {
        prediction.history.clear();
        return;
    }
    let moved = yours.moved != prediction.moved;
    if !moved && (yours.placed != prediction.placed || step.input.away) {
        return;
    }
    // Ours for the same step (a move from before what we kept: from now on).
    let i = (prediction.history.iter()).position(|(kept, ..)| *kept == yours.step);
    if !moved && i.is_some_and(|i| prediction.history[i].2 == yours.state) {
        prediction.report.agreed += 1;
        return;
    }
    if !moved && i.is_none() {
        return;
    }
    let Some((mut car, ghost)) = (cars.iter_mut()).find(|(car, _)| car.index == chosen.0) else {
        return;
    };
    let (was, turned) = (car.body.position, car.body.rotation);
    if !snapshot::load(&mut *car, &yours.state) {
        // (A car the server moved that we haven't got yet, a lent one: again with its next
        // state.)
        if !moved {
            warn!("The server sent a state that isn't our car's");
        }
        return;
    }
    if moved {
        prediction.moved = yours.moved;
    }
    let i = match i {
        Some(i) => i,
        None => {
            prediction.history.clear();
            prediction.end = Some(yours.state);
            return;
        }
    };
    prediction.history[i].2 = yours.state;
    // (The crashes were heard as the steps were first driven.)
    let knocks = car.knocks;
    let heard = connection.server_time(super::stamp(&time, &fixed));
    for (step, input, kept) in prediction.history.iter_mut().skip(i + 1) {
        car.drive(input, STEP);
        step_car(&mut car, &terrain, STEP);
        if let Some(heard) = heard.filter(|_| !ghost) {
            meet_again(&mut car, &others_at(&connection, *step, heard, &remote));
        }
        *kept = snapshot::save(&*car);
    }
    car.knocks = knocks;
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
