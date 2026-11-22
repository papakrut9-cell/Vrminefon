//! Phone VR: the phone's own sensors drive the in-game head, and the renderer (a later
//! phase, docs/phonevr/PLAN.md) draws the two eyes side by side on the screen so
//! Cardboard-style glasses show it in 3D. Where the sensor crate for standalone
//! headsets goes through OpenXR, here the head comes straight from the sensor HAL.
//!
//! The sensor thread reads the phone's (game) rotation vector through the NDK's
//! `ASensor*` calls — no JNI, no Java — and hands out the latest pose to
//! [`head`]. The first reading, and every [`recenter`], becomes "straight ahead";
//! the pose handed out is relative to that.
//!
//! Android only; other targets compile an empty crate.
#![cfg(target_os = "android")]

use glam::Quat;
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

/// The game rotation vector, preferred over the plain one: its heading is only
/// relative to where it started (no magnetometer), which is what a recenter does
/// anyway, and it does not swing when the phone's compass is confused.
const SENSOR_GAME_ROTATION_VECTOR: c_int = 16;
/// The rotation vector, when the phone has no game rotation vector.
const SENSOR_ROTATION_VECTOR: c_int = 15;
/// How often the sensor delivers, in microseconds: 100 Hz, above any phone display.
const RATE_US: c_int = 10_000;
/// The identity the event queue is created under, so [`ALooper_pollAll`] hands its
/// events back without any callbacks.
const QUEUE_IDENT: c_int = 3;
/// The sensor loop quits after this many queue errors in a row: losing the sensor
/// service is the only way to fail like this, and spinning on it burns the battery.
const MAX_ERRORS: u32 = 5;

/// Where the head is, as the sensors last saw it.
#[derive(Clone, Copy)]
pub struct HeadPose {
    /// Turning from where the head was recentered to now, as the sensor's quaternion
    /// (x, y, z, w) in the phone's sensor frame. The renderer turns this into the
    /// camera view; the phone lies sideways in the glasses, so that rotation is a
    /// landscape quarter turn the renderer folds in (later work).
    pub rotation: Quat,
    /// The sensor's accuracy field, radians (the game rotation vector reports none: 0).
    pub accuracy: f32,
}

#[derive(Clone, Copy, Default)]
struct State {
    /// The latest quaternion and its accuracy.
    latest: Option<(Quat, f32)>,
    /// Where the head was when it was first seen or last recentered.
    origin: Quat,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether the phone-sensor head tracking has started. False without
/// [`start`] (the flat game), or after the sensor gave up.
pub fn active() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// The latest head pose, relative to where the head was recentered.
pub fn head() -> Option<HeadPose> {
    if !RUNNING.load(Ordering::Relaxed) {
        return None;
    }
    let state = lock_state();
    let (rotation, accuracy) = state.latest?;
    Some(HeadPose {
        rotation: state.origin.conjugate() * rotation,
        accuracy,
    })
}

/// Make the current head pose "straight ahead".
pub fn recenter() {
    if RUNNING.load(Ordering::Relaxed) {
        let mut state = lock_state();
        state.origin = state.latest.map_or(Quat::IDENTITY, |(q, _)| q);
    }
}

/// Start the sensor thread. Only once; the game calls it when
/// `HORIZON_PHONEVR=1` chooses the phone-VR picture over the flat one.
pub fn start() {
    if RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = STATE.set(Mutex::new(State::default()));
    if let Err(error) = std::thread::Builder::new()
        .name("Phone VR".into())
        .spawn(sensor_loop)
    {
        log::error!("Phone VR: the sensor thread did not start ({error})");
        RUNNING.store(false, Ordering::Relaxed);
    }
}

fn lock_state() -> MutexGuard<'static, State> {
    let state = STATE.get().expect("Phone VR: head() before start()");
    match state.lock() {
        Ok(state) => state,
        Err(poisoned) => PoisonError::into_inner(poisoned),
    }
}

fn sensor_loop() {
    unsafe {
        let manager = ASensorManager_getInstanceForPackage(
            b"dev.horizonlab.horizon\0".as_ptr().cast::<c_char>(),
        );
        if manager.is_null() {
            log::warn!("Phone VR: the sensor manager did not start; running flat");
            RUNNING.store(false, Ordering::Relaxed);
            return;
        }
        let sensor = ASensorManager_getDefaultSensor(manager, SENSOR_GAME_ROTATION_VECTOR);
        let sensor = if sensor.is_null() {
            ASensorManager_getDefaultSensor(manager, SENSOR_ROTATION_VECTOR)
        } else {
            sensor
        };
        if sensor.is_null() {
            log::warn!("Phone VR: this phone has no rotation-vector sensor; running flat");
            RUNNING.store(false, Ordering::Relaxed);
            return;
        }
        let looper = ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
        let queue = ASensorManager_createEventQueue(
            manager,
            looper,
            QUEUE_IDENT,
            None,
            std::ptr::null_mut(),
        );
        if queue.is_null() {
            log::warn!("Phone VR: the sensor event queue did not open; running flat");
            RUNNING.store(false, Ordering::Relaxed);
            return;
        }
        if ASensorEventQueue_enableSensor(queue, sensor) < 0
            || ASensorEventQueue_setEventRate(queue, sensor, RATE_US) < 0
        {
            log::warn!("Phone VR: the head sensor would not start; running flat");
            RUNNING.store(false, Ordering::Relaxed);
            return;
        }
        log::info!("Phone VR: the head sensor is on");
        let mut events = [ASensorEvent::ZERO; 4];
        let mut errors = 0;
        let mut last_log = None;
        loop {
            let ident = ALooper_pollAll(
                -1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if ident != QUEUE_IDENT {
                // ALOOPER_POLL_ERROR, or a timeout that never comes with a -1 timeout.
                errors += 1;
                if errors >= MAX_ERRORS {
                    log::warn!("Phone VR: the head sensor stopped answering; head tracking is off");
                    break;
                }
                continue;
            }
            errors = 0;
            let count = ASensorEventQueue_getEvents(queue, events.as_mut_ptr(), events.len());
            if count > 0 {
                for event in &events[..count as usize] {
                    if event.kind == SENSOR_GAME_ROTATION_VECTOR
                        || event.kind == SENSOR_ROTATION_VECTOR
                    {
                        on_event(event, &mut last_log);
                    }
                }
            }
        }
        RUNNING.store(false, Ordering::Relaxed);
    }
}

/// Remember the reading and, once a second, put the head in the log so the first
/// days on hardware can be checked with plain `adb logcat`.
fn on_event(event: &ASensorEvent, last_log: &mut Option<Instant>) {
    let (x, y, z, w) = (event.data[0], event.data[1], event.data[2], event.data[3]);
    if !(x.is_finite() && y.is_finite() && z.is_finite() && w.is_finite())
        || (x == 0.0 && y == 0.0 && z == 0.0 && w == 0.0)
    {
        return;
    }
    let mut q = Quat::from_xyzw(x, y, z, w);
    if q.w < 0.0 {
        // The same rotation; keep the readings in one hemisphere so drifting near
        // the neutral pose does not flip signs.
        q = -q;
    }
    if q.length_squared() < 1e-12 {
        return;
    }
    let q = q.normalize();
    let accuracy = if event.kind == SENSOR_ROTATION_VECTOR {
        event.data[4]
    } else {
        0.0
    };
    let log_it = last_log.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
    let mut state = lock_state();
    if state.latest.is_none() {
        // Where the phone sits at the first reading is straight ahead.
        state.origin = q;
    }
    state.latest = Some((q, accuracy));
    drop(state);
    if log_it {
        *last_log = Some(Instant::now());
        let (yaw, pitch, roll) = q.to_euler(glam::EulerRot::YXZ);
        log::info!(
            "Phone VR: head yaw {:.1} pitch {:.1} roll {:.1} accuracy {accuracy:.2}",
            yaw.to_degrees(),
            pitch.to_degrees(),
            roll.to_degrees()
        );
    }
}

/// A `libandroid.so` handle; its guts are private to the platform.
#[repr(C)]
struct ASensorManager {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[repr(C)]
struct ASensorEventQueue {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[repr(C)]
struct ASensor {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[repr(C)]
struct ALooper {
    _opaque: [u8; 0],
}

/// The sensor reading, the NDK's `ASensorEvent` (sensor.h), laid out one to one;
/// the size check below keeps any platform that grows it from corrupting memory.
#[repr(C)]
#[derive(Clone, Copy)]
struct ASensorEvent {
    version: c_int,
    sensor: c_int,
    kind: c_int,
    reserved0: c_int,
    timestamp: i64,
    data: [f32; 16],
    reserved1: [u32; 16],
}

impl ASensorEvent {
    const ZERO: Self = Self {
        version: 0,
        sensor: 0,
        kind: 0,
        reserved0: 0,
        timestamp: 0,
        data: [0.0; 16],
        reserved1: [0; 16],
    };
}
const _: () = assert!(std::mem::size_of::<ASensorEvent>() == 152);

const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1 << 0;

#[link(name = "android")]
unsafe extern "C" {
    fn ALooper_prepare(options: c_int) -> *mut ALooper;
    fn ALooper_pollAll(
        timeout_ms: c_int,
        out_fd: *mut c_int,
        out_events: *mut c_int,
        out_data: *mut *mut c_void,
    ) -> c_int;
    fn ASensorManager_getInstanceForPackage(package_name: *const c_char) -> *mut ASensorManager;
    fn ASensorManager_getDefaultSensor(
        manager: *mut ASensorManager,
        sensor_type: c_int,
    ) -> *const ASensor;
    fn ASensorManager_createEventQueue(
        manager: *mut ASensorManager,
        looper: *mut ALooper,
        ident: c_int,
        callback: Option<unsafe extern "C" fn(c_int, c_int, *mut c_void) -> c_int>,
        data: *mut c_void,
    ) -> *mut ASensorEventQueue;
    fn ASensorEventQueue_enableSensor(
        queue: *mut ASensorEventQueue,
        sensor: *const ASensor,
    ) -> c_int;
    fn ASensorEventQueue_setEventRate(
        queue: *mut ASensorEventQueue,
        sensor: *const ASensor,
        rate: c_int,
    ) -> c_int;
    fn ASensorEventQueue_getEvents(
        queue: *mut ASensorEventQueue,
        events: *mut ASensorEvent,
        count: usize,
    ) -> isize;
}
