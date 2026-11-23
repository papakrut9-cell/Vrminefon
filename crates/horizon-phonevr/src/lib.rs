//! Phone VR: the phone's own sensors drive the in-game head, and the renderer (a
//! later phase, docs/phonevr/PLAN.md) draws the two eyes side by side on the screen
//! so Cardboard-style glasses show it in 3D. Where the sensor crate for standalone
//! headsets goes through OpenXR, here the head comes straight from the sensor HAL.
//!
//! The sensor thread subscribes to every orientation source the phone has -- the
//! fused rotation vectors, and behind them the raw accelerometer and gyroscope --
//! and picks the freshest working one, in a cascade: the game rotation vector,
//! then the rotation vector, then the crate's own fusion of the raw pair. Firmware
//! fusion stacks have been seen to die silently mid-session (a MediaTek game
//! rotation vector that keeps its subscription but stops delivering), so every
//! fused reading carries a health check and the cascade hands over without moving
//! the in-game head. The last source is a complementary filter of our own: the
//! gyroscope drives the turn and the accelerometer lays the horizon level.
//!
//! The sensor thread reads through the NDK's `ASensor*` calls -- no JNI, no Java --
//! and hands out the latest pose to [`head`] as the turn since the last
//! [`recenter`]: both the reading and the recenter go through the same base turn
//! from the sensor's east-north-up frame into the view's, so the phone's
//! orientation in the glasses (the landscape quarter turn included) cancels
//! out and what is left is where the head moved.
//!
//! The bulk of the crate compiles everywhere so its math can be tested on the
//! host; the sensor thread itself is Android only.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use glam::{Quat, Vec3};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// The NDK's sensor ids from `android/sensor.h`. With one lookalike to keep clear
/// of: 16 is the *uncalibrated* gyroscope, whose rad/s readings wrecked the head
/// on real hardware when 15 was meant.
const SENSOR_ACCELEROMETER: i32 = 1;
const SENSOR_GYROSCOPE: i32 = 4;
const SENSOR_GAME_ROTATION_VECTOR: i32 = 15;
const SENSOR_ROTATION_VECTOR: i32 = 11;

/// How long a source stays believed without a new reading (half a second against
/// the sensors' 100 Hz), before the cascade calls it dead.
const FRESH_NS: i64 = 500_000_000;
/// How long a fused orientation the gyroscope swears is turning may stand still
/// before it counts as frozen (the MediaTek fusion that stops mid-session).
const FROZEN_NS: i64 = 750_000_000;
/// How much total gyroscope rotation (radians, accumulated while a fused stack
/// stands still) convicts it of being frozen: a slow turn that changes nothing in
/// the fused reading.
const MOVED_RAD: f32 = 0.1;
/// How little a fused quaternion may move from its reference to still count as
/// the same one (a cosine; about eight tenths of a degree of turn).
const UNCHANGED_DOT: f32 = 0.9999;
/// How long the rotation vector may report UNRELIABLE (accuracy 0) before the
/// cascade calls it out.
/// Gyroscope speed, rad/s, below which the phone is standing still and the
/// fusion may learn its bias.
const GYRO_STILL: f32 = 0.02;
/// How much of the tilt error one accelerometer reading levels away.
const LEVEL_PER_READING: f32 = 0.02;
/// Gravity, m/s2, and the band around it a usable accelerometer reading falls in.
const RELIABLE_NS: i64 = 1_000_000_000;
const GRAVITY_LOW: f32 = 5.0;
const GRAVITY_HIGH: f32 = 15.0;
/// How long a fused pose may be old and still seed the own fusion at handover.
const SEED_NS: i64 = 2_000_000_000;

/// The base turn from the sensor's east-north-up frame into the view's (-Z forward,
/// +Y up, +X right): east onto +X, up onto +Y, north onto -Z (a quarter turn about
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
    /// (-Z forward, +Y up): identity is straight ahead.
    pub rotation: Quat,
    /// The sensor's accuracy field, radians (0 when the source reports none).
    pub accuracy: f32,
}

#[derive(Clone, Copy, Default)]
struct State {
    /// The latest head pose in the view's frame, and its accuracy.
    latest: Option<(Quat, f32)>,
    /// Where the head was when it was first seen or last recentered, in the
    /// view's frame.
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
    Some(HeadPose {
        rotation: latest * state.origin.conjugate(),
        accuracy,
    })
}

/// The latest head turn since the recenter in the renderer's frame (-Z forward, +Y
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

/// A reading from one source, ready to become the head.
#[derive(Clone, Copy)]
struct Publish {
    rotation: Quat,
    accuracy: f32,
    /// The cascade moved to a different source for this reading.
    switched: bool,
}

/// Remember a sensor reading, keeping the game's pose continuous when the cascade
/// switches sources: the head the game sees is `latest` relative to `origin`, so a
/// switch re-bases `origin` onto the old pose, and a first reading starts a fresh
/// straight-ahead.
fn publish(p: Publish) {
    let mut state = lock_state();
    if p.switched {
        if let Some((was, _)) = state.latest {
            let pose = was * state.origin.conjugate();
            state.origin = pose.inverse() * p.rotation;
        }
    }
    if state.latest.is_none() {
        state.origin = p.rotation;
    }
    state.latest = Some((p.rotation, p.accuracy));
}

/// One rung of the cascade.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    /// The game rotation vector: heading relative to its own start (no compass
    /// to confuse), and the rung the recenter makes redundant everywhere else.
    Game,
    /// The plain rotation vector, when the phone's game one is broken.
    Rot,
    /// The crate's own gyro-plus-accelerometer fusion, behind the firmware's.
    Own,
}

/// The last pose from one fused sensor, with the timestamps it needs to be judged.
#[derive(Clone, Copy)]
struct FusedTrack {
    /// The sensor's quaternion, device into east-north-up world.
    q: Quat,
    /// Its reading time, sensor-clock nanoseconds.
    ts: i64,
    /// When its quaternion last actually changed (frozen values do not move it).
    changed_at: i64,
    /// Its quaternion at `changed_at`, the reference the new readings are
    /// measured against (a per-reading compare sees nothing in a slow turn).
    ref_q: Quat,
    /// Gyroscope rotation, radians, piled up since `changed_at`.
    moved: f32,
    /// When it last reported a nonzero accuracy; the game rotation vector has no
    /// accuracy field, so its readings renew this every time.
    reliable_at: i64,
}

/// The crate's own fusion: a complementary filter with the gyroscope driving the
/// turn and the accelerometer laying the horizon level. Its `q`, like the fused
/// sensors', turns device coordinates into the east-north-up world.
#[derive(Default)]
struct OwnFusion {
    q: Quat,
    /// The gyroscope's resting drift, learned while the phone stands still.
    bias: Vec3,
    last_ts: Option<i64>,
    started: bool,
}

impl OwnFusion {
    /// Start the fusion from the best guess there is: a fresh-enough fused pose
    /// keeps the head exactly where it was across the handover, and otherwise the
    /// accelerometer lays the horizon (straight ahead keeps the yaw, the way a
    /// first sensor reading always has).
    fn seed(&mut self, from_fused: Option<Quat>, up_dev: Option<Vec3>) {
        self.q = from_fused.unwrap_or_else(|| match up_dev {
            Some(up) if up.length_squared() > 0.25 => {
                let up = up.normalize();
                let axis = up.cross(Vec3::Z);
                if axis.length_squared() < 1e-8 {
                    if up.z < 0.0 {
                        Quat::from_rotation_x(std::f32::consts::PI)
                    } else {
                        Quat::IDENTITY
                    }
                } else {
                    let angle = up.dot(Vec3::Z).clamp(-1.0, 1.0).acos();
                    Quat::from_axis_angle(axis.normalize(), angle)
                }
            }
            _ => Quat::IDENTITY,
        });
        self.last_ts = None;
        self.bias = Vec3::ZERO;
        self.started = true;
    }

    /// Follow one gyroscope reading (rad/s in device coordinates): a right turn of
    /// the head reads as a negative turn about the device's X, its up edge.
    fn turn(&mut self, w: Vec3, ts: i64) {
        let dt = match self.last_ts {
            Some(prev) => (ts - prev) as f32 * 1e-9,
            None => {
                self.last_ts = Some(ts);
                return;
            }
        };
        self.last_ts = Some(ts);
        if !(0.0 < dt && dt <= 0.2) {
            return;
        }
        if w.length() < GYRO_STILL {
            self.bias = self.bias.lerp(w, dt / 2.0);
        }
        let w = w - self.bias;
        self.q = (self.q * Quat::from_scaled_axis(w * dt)).normalize();
    }

    /// Nudge the pose a little toward laying the horizon level with one
    /// accelerometer reading (the device's measured up, in device coordinates).
    /// It never turns the head: only the yaw axis is left alone, the tilt is
    /// erased. The gyro drives the turn; this is the drift brake.
    fn level(&mut self, accel: Vec3) {
        let g = accel.length();
        if !(GRAVITY_LOW < g && g < GRAVITY_HIGH) {
            return;
        }
        let up = accel / g;
        let predicted = self.q.conjugate() * Vec3::Z;
        let axis = up.cross(predicted);
        self.q = (self.q * Quat::from_scaled_axis(axis * LEVEL_PER_READING)).normalize();
    }
}

/// The book a sensor reading lands in, and the cascade on top of it.
#[derive(Default)]
struct Reader {
    /// Tracks for the game rotation vector and the rotation vector, in cascade
    /// order.
    tracks: [Option<FusedTrack>; 2],
    /// The last gyroscope reading (rad/s) and its sensor time.
    gyro: Option<(Vec3, i64)>,
    /// The last accelerometer reading (m/s2) and its sensor time.
    accel: Option<(Vec3, i64)>,
    own: OwnFusion,
    /// The source the last accepted reading came from.
    active: Option<Source>,
    /// The newest reading time of any subscribed sensor.
    newest: i64,
}

impl Reader {
    /// Take one sensor event. Returns the reading when its source is the one now
    /// running the head -- a rung's own events are still booked while a lower rung
    /// drives, they just do not turn the camera.
    fn ingest(&mut self, kind: i32, ts: i64, data: [f32; 16]) -> Option<Publish> {
        match kind {
            SENSOR_GAME_ROTATION_VECTOR | SENSOR_ROTATION_VECTOR => {
                self.ingest_fused(kind, ts, data)
            }
            SENSOR_ACCELEROMETER | SENSOR_GYROSCOPE => self.ingest_raw(kind, ts, data),
            _ => None,
        }
    }

    fn ingest_fused(&mut self, kind: i32, ts: i64, data: [f32; 16]) -> Option<Publish> {
        let (x, y, z, w) = (data[0], data[1], data[2], data[3]);
        if !(x.is_finite() && y.is_finite() && z.is_finite() && w.is_finite())
            || (x == 0.0 && y == 0.0 && z == 0.0 && w == 0.0)
        {
            return None;
        }
        let mut q = Quat::from_xyzw(x, y, z, w).normalize();
        if q.w < 0.0 {
            // The same rotation; keep the readings in one hemisphere so drifting
            // near the neutral pose does not flip signs.
            q = -q;
        }
        let index = (kind == SENSOR_ROTATION_VECTOR) as usize;
        // Only the plain rotation vector reports an accuracy; the game rotation
        // vector has none (and nothing reliable after the quaternion).
        let accuracy = if kind == SENSOR_ROTATION_VECTOR {
            data[4]
        } else {
            1.0
        };
        let (changed_at, ref_q, moved, reliable_at) = match self.tracks[index] {
            Some(track) => {
                let (changed_at, ref_q, moved) = if track.ref_q.dot(q) >= UNCHANGED_DOT {
                    (track.changed_at, track.ref_q, track.moved)
                } else {
                    (ts, q, 0.0)
                };
                (
                    changed_at,
                    ref_q,
                    moved,
                    if accuracy > 0.0 {
                        ts
                    } else {
                        track.reliable_at
                    },
                )
            }
            None => (ts, q, 0.0, ts),
        };
        self.tracks[index] = Some(FusedTrack {
            q,
            ts,
            changed_at,
            ref_q,
            moved,
            reliable_at,
        });
        self.newest = self.newest.max(ts);
        let want = self.pick()?;
        if !matches!(
            (want, index),
            (Source::Game, 0) | (Source::Rot, 1) | (Source::Own, _)
        ) {
            return None;
        }
        let switched = self.active != Some(want);
        self.active = Some(want);
        Some(Publish {
            rotation: WORLD_TO_VIEW * q,
            accuracy: if kind == SENSOR_ROTATION_VECTOR {
                data[4]
            } else {
                0.0
            },
            switched,
        })
    }

    fn ingest_raw(&mut self, kind: i32, ts: i64, data: [f32; 16]) -> Option<Publish> {
        let v = Vec3::new(data[0], data[1], data[2]);
        if !v.is_finite() {
            return None;
        }
        if kind == SENSOR_GYROSCOPE {
            // Pile the turn onto the fused tracks' motion counters: they convict a
            // fused stack frozen only when the head really turned under it.
            if let Some((_, prev)) = self.gyro {
                let dt = (ts - prev) as f32 * 1e-9;
                if 0.0 < dt && dt <= 0.2 {
                    let step = v.length() * dt;
                    for track in self.tracks.iter_mut().flatten() {
                        track.moved += step;
                    }
                }
            }
            self.gyro = Some((v, ts));
            self.own.turn(v, ts);
        } else {
            self.accel = Some((v, ts));
            self.own.level(v);
        }
        self.newest = self.newest.max(ts);
        let want = self.pick()?;
        if !matches!(want, Source::Own) {
            return None;
        }
        if !self.own.started {
            self.start_own();
        }
        let switched = self.active != Some(Source::Own);
        self.active = Some(Source::Own);
        Some(Publish {
            rotation: WORLD_TO_VIEW * self.own.q,
            accuracy: 0.0,
            switched,
        })
    }

    /// Seed the own fusion across the handover, freshest pose first.
    fn start_own(&mut self) {
        let from_fused = [0usize, 1]
            .into_iter()
            .filter_map(|index| self.tracks[index])
            .filter(|track| self.newest.saturating_sub(track.ts) <= SEED_NS)
            .max_by_key(|track| track.ts)
            .map(|track| track.q);
        let up_dev = self.accel.map(|(accel, _)| accel);
        self.own.seed(from_fused, up_dev);
    }

    /// The best source right now: the highest cascade rung that has a healthy
    /// reading. The own fusion, unlike the fused sensors, cannot freeze -- its
    /// parts are the raw ones the phone's firmware cannot break.
    fn pick(&self) -> Option<Source> {
        if self.fused_ok(0) {
            return Some(Source::Game);
        }
        if self.fused_ok(1) {
            return Some(Source::Rot);
        }
        if self.own_ok() {
            return Some(Source::Own);
        }
        None
    }

    /// A fused source is healthy when its readings are fresh, its accuracy is not
    /// stuck at UNRELIABLE, and it is not visibly frozen -- standing on one
    /// quaternion while the gyroscope piled up a real turn under it.
    fn fused_ok(&self, index: usize) -> bool {
        let Some(track) = self.tracks[index] else {
            return false;
        };
        if self.newest.saturating_sub(track.ts) > FRESH_NS {
            return false;
        }
        if self.newest.saturating_sub(track.reliable_at) > RELIABLE_NS {
            return false;
        }
        let frozen =
            self.newest.saturating_sub(track.changed_at) > FROZEN_NS && track.moved >= MOVED_RAD;
        !frozen
    }

    /// The book, for the once-a-second log: how stale every source is and what
    /// the gyroscope has piled up under the fused stacks.
    fn report(&self) -> String {
        let age = |ts: i64| self.newest.saturating_sub(ts) as f32 / 1e9;
        let mut parts = Vec::new();
        for (name, track) in [("game", self.tracks[0]), ("rot", self.tracks[1])] {
            parts.push(match track {
                Some(t) => format!(
                    "{} age {:.1}s frozen {:.1}s turn {:.2}rad unreliable {:.1}s",
                    name,
                    age(t.ts),
                    age(t.changed_at),
                    t.moved,
                    age(t.reliable_at),
                ),
                None => format!("{name} none"),
            });
        }
        if let Some((w, wts)) = self.gyro {
            parts.push(format!("gyro age {:.1}s |w| {:.3}", age(wts), w.length()));
        } else {
            parts.push("gyro none".to_string());
        }
        if let Some((a, ats)) = self.accel {
            parts.push(format!("accel age {:.1}s |a| {:.1}", age(ats), a.length()));
        } else {
            parts.push("accel none".to_string());
        }
        parts.join(" | ")
    }

    fn own_ok(&self) -> bool {
        let Some((_, wts)) = self.gyro else {
            return false;
        };
        let Some((_, ats)) = self.accel else {
            return false;
        };
        self.newest.saturating_sub(wts) <= FRESH_NS && self.newest.saturating_sub(ats) <= FRESH_NS
    }
}

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

/// How often the sensors deliver, in microseconds: 100 Hz, above any phone
/// display.
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
        let kinds = [
            SENSOR_GAME_ROTATION_VECTOR,
            SENSOR_ROTATION_VECTOR,
            SENSOR_ACCELEROMETER,
            SENSOR_GYROSCOPE,
        ];
        let mut sensors: [*const ASensor; 4] = [std::ptr::null(); 4];
        for (index, kind) in kinds.into_iter().enumerate() {
            sensors[index] = ASensorManager_getDefaultSensor(manager, kind);
        }
        if sensors[0].is_null()
            && sensors[1].is_null()
            && (sensors[2].is_null() || sensors[3].is_null())
        {
            log::warn!(
                "Phone VR: no rotation vector and no working accelerometer plus \
                 gyroscope; running flat"
            );
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
        let mut enabled = 0;
        for sensor in sensors.into_iter().filter(|sensor| !sensor.is_null()) {
            if ASensorEventQueue_enableSensor(queue, sensor) >= 0
                && ASensorEventQueue_setEventRate(queue, sensor, RATE_US) >= 0
            {
                enabled += 1;
            } else {
                log::warn!(
                    "Phone VR: one of the head sensors would not start; the others carry on"
                );
            }
        }
        if enabled == 0 {
            log::warn!("Phone VR: the head sensors would not start; running flat");
            RUNNING.store(false, Ordering::Relaxed);
            return;
        }
        log::info!("Phone VR: the head sensors are on");
        // Since the raw sensors are subscribed too, events keep arriving even when
        // a fused stack stops mid-session -- which is what lets the reader notice
        // the freeze and hand over.
        let mut reader = Reader::default();
        let mut events = [ASensorEvent::ZERO; 8];
        let mut errors = 0;
        let mut source: Option<&'static str> = None;
        let mut last_log: Option<Instant> = None;
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
                    log::warn!(
                        "Phone VR: the head sensors stopped answering; head tracking is off"
                    );
                    break;
                }
                continue;
            }
            errors = 0;
            let count = ASensorEventQueue_getEvents(queue, events.as_mut_ptr(), events.len());
            if count <= 0 {
                continue;
            }
            for event in &events[..count as usize] {
                if let Some(published) = reader.ingest(event.kind, event.timestamp, event.data) {
                    publish(published);
                    source = Some(match reader.active {
                        Some(Source::Game) => "game_rotvec",
                        Some(Source::Rot) => "rotvec",
                        _ => "own gyro+accel",
                    });
                }
            }
            log_pose(&mut last_log, source, &reader);
        }
        RUNNING.store(false, Ordering::Relaxed);
    }
}

/// Once a second, put where the game sees the head in the log, with the source
/// that is driving it and the state of every subscribed sensor, so hardware can
/// be checked with plain `adb logcat`.
#[cfg(target_os = "android")]
fn log_pose(last_log: &mut Option<Instant>, source: Option<&str>, reader: &Reader) {
    let log_it = last_log.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
    if !log_it {
        return;
    }
    *last_log = Some(Instant::now());
    let report = reader.report();
    if let Some(pose) = head() {
        let (yaw, pitch, roll) = pose.rotation.to_euler(glam::EulerRot::YXZ);
        log::info!(
            "Phone VR: head yaw {:.1} pitch {:.1} roll {:.1} accuracy {:.2} source {} | {}",
            yaw.to_degrees(),
            pitch.to_degrees(),
            roll.to_degrees(),
            pose.accuracy,
            source.unwrap_or("none"),
            report,
        );
    } else {
        log::info!("Phone VR: no pose | {report}");
    }
}

#[cfg(target_os = "android")]
use std::time::Instant;

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

    /// A reading of the fused sensors for a device whose basis (its X edge, Y
    /// edge, screen normal) points along the columns, in the east-north-up world.
    fn fused_data(basis: Mat3) -> [f32; 16] {
        let q = Quat::from_mat3(&basis).normalize();
        let mut data = [0.0; 16];
        data[0..4].copy_from_slice(&[q.x, q.y, q.z, q.w]);
        data
    }

    /// A reading of a rate sensor (rad/s for the gyroscope, m/s2 for the
    /// accelerometer) in device coordinates.
    fn vec16(v: Vec3) -> [f32; 16] {
        let mut data = [0.0; 16];
        data[0..3].copy_from_slice(&[v.x, v.y, v.z]);
        data
    }

    /// The accelerometer reading when the phone lies landscape in the glasses,
    /// straight ahead: gravity points along its up edge (the device X), because
    /// that edge points at the sky (the world's up).
    fn glasses_flat() -> [f32; 16] {
        vec16(Vec3::X * 9.81)
    }

    /// The accelerometer reading for a device whose basis is `basis`: gravity up,
    /// measured in device coordinates.
    fn accel_of(basis: Mat3) -> [f32; 16] {
        vec16(basis.transpose() * Vec3::Z * 9.81)
    }

    /// Deliver one sensor event to the game exactly as the sensor thread does,
    /// and hand back the reading (or none, when the source is not the one running
    /// the head).
    fn step_game(reader: &mut Reader, kind: i32, ts: i64, data: &[f32; 16]) -> Option<Quat> {
        let published = reader.ingest(kind, ts, *data)?;
        publish(published);
        Some(published.rotation)
    }

    /// The reading when the phone lies landscape in the glasses, straight ahead:
    /// its X edge points up, its Y edge west, and its screen (Z) out of the
    /// glasses toward the wearer's face, south.
    fn recentered() -> Mat3 {
        Mat3::from_cols(Vec3::Z, Vec3::NEG_X, Vec3::NEG_Y)
    }

    fn is_close(got: Quat, want: Quat) {
        assert!(got.dot(want).abs() > 0.999, "got {got:?}, want {want:?}");
    }

    const NS: i64 = 1_000_000_000;

    #[test]
    fn sensor_type_ids_are_the_ndks() {
        // 15 is the game rotation vector, 11 the rotation vector, 1 the
        // accelerometer and 4 the gyroscope. 16, one past the game rotation
        // vector, is the uncalibrated gyroscope, whose rad/s readings wrecked the
        // head on real hardware when 15 was meant.
        assert_eq!(SENSOR_ACCELEROMETER, 1);
        assert_eq!(SENSOR_GYROSCOPE, 4);
        assert_eq!(SENSOR_GAME_ROTATION_VECTOR, 15);
        assert_eq!(SENSOR_ROTATION_VECTOR, 11);
        assert_ne!(SENSOR_GAME_ROTATION_VECTOR, 16);
    }

    #[test]
    fn fused_recentered_straight() {
        let _test = begin();
        let mut reader = Reader::default();
        let published = reader
            .ingest(SENSOR_GAME_ROTATION_VECTOR, NS, fused_data(recentered()))
            .unwrap();
        publish(published);
        assert!(published.switched);
        is_close(
            published.rotation,
            WORLD_TO_VIEW * Quat::from_mat3(&recentered()),
        );
        is_close(head().unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn fused_turning_right_is_a_positive_yaw() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The head (and the phone in the glasses) turns right: a quarter turn
        // clockwise from above, about the world's up.
        let turned = Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2) * recentered();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            2 * NS,
            &fused_data(turned),
        );
        // The view turns the same quarter, but backward about its +Y: forward
        // goes from -Z to +X -- to the right on the screen, the way a mouse pull
        // does.
        is_close(
            head().unwrap().rotation,
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        );
        let f = head().unwrap().rotation * Vec3::NEG_Z;
        assert!(f.abs_diff_eq(Vec3::X, 1e-4), "forward {f:?}");
    }

    #[test]
    fn tilting_up_is_a_negative_pitch() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The head tips up: a quarter of the way from facing north to facing the
        // sky, about the world's east.
        let tilted = Mat3::from_rotation_x(std::f32::consts::FRAC_PI_4) * recentered();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            2 * NS,
            &fused_data(tilted),
        );
        is_close(
            head().unwrap().rotation,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_4),
        );
        let f = head().unwrap().rotation * Vec3::NEG_Z;
        assert!(f.y > 0.0, "forward {f:?} looks down");
    }

    #[test]
    fn rolling_the_phone_does_not_turn() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // Dialing the phone in the glasses: a quarter turn about its own screen
        // normal, in the device frame (so, on the right of the matrix).
        let rolled = recentered() * Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            2 * NS,
            &fused_data(rolled),
        );
        let f = head().unwrap().rotation * Vec3::NEG_Z;
        // Where the head looks is unchanged; the pose is a last-chance roll,
        // absorbed by the game's flat HUD.
        assert!(f.abs_diff_eq(Vec3::NEG_Z, 1e-4), "forward turned to {f:?}");
    }

    #[test]
    fn fused_recenter_starts_a_new_straight_ahead() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        let turned = Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2) * recentered();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            2 * NS,
            &fused_data(turned),
        );
        recenter();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            3 * NS,
            &fused_data(turned),
        );
        is_close(head().unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_source_switch_keeps_the_pose() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The fused stack drives a tilt up, then goes quiet -- so its last pose
        // goes stale and the own fusion takes over, seeded from that last pose.
        // The tilt the fused stack built up must survive the handover.
        let tilted = Mat3::from_rotation_x(std::f32::consts::FRAC_PI_4) * recentered();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            2 * NS,
            &fused_data(tilted),
        );
        is_close(
            head().unwrap().rotation,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_4),
        );
        for step in 30..45 {
            let ts = step * NS / 10;
            step_game(&mut reader, SENSOR_GYROSCOPE, ts, &vec16(Vec3::ZERO));
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &accel_of(tilted));
        }
        let pose = head().unwrap();
        assert_eq!(pose.accuracy, 0.0);
        is_close(
            pose.rotation,
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_4),
        );
    }

    #[test]
    fn a_frozen_fused_stack_hands_over_to_the_raw_imu() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The MediaTek bug, a small stage of it: the fused stack keeps delivering
        // the same quaternion (fresh, but visibly frozen to the gyroscope's eye)
        // while the head is turning. It is benched, and the own fusion takes over
        // from its last real pose.
        let mut handover = None;
        for step in 12..28 {
            let ts = step * NS / 10;
            step_game(
                &mut reader,
                SENSOR_GAME_ROTATION_VECTOR,
                ts,
                &fused_data(recentered()),
            );
            if let Some(published) = step_game(
                &mut reader,
                SENSOR_GYROSCOPE,
                ts,
                &vec16(Vec3::new(-0.5, 0.0, 0.0)),
            ) {
                if handover.is_none() {
                    handover = Some(published);
                }
            }
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &glasses_flat());
        }
        assert!(handover.is_some(), "the fusion never took over");
        is_close(
            handover.unwrap(),
            WORLD_TO_VIEW * Quat::from_mat3(&recentered()),
        );
        // Half a second of turning at half a radian after the handover: the head
        // now moves with the raw gyroscope.
        let f = head().unwrap().rotation * Vec3::NEG_Z;
        assert!(f.x > 0.1, "forward {f:?} did not follow the turn");
    }

    #[test]
    fn a_slow_turn_under_a_frozen_stack_still_hands_over() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The phone case that started it: a slow head turn (six degrees a second,
        // far below any single-gyroscope-reading threshold) under a fused stack
        // frozen on one quaternion. The turn piles up in the motion counters, and
        // the stack is benched all the same.
        let mut handover = false;
        for step in 12..26 {
            let ts = step * NS / 10;
            step_game(
                &mut reader,
                SENSOR_GAME_ROTATION_VECTOR,
                ts,
                &fused_data(recentered()),
            );
            if step_game(
                &mut reader,
                SENSOR_GYROSCOPE,
                ts,
                &vec16(Vec3::new(-0.1, 0.0, 0.0)),
            )
            .is_some()
            {
                handover = true;
            }
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &glasses_flat());
        }
        assert!(
            handover,
            "the frozen stack kept the head even for a slow turn"
        );
    }

    #[test]
    fn an_unreliable_rotation_vector_is_benched() {
        let _test = begin();
        let mut reader = Reader::default();
        // The rotation vector (reads go through it because the game one is
        // absent) reports UNRELIABLE (accuracy 0) and never recovers: after a
        // second of that it is benched too, and the own fusion drives. The seed
        // comes from its last reading, so the head does not jump.
        step_game(
            &mut reader,
            SENSOR_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        assert!(matches!(reader.active, Some(Source::Rot)));
        for step in 12..24 {
            let ts = step * NS / 10;
            step_game(
                &mut reader,
                SENSOR_ROTATION_VECTOR,
                ts,
                &fused_data(recentered()),
            );
            step_game(&mut reader, SENSOR_GYROSCOPE, ts, &vec16(Vec3::ZERO));
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &glasses_flat());
        }
        assert!(matches!(reader.active, Some(Source::Own)));
        is_close(head().unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_silent_fused_stack_hands_over_to_the_raw_imu() {
        let _test = begin();
        let mut reader = Reader::default();
        step_game(
            &mut reader,
            SENSOR_GAME_ROTATION_VECTOR,
            NS,
            &fused_data(recentered()),
        );
        // The other stage: the fused events simply stop. The cascade waits out the
        // half-second belief the last reading earned, then hands over -- seeded
        // from that reading, so the head does not jump.
        let mut handover_at = None;
        for step in 12..26 {
            let ts = step * NS / 10;
            if step_game(&mut reader, SENSOR_GYROSCOPE, ts, &vec16(Vec3::ZERO)).is_some()
                && handover_at.is_none()
            {
                handover_at = Some(ts);
            }
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &glasses_flat());
        }
        let handover_at = handover_at.expect("the fusion never took over");
        assert!(handover_at >= 3 * NS / 2, "handover at {handover_at}");
        assert!(handover_at <= 8 * NS / 5, "handover at {handover_at}");
        is_close(head().unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn the_own_imu_turns_right_and_recovers_itself_later() {
        let _test = begin();
        let mut reader = Reader::default();
        // No fused stack ever shows up: the phone is one of the ones whose
        // rotation vectors are dead on arrival, and everything comes from the raw
        // pair. Two seconds of turning right at 0.9 rad/s (with the accelerometer
        // keeping the horizon), a bit past a quarter turn and a half.
        for step in 0..201 {
            let ts = step * NS / 100;
            step_game(
                &mut reader,
                SENSOR_GYROSCOPE,
                ts,
                &vec16(Vec3::new(-0.9, 0.0, 0.0)),
            );
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &glasses_flat());
        }
        let f = head().unwrap().rotation * Vec3::NEG_Z;
        assert!(f.x > 0.8, "forward {f:?} did not follow the turn");
        assert!(f.y.abs() < 0.2, "forward {f:?} left the horizon");
        // And the games' recenter works the same on this source as on any other.
        recenter();
        is_close(head().unwrap().rotation, Quat::IDENTITY);
    }

    #[test]
    fn the_own_imu_lays_the_horizon_from_the_accelerometer() {
        let _test = begin();
        let mut reader = Reader::default();
        // The phone tipped up and to one side: its measured up (device
        // coordinates) is halfway between the device's screen normal and its west
        // edge. The seed lays the horizon from exactly that.
        let up_dev = Vec3::new(
            0.0,
            -std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        step_game(&mut reader, SENSOR_GYROSCOPE, NS, &vec16(Vec3::ZERO));
        let first =
            step_game(&mut reader, SENSOR_ACCELEROMETER, NS, &vec16(up_dev * 9.81)).unwrap();
        is_close(
            first,
            WORLD_TO_VIEW * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4),
        );
        // Then the gyro's own drift (or a bad turn) puts the horizon out by
        // roughly half a radian, and the accelerometer walks it back -- without
        // turning the head, only laying it level again.
        reader.own.q =
            (reader.own.q * Quat::from_rotation_x(std::f32::consts::FRAC_PI_6)).normalize();
        for step in 1..301 {
            let ts = step * NS / 100;
            step_game(&mut reader, SENSOR_GYROSCOPE, ts, &vec16(Vec3::ZERO));
            step_game(&mut reader, SENSOR_ACCELEROMETER, ts, &vec16(up_dev * 9.81));
        }
        let predicted_up = reader.own.q.conjugate() * Vec3::Z;
        let laid = (predicted_up - up_dev).length();
        assert!(laid < 0.01, "the horizon is {laid} off after the leveling");
    }
}
