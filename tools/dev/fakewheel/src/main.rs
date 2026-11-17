// Virtual steering wheel for testing car-sim: scripted axes and buttons, logs force feedback.
use evdev::{
    uinput::VirtualDevice, AbsInfo, AbsoluteAxisCode, AttributeSet, EventSummary, EventType,
    FFEffectCode, FFEffectKind, InputEvent, KeyCode, UInputCode, UinputAbsSetup,
};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::os::fd::{AsFd, AsRawFd};
use std::time::{Duration, Instant};

fn main() -> std::io::Result<()> {
    let axis = |code, min, max| UinputAbsSetup::new(code, AbsInfo::new(0, min, max, 0, 0, 1));
    let mut device = VirtualDevice::builder()?
        .name("Fake Wheel")
        .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_X, -32768, 32767))?
        .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Y, 0, 65535))?
        .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Z, 0, 65535))?
        .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_RZ, 0, 65535))?
        .with_keys(&AttributeSet::from_iter([
            KeyCode::BTN_SOUTH,
            KeyCode::BTN_EAST,
            KeyCode::BTN_NORTH,
            KeyCode::BTN_WEST,
            KeyCode::BTN_TRIGGER,
            KeyCode::BTN_THUMB,
        ]))?
        .with_ff(&AttributeSet::from_iter([FFEffectCode::FF_CONSTANT]))?
        .with_ff_effects_max(16)
        .build()?;
    std::thread::sleep(Duration::from_millis(500));
    for path in device.enumerate_dev_nodes_blocking()? {
        let path = path?;
        println!("node {}", path.display());
        std::process::Command::new("chmod")
            .arg("666")
            .arg(&path)
            .status()?;
    }
    let start = Instant::now();
    let mut next_id = 0i16;
    let mut last_log = 0.0;
    loop {
        let t = start.elapsed().as_secs_f32();
        // Script: clutch in and first gate held; let the clutch out with throttle from 25 s;
        // steer left from 40 s.
        let clutch = if t < 40.0 {
            1.0
        } else {
            (1.0 - (t - 40.0) / 4.0).max(0.0)
        };
        let throttle = if t < 40.0 { 0.0 } else { 0.6 };
        let steer = if t < 90.0 { 0.0 } else { 0.06 };
        // Wiggle every axis once, so the game hears a value from each.
        let wiggle = if (20.0..21.0).contains(&t) { 100 } else { 0 };
        let ev = |code: AbsoluteAxisCode, v: i32| {
            InputEvent::new(
                EventType::ABSOLUTE.0,
                code.0,
                if v > 30000 { v - wiggle } else { v + wiggle },
            )
        };
        device.emit(&[
            ev(AbsoluteAxisCode::ABS_X, (steer * 32767.0) as i32),
            ev(AbsoluteAxisCode::ABS_Y, (throttle * 65535.0) as i32),
            ev(AbsoluteAxisCode::ABS_Z, 0),
            ev(AbsoluteAxisCode::ABS_RZ, (clutch * 65535.0) as i32),
            InputEvent::new(
                EventType::KEY.0,
                KeyCode::BTN_WEST.0,
                (t < 30.0 || t > 32.0) as i32,
            ),
        ])?;
        let fd = device.as_raw_fd();
        let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
        let mut fds = [PollFd::new(borrowed.as_fd(), PollFlags::POLLIN)];
        if poll(&mut fds, PollTimeout::from(20u16)).unwrap_or(0) > 0 {
            let events: Vec<InputEvent> = device.fetch_events()?.collect();
            for event in events {
                match event.destructure() {
                    EventSummary::UInput(event, UInputCode::UI_FF_UPLOAD, ..) => {
                        let mut upload = device.process_ff_upload(event)?;
                        if upload.effect_id() < 0 {
                            upload.set_effect_id(next_id);
                            next_id += 1;
                        }
                        upload.set_retval(0);
                        if let FFEffectKind::Constant { level, .. } = upload.effect().kind {
                            if t - last_log > 0.5 || level.abs() > 30000 {
                                println!(
                                    "t {t:5.1} constant level {level} dir {:#x}",
                                    upload.effect().direction
                                );
                                last_log = t;
                            }
                        }
                    }
                    EventSummary::UInput(event, UInputCode::UI_FF_ERASE, ..) => {
                        let erase = device.process_ff_erase(event)?;
                        println!("t {t:5.1} erase {}", erase.effect_id());
                    }
                    EventSummary::ForceFeedback(_, code, value) => {
                        println!("t {t:5.1} play effect {} value {value}", code.0);
                    }
                    _ => {}
                }
            }
        }
        if t > 400.0 {
            return Ok(());
        }
    }
}
