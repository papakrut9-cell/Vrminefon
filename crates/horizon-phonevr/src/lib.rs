//! Phone VR: the phone's own sensors drive the in-game head, and the renderer (a later
//! phase, docs/phonevr/PLAN.md) draws the two eyes side by side on the screen so
//! Cardboard-style glasses show it in 3D. Where the sensor crate for standalone
//! headsets goes through OpenXR, here the head comes straight from the sensor HAL.
//!
//! The sensor thread reads the phone's (game) rotation vector through the NDK's
//! `ASensor*` calls — no JNI, no Java — and hands out the latest pose to [`head`] as
//! the turn since the last [`recenter`]: both the reading and the recenter go through
//! the same base turn from the sensor's east-north-up frame into the view's, so the
//! phone's orientation in the glasses (the landscape quarter turn included) cancels
//! out and what is left is where the head moved.
//!
//! The bulk of the crate compiles everywhere so its math can be tested on the host;
//! the sensor thread itself is Android only.

use glam::Quat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

/// The game rotation vector, preferred over the plain one: its heading is only
/// relative to where it started (no magnetometer), which is what a recenter does
/// anyway, and it does not swing when the phone's compass is confused. This is the
/// NDK's `ASENSOR_TYPE_GAME_ROTATION_VECTOR` — 15, next to the plain rotation
/// vector's 11 and (one past it) the uncalibrated gyroscope's 16, whose readings are
/// rad/s speeds, not an orientation: getting these two ids wrong was exactly the
/// head thrashing about on hardware that 16 is on.
const SENSOR_GAME_ROTATION_VECTOR: i32 = 15;
/// The rotation vector, when the phone has no game rotation vector
/// (`ASENSOR_TYPE_ROTATION_VECTOR`).
const SENSOR_ROTATION_VECTOR: i32 = 11;

/// The base turn from the sensor's east-north-up frame into the view's (−Z forward,
/// +Y up, +X right): east onto +X, up onto +Y, north onto −Z (a quarter turn about
/// the world's east). Both the reading and the recenter pass through it, so it is
/// what makes the sensor's quaternion aim a camera with.
const WORLD_TO_VIEW: Quat = Quat::from_xyzw(
    -std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
);

/// Where the head is, as the sensors last saw it.
#[derive(Clone, Copy)]
pub struct HeadPose {
    /// The turn from where the head was recentered to now, in the view's frame
    /// (−Z forward, +Y up): identity is straight ahead.
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
    let (latest, accuracy) = state.latest?;
    let now = WORLD_TO_VIEW * latest;
    let was = WORLD_TO_VIEW * state.origin;
    Some(HeadPose {
        rotation: now * was.conjugate(),
        accuracy,
    })
}

/// The latest head turn since the recenter in the renderer's frame (−Z forward, +Y
/// up), or none while the sensor is off.
pub fn view_rotation() -> Option<Quat> {
    head().map(|pose| pose.rotation)
}

/// Make the current head pose "straight ahead".
pub fn recenter() {
    if RUNNING.load(Ordering::Relaxed) {
        let mut state = lock_state();
        if let Some((q, _)) = state.latest {
            state.origin = q;
        }
    }
}

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

fn lock_state() -> MutexGuard<'static, State> {
    match state().lock() {
        Ok(state) => state,
        Err(poisoned) => PoisonError::into_inner(poisoned),
    }
}

/// Remember a sensor reading. Validates it (the first days of garbage readings, say
/// from the wrong sensor type, must not reach the camera), and, once a second, puts
/// where the game sees the head in the log so hardware can be checked with plain
/// `adb logcat`.
fn put_quaternion(x: f32, y: f32, z: f32, w: f32, accuracy: f32) {
    if !(x.is_finite() && y.is_finite() && z.is_finite() && w.is_finite())
        || (x == 0.0 && y == 0.0 && z == 0.0 && w == 0.0)
    {
        return;
    }
    let mut q = Quat::from_xyzw(x, y, z, w).normalize();
    if q.w < 0.0 {
        // The same rotation; keep the readings in one hemisphere so drifting near
        // the neutral pose does not flip signs.
        q = -q;
    }
    let mut last_log = LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let log_it = last_log.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
    {
        let mut state = lock_state();
        if state.latest.is_none() {
            // Where the phone sits at the first reading is straight ahead.
            state.origin = q;
        }
        state.latest = Some((q, accuracy));
    }
    if log_it {
        *last_log = Some(Instant::now());
        if let Some(pose) = head() {
            let (yaw, pitch, roll) = pose.rotation.to_euler(glam::EulerRot::YXZ);
            log::info!(
                "Phone VR: head yaw {:.1} pitch {:.1} roll {:.1} accuracy {accuracy:.2}",
                yaw.to_degrees(),
                pitch.to_degrees(),
                roll.to_degrees()
            );
        }
    }
}

static LOG: Mutex<Option<Instant>> = Mutex::new(None);

/// Start the sensor thread. Only once; the game calls it when
/// `HORIZON_PHONEVR=1` chooses the phone-VR picture over the flat one.
#[cfg(target_os = "android")]
pub fn start() {
    if RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Err(error) = std::thread::Builder::new()
        .name("Phone VR".into())
        .spawn(sensor_loop)
    {
        log::error!("Phone VR: the sensor thread did not start ({error})");
        RUNNING.store(false, Ordering::Relaxed);
    }
}

/// A plain start, where there are no phone sensors to read.
#[cfg(not(target_os = "android"))]
pub fn start() {}

/// How often the sensor delivers, in microseconds: 100 Hz, above any phone display.
#[cfg(target_os = "android")]
const RATE_US: i32 = 10_000;
/// The identity the event queue is created under, so [`ALooper_pollAll`] hands its
/// events back without any callbacks.
#[cfg(target_os = "android")]
const QUEUE_IDENT: i32 = 3;
/// The sensor loop quits after this many queue errors in a row: losing the sensor
/// service is the only way to fail like this, and spinning on it burns the battery.
#[cfg(target_os = "android")]
const MAX_ERRORS: u32 = 5;

#[cfg(target_os = "android")]
fn sensor_loop() {
    unsafe {
        let manager = ASensorManager_getInstanceForPackage(
            b"dev.horizonlab.horizon\0"
                .as_ptr()
                .cast::<std::ffi::c_char>(),
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
                        // Only the plain rotation vector reports an accuracy; the game
                        // rotation vector has none (and nothing reliable after the
                        // quaternion).
                        let accuracy = if event.kind == SENSOR_ROTATION_VECTOR {
                            event.data[4]
                        } else {
                            0.0
                        };
                        put_quaternion(
                            event.data[0],
                            event.data[1],
                            event.data[2],
                            event.data[3],
                            accuracy,
                        );
                    }
                }
            }
        }
        RUNNING.store(false, Ordering::Relaxed);
    }
}

/// A `libandroid.so` handle; its guts are private to the platform.
#[cfg(target_os = "android")]
#[repr(C)]
struct ASensorManager {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[cfg(target_os = "android")]
#[repr(C)]
struct ASensorEventQueue {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[cfg(target_os = "android")]
#[repr(C)]
struct ASensor {
    _opaque: [u8; 0],
}
/// A `libandroid.so` handle; its guts are private to the platform.
#[cfg(target_os = "android")]
#[repr(C)]
struct ALooper {
    _opaque: [u8; 0],
}

/// The sensor reading, the NDK's `ASensorEvent` (sensor.h), laid out one to one;
/// the size check below keeps any platform that grows it from corrupting memory.
#[cfg(target_os = "android")]
#[repr(C)]
#[derive(Clone, Copy)]
struct ASensorEvent {
    version: i32,
    sensor: i32,
    kind: i32,
    reserved0: i32,
    timestamp: i64,
    data: [f32; 16],
    reserved1: [u32; 16],
}

#[cfg(target_os = "android")]
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
#[cfg(target_os = "android")]
const _: () = assert!(std::mem::size_of::<ASensorEvent>() == 152);

#[cfg(target_os = "android")]
const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: i32 = 1 << 0;

#[cfg(target_os = "android")]
#[link(name = "android")]
unsafe extern "C" {
    fn ALooper_prepare(options: i32) -> *mut ALooper;
    fn ALooper_pollAll(
        timeout_ms: i32,
        out_fd: *mut i32,
        out_events: *mut i32,
        out_data: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn ASensorManager_getInstanceForPackage(
        package_name: *const std::ffi::c_char,
    ) -> *mut ASensorManager;
    fn ASensorManager_getDefaultSensor(
        manager: *mut ASensorManager,
        sensor_type: i32,
    ) -> *const ASensor;
    fn ASensorManager_createEventQueue(
        manager: *mut ASensorManager,
        looper: *mut ALooper,
        ident: i32,
        callback: Option<unsafe extern "C" fn(i32, i32, *mut std::ffi::c_void) -> i32>,
        data: *mut std::ffi::c_void,
    ) -> *mut ASensorEventQueue;
    fn ASensorEventQueue_enableSensor(queue: *mut ASensorEventQueue, sensor: *const ASensor)
    -> i32;
    fn ASensorEventQueue_setEventRate(
        queue: *mut ASensorEventQueue,
        sensor: *const ASensor,
        rate: i32,
    ) -> i32;
    fn ASensorEventQueue_getEvents(
        queue: *mut ASensorEventQueue,
        events: *mut ASensorEvent,
        count: usize,
    ) -> isize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat3, Vec3};

    /// Hold while a test runs (they share the one state), and drop everything the
    /// earlier tests put in.
    fn begin() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state = lock_state();
        *state = State::default();
        RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
        guard
    }

    static TEST: Mutex<()> = Mutex::new(());

    /// Feed a reading: the sensor's quaternion for a device whose basis (its X edge,
    /// Y edge, screen normal) points along the columns, in the east-north-up frame.
    fn put(basis: Mat3) {
        let q = Quat::from_mat3(&basis).normalize();
        put_quaternion(q.x, q.y, q.z, q.w, 0.0);
    }

    /// The reading when the phone lies landscape in the glasses, straight ahead: its
    /// X edge points up, its Y edge west, and its screen (Z) out of the glasses
    /// toward the wearer's face, south.
    fn recentered() -> Mat3 {
        Mat3::from_cols(Vec3::Z, Vec3::NEG_X, Vec3::NEG_Y)
    }

    fn is_close(got: Quat, want: Quat) {
        assert!(got.dot(want).abs() > 0.999, "got {got:?}, want {want:?}");
    }

    #[test]
    fn recentered_straight() {
        let _test = begin();
        put(recentered());
        is_close(view_rotation().unwrap(), Quat::IDENTITY);
    }

    #[test]
    fn turning_right_is_a_positive_yaw() {
        let _test = begin();
        put(recentered());
        // The head (and the phone in the glasses) turns right: a quarter turn
        // clockwise from above, about the world's up.
        put(Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2) * recentered());
        // The view turns the same quarter, but backward about its +Y: forward goes
        // from −Z to +X — to the right on the screen, the way a mouse pull does.
        is_close(
            view_rotation().unwrap(),
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        );
        let f = view_rotation().unwrap() * Vec3::NEG_Z;
        assert!(f.abs_diff_eq(Vec3::X, 1e-4), "forward {f:?}");
    }

    #[test]
    fn tilting_up_is_a_negative_pitch() {
        let _test = begin();
        put(recentered());
        // The head tips up: a quarter of the way from facing north to facing the sky,
        // about the world's east.
        put(Mat3::from_rotation_x(std::f32::consts::FRAC_PI_4) * recentered());
        is_close(
            view_rotation().unwrap(),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_4),
        );
        let f = view_rotation().unwrap() * Vec3::NEG_Z;
        assert!(f.y > 0.0, "forward {f:?} looks down");
    }

    #[test]
    fn rolling_the_phone_does_not_turn() {
        let _test = begin();
        put(recentered());
        // Dialing the phone in the glasses: a quarter turn about its own screen
        // normal, in the device frame (so, on the right of the matrix).
        put(recentered() * Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let q = view_rotation().unwrap();
        let f = q * Vec3::NEG_Z;
        // Where the head looks is unchanged; the pose is a last-chance roll, absorbed
        // by the game's flat HUD.
        assert!(f.abs_diff_eq(Vec3::NEG_Z, 1e-4), "forward turned to {f:?}");
    }

    #[test]
    fn recenter_starts_a_new_straight_ahead() {
        let _test = begin();
        put(recentered());
        put(Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2) * recentered());
        recenter();
        put(Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2) * recentered());
        is_close(view_rotation().unwrap(), Quat::IDENTITY);
    }

    #[test]
    fn sensor_type_ids_are_the_ndks() {
        let _test = begin();
        // 15 is the game rotation vector and 11 the rotation vector; 16 is the
        // uncalibrated gyroscope, whose readings wrecked the head on real hardware.
        assert_eq!(SENSOR_GAME_ROTATION_VECTOR, 15);
        assert_eq!(SENSOR_ROTATION_VECTOR, 11);
        assert_ne!(SENSOR_GAME_ROTATION_VECTOR, SENSOR_ROTATION_VECTOR);
    }
}
