//! Hardware input on Android: game controllers, a mouse and a keyboard.
//!
//! winit has no gamepad events and reads every motion event as a touch, so the
//! patched winit (third_party/winit) hands each input event to [`filter`] first.
//! Controller buttons and axes become a [`PadState`] that `pad.rs` reads as its
//! hardware pad, so a controller plays like one on the desktop (bindings, menu
//! cursor, deadzones). Mouse events are queued for the game thread
//! (`Game::android_input_frame`). Keyboard keys stay with winit, which already
//! maps them to the physical keys the game handles; they are only noted so the
//! touch controls can hide.
use horizon_client::gamepad::{Button, PadState};
use std::sync::Mutex;
use winit::platform::android::activity::input::{Axis, InputEvent, KeyAction, Keycode, MotionAction};

/// `InputDevice.SOURCE_*` bits.
const SOURCE_KEYBOARD: u32 = 0x0000_0101;
const SOURCE_DPAD: u32 = 0x0000_0201;
const SOURCE_GAMEPAD: u32 = 0x0000_0401;
const SOURCE_JOYSTICK: u32 = 0x0100_0010;
const SOURCE_MOUSE: u32 = 0x0000_2002;
const SOURCE_MOUSE_RELATIVE: u32 = 0x0002_0004;

/// What the game thread gets from a mouse.
#[derive(Clone, Copy, Debug)]
pub(super) enum MouseInput {
    /// The pointer, in window pixels.
    Move([f64; 2]),
    /// Movement while the pointer is captured, in pixels.
    Delta([f64; 2]),
    /// 0 left, 1 right, 2 middle, 3 back, 4 forward.
    Button(u8, bool),
    /// Wheel clicks, up and right positive.
    Wheel([f32; 2]),
}

#[derive(Default)]
struct Shared {
    /// Buttons held on the controller's keys.
    keys: u32,
    /// Analog trigger buttons held as keys (L2, R2).
    key_triggers: [bool; 2],
    left: [f32; 2],
    z_rz: [f32; 2],
    rx_ry: [f32; 2],
    /// Analog triggers: LTRIGGER/RTRIGGER and BRAKE/GAS.
    triggers: [f32; 2],
    /// The hat (the D-pad on most controllers) as buttons.
    hat: u32,
    /// The right stick shows up on RX/RY (some DirectInput controllers); Z/RZ
    /// then carry the triggers.
    rx_ry_stick: bool,
    /// Trigger axes have moved (LTRIGGER, RTRIGGER, GAS, BRAKE).
    trigger_axes: bool,
    mouse: Vec<MouseInput>,
    mouse_buttons: u32,
    /// A primary click without a button state is held.
    tap: bool,
    keyboard_used: bool,
    mouse_used: bool,
    /// Escape (or Back) presses from a hardware keyboard.
    escapes: u32,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    keys: 0,
    key_triggers: [false; 2],
    left: [0.0; 2],
    z_rz: [0.0; 2],
    rx_ry: [0.0; 2],
    triggers: [0.0; 2],
    hat: 0,
    rx_ry_stick: false,
    trigger_axes: false,
    mouse: Vec::new(),
    mouse_buttons: 0,
    tap: false,
    keyboard_used: false,
    mouse_used: false,
    escapes: 0,
});

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Route input events through [`filter`]; called once before the event loop starts.
pub(super) fn install() {
    winit::platform::android::set_input_filter(filter);
}

/// The controller as the game reads it.
pub(super) fn pad_state() -> PadState {
    let s = shared();
    let right = if s.rx_ry_stick { s.rx_ry } else { s.z_rz };
    let mut triggers = s.triggers;
    if s.rx_ry_stick && !s.trigger_axes {
        triggers = [s.z_rz[0].max(0.0), s.z_rz[1].max(0.0)];
    }
    for i in 0..2 {
        if s.key_triggers[i] {
            triggers[i] = 1.0;
        }
    }
    let pad = PadState { buttons: s.keys | s.hat, left: s.left, right: [right[0], -right[1]], triggers };
    drop(s);
    let pad = match horizon_xr::pad() {
        Some(touch) => with_touch_controllers(pad, touch),
        None => pad,
    };
    if pad.buttons != 0 || pad.left != [0.0; 2] || pad.right != [0.0; 2] || pad.triggers != [0.0; 2] {
        horizon_xr::note_input(horizon_xr::InputKind::PadHeld);
    }
    pad
}

/// VR: the Touch controllers on top of any other controller. Right stick turns (the head
/// looks up and down), left stick walks, A jumps, B goes back, Y opens the inventory,
/// triggers use and attack, the sticks' clicks sprint and sneak, the menu button pauses.
/// X (dropping) and the grips (the off hand, F3, one item from the carried stack) are
/// `xr_buttons`'.
fn with_touch_controllers(mut pad: PadState, touch: horizon_xr::Pad) -> PadState {
    for (held, button) in [
        (touch.a, Button::South),
        // In an inventory or a container B quick-moves (`xr_buttons`); Y closes it.
        (
            touch.b
                && !CONTAINER_SCREEN.load(std::sync::atomic::Ordering::Relaxed)
                && !CHAT_BUTTON.load(std::sync::atomic::Ordering::Relaxed),
            Button::East,
        ),
        // Y with X held throws the rest of the stack (`xr_buttons`), it opens nothing; in an
        // inventory Y closes it there, and opens nothing again until it is let go.
        (
            touch.y
                && !touch.x
                && !CONTAINER_SCREEN.load(std::sync::atomic::Ordering::Relaxed)
                && !Y_LATCH.load(std::sync::atomic::Ordering::Relaxed),
            Button::North,
        ),
        (touch.menu, Button::Start),
        (touch.left_click, Button::LeftStick),
        (touch.right_click, Button::RightStick),
    ] {
        if held {
            pad.set(button, true);
        }
    }
    // The left stick walks where the head faces, though the player's yaw follows the
    // right hand's aim: turned by the difference (Minecraft's input: x left, z ahead).
    if !touch.y {
        Y_LATCH.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let touch = {
        let turn = horizon_xr::move_turn().to_radians();
        let (sin, cos) = turn.sin_cos();
        let (x, z) = (-touch.left[0], touch.left[1]);
        let (x, z) = (x * cos - z * sin, x * sin + z * cos);
        horizon_xr::Pad { left: [-x, z], ..touch }
    };
    // Over the wrist's hotbar, the right trigger picks a slot (`xr_look`), not the pad; with
    // a bow it draws the string (`xr_bow`).
    let touch = if BOW_STRING_LEFT.load(std::sync::atomic::Ordering::Relaxed) {
        // A bow: the left trigger draws it (`xr_bow`); neither uses nor attacks.
        horizon_xr::Pad { triggers: [0.0; 2], ..touch }
    } else if horizon_xr::wrist_slot().is_some()
        || BOW_HELD.load(std::sync::atomic::Ordering::Relaxed)
    {
        horizon_xr::Pad { triggers: [touch.triggers[0], 0.0], ..touch }
    } else {
        touch
    };
    // In a menu the triggers click where their controllers point (`xr_pointer`), not the
    // pad, and the right stick scrolls there.
    let touch = if horizon_xr::menu_open() {
        horizon_xr::Pad { triggers: [0.0; 2], right: [0.0; 2], ..touch }
    } else {
        touch
    };
    let stronger = |a: [f32; 2], b: [f32; 2]| if b[0].hypot(b[1]) > a[0].hypot(a[1]) { b } else { a };
    pad.left = stronger(pad.left, touch.left);
    pad.right = stronger(pad.right, touch.right);
    pad.triggers = [pad.triggers[0].max(touch.triggers[0]), pad.triggers[1].max(touch.triggers[1])];
    pad
}

/// Everything let go (the app went to the background, a controller went away).
pub(super) fn reset() {
    let mut s = shared();
    s.keys = 0;
    s.key_triggers = [false; 2];
    s.left = [0.0; 2];
    s.z_rz = [0.0; 2];
    s.rx_ry = [0.0; 2];
    s.triggers = [0.0; 2];
    s.hat = 0;
    s.mouse_buttons = 0;
    s.tap = false;
    s.mouse.clear();
}

/// Mouse events since the last call.
pub(super) fn take_mouse() -> Vec<MouseInput> {
    std::mem::take(&mut shared().mouse)
}

/// A hardware keyboard or a mouse was used since the last call.
pub(super) fn take_used() -> (bool, bool) {
    let mut s = shared();
    let used = (s.keyboard_used, s.mouse_used);
    s.keyboard_used = false;
    s.mouse_used = false;
    used
}

/// VR: an inventory or container screen is open: B quick-moves there instead of going back.
static CONTAINER_SCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// VR: B opens the chat in play and closes it again (`xr_buttons`), it is no pad button there.
/// VR: where the window's player stands and where their head looks (yaw, pitch), for what
/// asks where a player looks rather than aims (an enderman's stare): the game's own look
/// goes along the right hand. Read on the server thread too (`Game::player_view`).
pub(super) static XR_HEAD: Mutex<Option<([f64; 3], (f32, f32))>> = Mutex::new(None);
static CHAT_BUTTON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// VR: Y closed an inventory or container (`xr_buttons`): it opens nothing until let go.
static Y_LATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// VR: clicks queued with Shift held (`xr_buttons`); Shift lets go once they are handled.
static SHIFT_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// VR: the hand strikes by itself (a blow into a block, a swing at a mob), for the arm's swing.
static GESTURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a gesture (not the trigger) is attacking or mining now.
pub(super) fn xr_gesture_active() -> bool {
    GESTURE.load(std::sync::atomic::Ordering::Relaxed)
}

/// VR: a bow is held, so the right trigger draws its string (`xr_bow`), never attacks.
static BOW_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// VR: a bow (not a crossbow) is held: the left hand draws its string with the left
/// trigger (`xr_bow`), so neither trigger reaches the pad.
static BOW_STRING_LEFT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// VR: the boat's turn the view has still to follow (degrees) and the last frame's time
/// (`defer_boat_turn`).
static BOAT_TURN: Mutex<(f32, Option<std::time::Instant>)> = Mutex::new((0.0, None));

/// VR: `degrees` of a boat's turn this tick, followed by the view over the next tick's
/// frames (`xr_look`). False off VR.
pub(super) fn defer_boat_turn(degrees: f32) -> bool {
    if !horizon_xr::active() {
        return false;
    }
    BOAT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0 += degrees;
    true
}

/// VR: an arrow is nocked or the bow drawn (`xr_bow`): the hands do not row.
static BOW_DRAWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// VR: a point's positions over the last second, for Vivecraft's averaged speeds.
type Samples = Vec<(std::time::Instant, glam::Vec3)>;

fn sample(samples: &mut Samples, now: std::time::Instant, at: glam::Vec3) {
    samples.push((now, at));
    samples.retain(|(t, _)| now.duration_since(*t).as_secs_f32() <= 1.0);
}

/// Vivecraft's `averageSpeed`: the way the point made over the last `window` seconds, per second.
fn average_speed(samples: &Samples, now: std::time::Instant, window: f32) -> f32 {
    let recent: Vec<_> = samples.iter().filter(|(t, _)| now.duration_since(*t).as_secs_f32() <= window).collect();
    let (Some(first), Some(last)) = (recent.first(), recent.last()) else {
        return 0.0;
    };
    let span = last.0.duration_since(first.0).as_secs_f32();
    if span < 0.05 {
        return 0.0;
    }
    recent.windows(2).map(|p| p[1].1.distance(p[0].1)).sum::<f32>() / span
}

/// Vivecraft's `averagePosition`: where the point was on average over the last `window` seconds.
fn average_position(samples: &Samples, now: std::time::Instant, window: f32) -> Option<glam::Vec3> {
    let recent: Vec<_> = samples.iter().filter(|(t, _)| now.duration_since(*t).as_secs_f32() <= window).collect();
    (!recent.is_empty()).then(|| recent.iter().map(|(_, p)| *p).sum::<glam::Vec3>() / recent.len() as f32)
}

/// VR: the right hand this frame (`Game::xr_right_swing`).
#[derive(Clone, Copy)]
struct XrSwing {
    /// The controller in the world, and the way it points.
    hand: [f64; 3],
    dir: [f64; 3],
    /// How far out from the hand the weapon's end mines, and reaches mobs.
    weapon: f64,
    entity_reach: f64,
    /// Metres a second in the room, 30 cm out along the controller, over a third of a second.
    speed: f32,
    sword: bool,
    /// The way that point went over that time, in the world's axes (a unit vector), if it moved.
    way: Option<[f64; 3]>,
}

/// What a throw by hand does (`Game::xr_throw`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Thrown {
    Item,
    Rod,
    Trident,
}

/// VR: a throw by hand is charging or on its way (`xr_throw`): the left trigger's use stays
/// with the game's own aim, and the swing and mining wait.
static THROWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// VR: food or a potion is held at the mouth (the use button held by the gesture).
static EATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn has(source: u32, bits: u32) -> bool {
    source & bits == bits
}

/// The controller button an Android key code stands for. BUTTON_1..12 are
/// DirectInput controllers without a key layout, in the usual order
/// (1 X, 2 A, 3 B, 4 Y, 5 L1, 6 R1, 7 L2, 8 R2, 9 select, 10 start, 11 L3, 12 R3).
fn pad_button(code: Keycode) -> Option<Button> {
    Some(match code {
        Keycode::ButtonA | Keycode::Button2 => Button::South,
        Keycode::ButtonB | Keycode::Button3 => Button::East,
        Keycode::ButtonX | Keycode::Button1 => Button::West,
        Keycode::ButtonY | Keycode::Button4 => Button::North,
        Keycode::ButtonL1 | Keycode::Button5 => Button::LeftBumper,
        Keycode::ButtonR1 | Keycode::Button6 => Button::RightBumper,
        Keycode::ButtonL2 | Keycode::Button7 => Button::LeftTrigger,
        Keycode::ButtonR2 | Keycode::Button8 => Button::RightTrigger,
        Keycode::ButtonSelect | Keycode::Button9 => Button::Select,
        Keycode::ButtonStart | Keycode::Button10 => Button::Start,
        Keycode::ButtonThumbl | Keycode::Button11 => Button::LeftStick,
        Keycode::ButtonThumbr | Keycode::Button12 => Button::RightStick,
        Keycode::DpadUp => Button::DPadUp,
        Keycode::DpadDown => Button::DPadDown,
        Keycode::DpadLeft => Button::DPadLeft,
        Keycode::DpadRight => Button::DPadRight,
        // A controller's back key (the View/Select button on many).
        Keycode::Back => Button::Select,
        _ => return None,
    })
}

fn is_gamepad_key(code: Keycode) -> bool {
    matches!(
        code,
        Keycode::ButtonA
            | Keycode::ButtonB
            | Keycode::ButtonX
            | Keycode::ButtonY
            | Keycode::ButtonL1
            | Keycode::ButtonR1
            | Keycode::ButtonL2
            | Keycode::ButtonR2
            | Keycode::ButtonSelect
            | Keycode::ButtonStart
            | Keycode::ButtonThumbl
            | Keycode::ButtonThumbr
            | Keycode::Button1
            | Keycode::Button2
            | Keycode::Button3
            | Keycode::Button4
            | Keycode::Button5
            | Keycode::Button6
            | Keycode::Button7
            | Keycode::Button8
            | Keycode::Button9
            | Keycode::Button10
            | Keycode::Button11
            | Keycode::Button12
    )
}

/// Every input event, before winit: `true` takes it.
fn filter(event: &InputEvent<'_>) -> bool {
    match event {
        InputEvent::KeyEvent(key) => {
            let source: u32 = key.source().into();
            let code = key.key_code();
            let down = match key.action() {
                KeyAction::Down => true,
                KeyAction::Up => false,
                _ => return false,
            };
            // A mouse's right button turns into Back when nobody takes it: the
            // button already went to the game.
            if code == Keycode::Back && has(source, SOURCE_MOUSE) {
                return true;
            }
            let from_pad = has(source, SOURCE_GAMEPAD)
                || has(source, SOURCE_JOYSTICK)
                || is_gamepad_key(code)
                || (source == SOURCE_DPAD
                    && matches!(code, Keycode::DpadUp | Keycode::DpadDown | Keycode::DpadLeft | Keycode::DpadRight));
            if from_pad {
                horizon_xr::note_input(horizon_xr::InputKind::AndroidKey);
                let Some(button) = pad_button(code) else {
                    // Home/Mode and the rest stay with the system.
                    return false;
                };
                let mut s = shared();
                match button {
                    Button::LeftTrigger => s.key_triggers[0] = down,
                    Button::RightTrigger => s.key_triggers[1] = down,
                    _ => {
                        if down {
                            s.keys |= button.bit();
                        } else {
                            s.keys &= !button.bit();
                        }
                    }
                }
                return true;
            }
            // A hardware keyboard's Esc: close the
            // screen or pause, as the touch back does.
            if has(source, SOURCE_KEYBOARD)
                && !has(source, SOURCE_MOUSE)
                && code == Keycode::Escape
            {
                let mut s = shared();
                s.keyboard_used = true;
                if down && key.repeat_count() == 0 {
                    s.escapes = 1;
                }
                return true;
            }
            if has(source, SOURCE_KEYBOARD)
                && !matches!(code, Keycode::Back | Keycode::Home | Keycode::VolumeUp | Keycode::VolumeDown | Keycode::VolumeMute | Keycode::Power)
            {
                shared().keyboard_used = true;
            }
            false
        }
        InputEvent::MotionEvent(motion) => {
            let source: u32 = motion.source().into();
            if has(source, SOURCE_JOYSTICK) {
                horizon_xr::note_input(horizon_xr::InputKind::AndroidMotion);
                if motion.action() != MotionAction::Move {
                    return true;
                }
                let p = motion.pointer_at_index(0);
                let a = |axis| {
                    let v = p.axis_value(axis);
                    if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }
                };
                let mut s = shared();
                s.left = [a(Axis::X), -a(Axis::Y)];
                s.z_rz = [a(Axis::Z), a(Axis::Rz)];
                s.rx_ry = [a(Axis::Rx), a(Axis::Ry)];
                if s.rx_ry[0].abs() > 0.3 || s.rx_ry[1].abs() > 0.3 {
                    s.rx_ry_stick = true;
                }
                let (lt, rt) = (a(Axis::Ltrigger).max(a(Axis::Brake)), a(Axis::Rtrigger).max(a(Axis::Gas)));
                if lt > 0.05 || rt > 0.05 {
                    s.trigger_axes = true;
                }
                s.triggers = [lt.max(0.0), rt.max(0.0)];
                let (hx, hy) = (a(Axis::HatX), a(Axis::HatY));
                let mut hat = 0;
                if hx < -0.5 {
                    hat |= Button::DPadLeft.bit();
                } else if hx > 0.5 {
                    hat |= Button::DPadRight.bit();
                }
                if hy < -0.5 {
                    hat |= Button::DPadUp.bit();
                } else if hy > 0.5 {
                    hat |= Button::DPadDown.bit();
                }
                s.hat = hat;
                return true;
            }
            let relative = has(source, SOURCE_MOUSE_RELATIVE);
            if !relative && !has(source, SOURCE_MOUSE) {
                return false;
            }
            let p = motion.pointer_at_index(0);
            let mut s = shared();
            s.mouse_used = true;
            match motion.action() {
                MotionAction::Scroll => {
                    let w = [p.axis_value(Axis::Hscroll), p.axis_value(Axis::Vscroll)];
                    if w != [0.0, 0.0] {
                        s.mouse.push(MouseInput::Wheel(w));
                    }
                }
                MotionAction::Move | MotionAction::HoverMove | MotionAction::Down | MotionAction::Up => {
                    if relative {
                        let d = [f64::from(p.x()), f64::from(p.y())];
                        if d != [0.0, 0.0] {
                            s.mouse.push(MouseInput::Delta(d));
                        }
                    } else {
                        s.mouse.push(MouseInput::Move([f64::from(p.x()), f64::from(p.y())]));
                    }
                }
                _ => {}
            }
            // Buttons from the state each event carries: press and release events, and
            // the down/up a primary click also sends, all agree on it.
            let mut buttons = motion.button_state().0 & 0x1f;
            // A touchpad tap may come as down/up without a button state: the primary button.
            let action = motion.action();
            if buttons == 0 && action == MotionAction::Down {
                s.tap = true;
            }
            if s.tap {
                if matches!(action, MotionAction::Up | MotionAction::Cancel) {
                    s.tap = false;
                } else {
                    buttons |= 1;
                }
            }
            let changed = buttons ^ s.mouse_buttons;
            for i in 0..5u8 {
                if changed & (1 << i) != 0 {
                    s.mouse.push(MouseInput::Button(i, buttons & (1 << i) != 0));
                }
            }
            s.mouse_buttons = buttons;
            true
        }
        _ => false,
    }
}

impl super::Game<'_> {
    /// Once per loop iteration, before the pads are read: mouse events go to the
    /// game as the window's own would, the pointer is captured while playing with
    /// a mouse, and the touch controls hide while a controller, keyboard or mouse
    /// plays (a touch brings them back, `touch_event`).
    /// VR: the player looks where the head does (`horizon_xr::look`), through
    /// `Entity.turn` like the mouse, so everything that follows a turn still does.
    /// VR: the right hand's swing, measured as Vivecraft's roomscale combat does: a point 30 cm
    /// out along the controller, its average speed in the room over the last third of a
    /// second (the hand's own movement: walking or turning adds nothing). The weapon's
    /// length and reach go by what the hand holds: a sword, trident or spear 0.6 m and
    /// 2.5 m against mobs, another tool 0.35 m and 1.55 m, any other item 0.1 m and 0.4 m,
    /// the bare hand 0.3 m against mobs.
    fn xr_right_swing(&self) -> Option<XrSwing> {
        use std::time::Instant;
        static TIP: Mutex<Samples> = Mutex::new(Vec::new());
        static WAY: Mutex<Samples> = Mutex::new(Vec::new());
        let now = Instant::now();
        let eye = self.eye_position()?;
        let pose = horizon_xr::last_hands()?[horizon_xr::Hand::RightAim as usize]?;
        let (rotation, position) = horizon_xr::room_hand_poses()?[horizon_xr::Hand::RightAim as usize]?;
        let mut tip = TIP.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        sample(&mut tip, now, position + rotation * glam::Vec3::NEG_Z * 0.3);
        let speed = average_speed(&tip, now, 0.33);
        let along = pose.rotation * glam::Vec3::NEG_Z;
        let hand = [0, 1, 2].map(|i| eye[i] + f64::from(pose.offset[i]));
        let dir = [0, 1, 2].map(|i| f64::from(along[i]));
        let mut ways = WAY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        sample(&mut ways, now, pose.offset + along * 0.3);
        let recent: Vec<_> = ways.iter().filter(|(t, _)| now.duration_since(*t).as_secs_f32() <= 0.33).collect();
        let way = recent
            .first()
            .zip(recent.last())
            .map(|(a, b)| b.1 - a.1)
            .filter(|w| w.length() > 0.05)
            .map(|w| {
                let w = w.normalize();
                [f64::from(w.x), f64::from(w.y), f64::from(w.z)]
            });
        let item = self.visible_hand_items()[0].clone();
        let name = item.as_ref().map_or("", |s| &*s.item);
        let sword = name.ends_with("_sword") || name.ends_with("_spear") || name == "minecraft:trident";
        // Vivecraft's: the player's entity reach (3) less the arm's half metre.
        let reach = 2.5;
        let (weapon, entity_add) = if sword {
            (0.6, reach - 0.6)
        } else if super::vr_is_tool(name) {
            (0.35, reach * 0.62 - 0.35)
        } else if item.is_some() {
            (0.1, reach * 0.16 - 0.1)
        } else {
            (0.0, 0.3)
        };
        Some(XrSwing { hand, dir, weapon, entity_reach: weapon + entity_add, speed, sword, way })
    }
    /// VR: our own gestures for what Vivecraft has none for, measured the same way as its
    /// swing (`XrSwing`: over 2.5 m/s, the hand's own speed in the room). One per stroke: the
    /// hand has to slow down again (under 1.5 m/s) before the next.
    /// - Something thrown (a splash or lingering potion, a snowball, an egg, an ender pearl, a
    ///   bottle o' enchanting, a wind charge): a stroke away from the head throws it the way
    ///   the hand went.
    /// - A fishing rod: a stroke away from the head casts it that way; a stroke back towards
    ///   the head (or up) reels it in.
    /// - A trident: the hand raised beside the head, at its height or above, holds it back
    ///   (charging); a stroke away from the head throws it that way. Lowered under the chest
    ///   it is let go (the game throws it along the hand if it charged long enough).
    /// Returns where to look while the throw's click lands.
    fn xr_throw(&mut self, swing: Option<XrSwing>) -> Option<[f64; 3]> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};
        static TARGET: Mutex<Option<(Instant, [f64; 3])>> = Mutex::new(None);
        static ARMED: AtomicBool = AtomicBool::new(true);
        static TRIDENT: AtomicBool = AtomicBool::new(false);
        static CAST: Mutex<Option<String>> = Mutex::new(None);
        let now = Instant::now();
        let held = *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((until, at)) = held
            && now < until
        {
            THROWING.store(true, Ordering::Relaxed);
            return Some(at);
        }
        THROWING.store(TRIDENT.load(Ordering::Relaxed), Ordering::Relaxed);
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let name = self.visible_hand_items()[0].as_ref().map(|s| s.item.to_string()).filter(|_| playing);
        let kind = Self::xr_thrown_kind(name.as_deref());
        if TRIDENT.load(Ordering::Relaxed) && kind != Some(Thrown::Trident) {
            TRIDENT.store(false, Ordering::Relaxed);
            shared().mouse.push(MouseInput::Button(1, false));
        }
        // The rod's line is out until a reel, or another item.
        let mut cast = CAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if kind != Some(Thrown::Rod) {
            *cast = None;
        }
        let (kind, swing, eye) = (kind?, swing?, self.eye_position()?);
        if swing.speed < 1.5 {
            ARMED.store(true, Ordering::Relaxed);
        }
        // How much the stroke goes away from the head (1 straight away, -1 straight back).
        let out = [0, 1, 2].map(|i| swing.hand[i] - eye[i]);
        let far = out.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-3);
        let outward = swing.way.map_or(0.0, |way| (0..3).map(|i| way[i] * out[i]).sum::<f64>() / far);
        let stroke = swing.speed > 2.5 && ARMED.load(Ordering::Relaxed);
        let mut s = shared();
        let target = swing.way.map(|way| [0, 1, 2].map(|i| eye[i] + way[i] * 10.0));
        match kind {
            Thrown::Trident => {
                let hand = horizon_xr::last_hands()?[horizon_xr::Hand::RightGrip as usize]?.offset;
                if !TRIDENT.load(Ordering::Relaxed) {
                    if hand.y > -0.1 {
                        TRIDENT.store(true, Ordering::Relaxed);
                        THROWING.store(true, Ordering::Relaxed);
                        s.mouse.push(MouseInput::Button(1, true));
                    }
                    return None;
                }
                let thrown = stroke && outward > 0.3;
                if !thrown && hand.y > -0.45 {
                    return None;
                }
                TRIDENT.store(false, Ordering::Relaxed);
                s.mouse.push(MouseInput::Button(1, false));
                ARMED.store(false, Ordering::Relaxed);
                let target = target.filter(|_| thrown)?;
                *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((now + Duration::from_millis(150), target));
                Some(target)
            }
            Thrown::Rod => {
                let reel = cast.is_some() && stroke && outward < -0.2
                    || cast.is_some() && stroke && swing.way.is_some_and(|w| w[1] > 0.6);
                let throw = cast.is_none() && stroke && outward > 0.3;
                if !reel && !throw {
                    return None;
                }
                ARMED.store(false, Ordering::Relaxed);
                *cast = if throw { name.clone() } else { None };
                s.mouse.push(MouseInput::Button(1, true));
                s.mouse.push(MouseInput::Button(1, false));
                let target = target.filter(|_| throw)?;
                *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((now + Duration::from_millis(150), target));
                Some(target)
            }
            Thrown::Item => {
                if !stroke || outward < 0.3 {
                    return None;
                }
                let target = target?;
                ARMED.store(false, Ordering::Relaxed);
                *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((now + Duration::from_millis(150), target));
                THROWING.store(true, Ordering::Relaxed);
                s.mouse.push(MouseInput::Button(1, true));
                s.mouse.push(MouseInput::Button(1, false));
                Some(target)
            }
        }
    }
    /// What `xr_throw` does with the right hand's item.
    fn xr_thrown_kind(name: Option<&str>) -> Option<Thrown> {
        match name? {
            "minecraft:trident" => Some(Thrown::Trident),
            "minecraft:fishing_rod" => Some(Thrown::Rod),
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:snowball"
            | "minecraft:egg"
            | "minecraft:blue_egg"
            | "minecraft:brown_egg"
            | "minecraft:ender_pearl"
            | "minecraft:experience_bottle"
            | "minecraft:wind_charge" => Some(Thrown::Item),
            _ => None,
        }
    }
    /// VR: a swing at a mob, as Vivecraft's: faster than 2.5 m/s (3.75 in creative), the
    /// weapon's reach from the hand (not through blocks) meeting the mob's box. A mob is hit
    /// once as the weapon goes into it; it has to come out before it hits again. The player
    /// turns to it for a moment and the attack goes the usual way (cooldown, damage).
    /// Returns where to look.
    fn xr_swing(&mut self, swing: Option<XrSwing>) -> Option<[f64; 3]> {
        use std::time::{Duration, Instant};
        static TARGET: Mutex<Option<(Instant, [f64; 3])>> = Mutex::new(None);
        static LAST: Mutex<(Vec<u32>, Option<[f64; 3]>)> = Mutex::new((Vec::new(), None));
        let now = Instant::now();
        let held = *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((until, at)) = held
            && now < until
        {
            return Some(at);
        }
        let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let Some(swing) = swing.filter(|_| playing) else {
            *last = (Vec::new(), None);
            return None;
        };
        let length = self
            .clip_ray(swing.hand, swing.dir, swing.entity_reach)
            .map_or(swing.entity_reach, |hit| {
                (0..3).map(|i| (hit.point[i] - swing.hand[i]).powi(2)).sum::<f64>().sqrt()
            });
        let tip = [0, 1, 2].map(|i| swing.hand[i] + swing.dir[i] * length);
        // The box from the hand to the tip, and to the last tip for a fast swing.
        let mut lo = [0, 1, 2].map(|i| swing.hand[i].min(tip[i]));
        let mut hi = [0, 1, 2].map(|i| swing.hand[i].max(tip[i]));
        if let Some(before) = last.1 {
            for i in 0..3 {
                lo[i] = lo[i].min(before[i]);
                hi[i] = hi[i].max(before[i]);
            }
        }
        last.1 = Some(tip);
        let weapon = horizon_core::geometry::Aabb::new(
            horizon_core::geometry::Vec3::new(lo[0], lo[1], lo[2]),
            horizon_core::geometry::Vec3::new(hi[0], hi[1], hi[2]),
        );
        let threshold = if self.creative { 3.75 } else { 2.5 };
        let inside: Vec<(u32, [f64; 3])> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && m.hitbox().intersects(weapon))
            .map(|m| {
                let b = m.hitbox();
                let (a, z) = (b.min_position(), b.max_position());
                (m.id, [(a.x + z.x) / 2.0, (a.y + z.y) / 2.0, (a.z + z.z) / 2.0])
            })
            .collect();
        let fast = swing.speed > threshold;
        let target = inside.iter().find(|(id, _)| fast && !last.0.contains(id)).map(|&(_, at)| at);
        last.0 = if fast { inside.iter().map(|&(id, _)| id).collect() } else { Vec::new() };
        let target = target?;
        *TARGET.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((now + Duration::from_millis(150), target));
        let mut s = shared();
        s.mouse.push(MouseInput::Button(0, true));
        s.mouse.push(MouseInput::Button(0, false));
        Some(target)
    }
    /// VR: what the hands do without a click. Mining as Vivecraft's roomscale mining: the
    /// weapon's end (`XrSwing::weapon` out from the hand), moving faster than 2.5 m/s, going
    /// from the air into a block digs it three ticks' worth, up to four more for a harder
    /// blow; then it has to come out before the next blow. A sword only cuts what breaks at
    /// once. Food or drink as Vivecraft's eating: held within 25 cm of the eyes (20 cm out
    /// along the controller, its position averaged over a third of a second) it is eaten,
    /// a drink only with the controller tipped over; after 2.1 s the use starts again.
    /// Returns where to look while a blow lands.
    fn xr_hand_actions(&mut self, swing: Option<XrSwing>) -> Option<[f64; 3]> {
        use std::sync::atomic::Ordering;
        use std::time::{Duration, Instant};
        static MINING: Mutex<Option<([i32; 3], Instant)>> = Mutex::new(None);
        static POINT: Mutex<(Option<[f64; 3]>, bool)> = Mutex::new((None, false));
        static FOOD: Mutex<[Samples; 2]> = Mutex::new([Vec::new(), Vec::new()]);
        static EAT_START: Mutex<Option<Instant>> = Mutex::new(None);
        let now = Instant::now();
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let mut s = shared();
        let mut mining = MINING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut point = POINT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(swing) = swing.filter(|_| playing) else {
            mining.take();
            *point = (None, false);
            if EATING.swap(false, Ordering::Relaxed) {
                s.mouse.push(MouseInput::Button(1, false));
            }
            return None;
        };
        // Eating and drinking, either hand.
        let items = self.visible_hand_items();
        let hands = horizon_xr::last_hands();
        let mut food = FOOD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut edible = false;
        for (i, hand) in [(0, horizon_xr::Hand::RightAim), (1, horizon_xr::Hand::LeftAim)] {
            let kind = items[i].as_ref().and_then(|stack| {
                let components = self
                    .pack
                    .item_renderer
                    .models
                    .components(&horizon_client::item_render::stack_view(stack));
                let drink = matches!(&*stack.item, "minecraft:potion" | "minecraft:milk_bucket" | "minecraft:honey_bottle" | "minecraft:ominous_bottle");
                (components.get("minecraft:consumable").is_some() || components.get("minecraft:food").is_some())
                    .then_some(drink)
            });
            let pose = hands.and_then(|h| h[hand as usize]);
            let (Some(drink), Some(pose)) = (kind, pose) else {
                food[i].clear();
                continue;
            };
            let at = pose.offset + pose.rotation * glam::Vec3::NEG_Z * 0.2;
            sample(&mut food[i], now, at);
            let near = average_position(&food[i], now, 0.333).is_some_and(|p| p.length() < 0.25);
            let tipped = (pose.rotation * glam::Vec3::Y).y <= 0.0;
            // The main hand's food first, as the use goes.
            if near && (!drink || tipped) && (i == 0 || !edible) {
                edible = true;
            }
        }
        drop(food);
        let mut start = EAT_START.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let eating = EATING.load(Ordering::Relaxed);
        if eating && (!edible || start.is_some_and(|t| now.duration_since(t) > Duration::from_millis(2100))) {
            EATING.store(false, Ordering::Relaxed);
            s.mouse.push(MouseInput::Button(1, false));
        } else if !eating && edible {
            EATING.store(true, Ordering::Relaxed);
            *start = Some(now);
            s.mouse.push(MouseInput::Button(1, true));
        }
        drop(start);
        // Mining: from the weapon's end at the last frame to now, the first block entered
        // from the air.
        let solid = |b: [i32; 3]| {
            let state = self.world.block(b[0], b[1], b[2]);
            !self.res.facts().is_air(state)
                && !matches!(&*self.res.facts().get(state).name, "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column")
        };
        let end = [0, 1, 2].map(|i| swing.hand[i] + swing.dir[i] * swing.weapon);
        let from = point.0.replace(end).unwrap_or(end);
        let way = (0..3).map(|i| (end[i] - from[i]).powi(2)).sum::<f64>().sqrt();
        let steps = (way / 0.02).ceil().max(1.0) as usize;
        let cell = |t: f64| [0, 1, 2].map(|i| (from[i] + (end[i] - from[i]) * t).floor() as i32);
        let entered = (!solid(cell(0.0)))
            .then(|| (1..=steps).map(|k| cell(k as f64 / steps as f64)).find(|&b| solid(b)))
            .flatten();
        let threshold = if self.creative { 3.75 } else { 2.5 };
        let can_act = !edible && swing.speed > threshold && !point.1;
        let end_solid = solid(cell(1.0));
        let block = entered.filter(|_| can_act);
        let block = block.filter(|&b| !swing.sword || self.destroy_progress(b) >= 1.0);
        // Vivecraft's `lastWeaponSolid`: set by a blow, cleared once the end is out again.
        point.1 = if block.is_some() { true } else { point.1 && end_solid };
        if let Some(block) = block {
            let ticks = 3 + (swing.speed - threshold).clamp(0.0, 4.0) as u32;
            *super::remote::VR_MINING_HOLD.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(now + Duration::from_secs(10));
            super::remote::VR_BLOWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(ticks);
            *mining = Some((block, now + Duration::from_millis(200)));
        }
        match *mining {
            Some((block, until)) if now < until => Some(block.map(|v| f64::from(v) + 0.5)),
            Some(_) => {
                *mining = None;
                None
            }
            None => None,
        }
    }
    /// VR: rowing, as Vivecraft's: in a boat, with the stick not pushed ahead and no arrow
    /// nocked, each hand's average speed over half a second, less half a metre a second
    /// (two at most), is its oar. Both oars push ahead; one more than the other turns the
    /// boat away from its side.
    fn xr_body(&mut self) {
        use horizon_xr::Hand;
        use std::time::Instant;
        static HANDS: Mutex<[Samples; 2]> = Mutex::new([Vec::new(), Vec::new()]);
        static ROWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let now = Instant::now();
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let room = horizon_xr::room_hands();
        let mut hands = HANDS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (i, hand) in [(0, Hand::LeftGrip), (1, Hand::RightGrip)] {
            match room.and_then(|r| r[hand as usize]) {
                Some(p) => sample(&mut hands[i], now, p),
                None => hands[i].clear(),
            }
        }
        // The shield (our own; Vivecraft blocks with a button): up while its hand is held in
        // front, at least 15 cm ahead of the head and no lower than the chest. A shield on
        // the left arm blocks beside a weapon, a tool or an empty right hand (the right hand's
        // item takes the use first).
        static SHIELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let items = self.visible_hand_items();
        let shield = |i: usize| items[i].as_ref().is_some_and(|s| &*s.item == "minecraft:shield");
        let free = items[0].as_ref().is_none_or(|s| super::vr_is_tool(&s.item));
        let was = SHIELD.load(std::sync::atomic::Ordering::Relaxed);
        let world = horizon_xr::last_hands();
        let ahead = horizon_xr::head_look().map(|(_, _, f)| glam::Vec3::new(f.x, 0.0, f.z).normalize_or(glam::Vec3::NEG_Z));
        let raised = |hand: Hand| {
            world.and_then(|h| h[hand as usize]).zip(ahead).is_some_and(|(p, ahead)| {
                p.offset.y > if was { -0.45 } else { -0.35 } && p.offset.dot(ahead) > if was { 0.1 } else { 0.15 }
            })
        };
        let up = playing
            && !EATING.load(std::sync::atomic::Ordering::Relaxed)
            && ((shield(0) && raised(Hand::RightGrip)) || (shield(1) && free && raised(Hand::LeftGrip)));
        if up != was {
            SHIELD.store(up, std::sync::atomic::Ordering::Relaxed);
            shared().mouse.push(MouseInput::Button(1, up));
        }
        let stick_ahead = horizon_xr::pad().is_some_and(|p| p.left[1] > 0.5);
        let boat = playing
            && self.riding_boat_fluid_box().is_some()
            && !stick_ahead
            && !BOW_DRAWING.load(std::sync::atomic::Ordering::Relaxed);
        let oar = |i: usize| (average_speed(&hands[i], now, 0.5) - 0.5).clamp(0.0, 2.0);
        let (left, right) = (oar(0), oar(1));
        let ahead = if left > 0.0 && right > 0.0 { (left + right) / 2.0 } else { 0.0 };
        let rowing = boat && left + right + ahead > 0.0;
        if rowing {
            // The boat's own input: ahead at Vivecraft's min(0.04, 0.06 × oars) of its 0.04,
            // a turn of (left − right) / 1.5 degrees a tick of its one.
            self.pad.injected.left = [((left - right) / 1.5).clamp(-1.0, 1.0), (0.06 * ahead / 0.04).min(1.0)];
            ROWING.store(true, std::sync::atomic::Ordering::Relaxed);
        } else if ROWING.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.pad.injected.left = [0.0; 2];
        }
    }
    /// VR: swimming, as Vivecraft's: in water or lava with the stick at rest, hands drawn in
    /// towards the body (the middle between them coming nearer the neck) push the player
    /// out towards where the hands and the head point, as much as they came nearer; the push
    /// fades by a tenth each tick.
    fn xr_swim(&mut self) {
        use std::time::{Duration, Instant};
        static STATE: Mutex<(Option<Instant>, f64, [f64; 3])> = Mutex::new((None, 0.0, [0.0; 3]));
        let now = Instant::now();
        let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let wet = self.player.as_ref().is_some_and(|p| p.fluids.was_touching_water);
        let still = horizon_xr::pad().is_none_or(|p| p.left[0].abs() < 0.1 && p.left[1].abs() < 0.1);
        let hands = horizon_xr::last_hands().and_then(|h| {
            Some((h[horizon_xr::Hand::LeftAim as usize]?, h[horizon_xr::Hand::RightAim as usize]?))
        });
        let (Some((left, right)), Some((_, _, forward))) = (hands, horizon_xr::head_look()) else {
            return;
        };
        if !playing || !wet || !still || self.riding.is_some() {
            *state = (None, 0.0, [0.0; 3]);
            return;
        }
        // Once a tick.
        if state.0.is_some_and(|t| now.duration_since(t) < Duration::from_millis(50)) {
            return;
        }
        let first = state.0.replace(now).is_none();
        let middle = (left.offset + right.offset) * 0.5;
        let neck = glam::Vec3::new(0.0, -0.3, 0.0);
        let move_dir = ((middle - neck).normalize_or_zero() + forward) * 0.5;
        let hands_dir = (left.rotation * glam::Vec3::NEG_Z + right.rotation * glam::Vec3::NEG_Z) * 0.5;
        let dir_factor = (hands_dir + move_dir).length() * 0.5;
        let distance = f64::from(neck.distance(middle));
        let delta = state.1 - distance;
        state.1 = distance;
        if !first && delta > 0.0 {
            let k = delta * 1.3 * f64::from(dir_factor) * 0.15;
            for i in 0..3 {
                state.2[i] += f64::from(move_dir[i]) * k;
            }
        }
        let motion = state.2;
        if let Some(p) = self.player.as_mut() {
            p.velocity.x += motion[0];
            p.velocity.y += motion[1];
            p.velocity.z += motion[2];
        }
        state.2 = motion.map(|v| v * 0.9);
    }
    /// VR: the bow with both hands: held in either hand it shows in the right one; the left
    /// hand brought to it (within 20 cm) nocks an arrow for half a second, the left trigger
    /// squeezed then draws (use held) and letting go shoots. The arrow flies from the left
    /// hand through the right (along the arrow drawn between them), so the player looks that
    /// way while drawing and a moment after. Returns where to look.
    fn xr_bow(&mut self) -> Option<[f64; 3]> {
        use std::time::{Duration, Instant};
        // Drawing, the last aim, until when the player keeps looking along it, and until when
        // an arrow stays nocked.
        static BOW: Mutex<(bool, [f64; 3], Option<Instant>, Option<Instant>)> =
            Mutex::new((false, [0.0; 3], None, None));
        static TRIGGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let now = Instant::now();
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let items = self.visible_hand_items();
        let bow_held = playing && items.iter().flatten().any(|s| &*s.item == "minecraft:bow");
        // The crossbow (our own; Vivecraft leaves it to the buttons), in the right hand: the
        // right trigger squeezed with the left hand at it (within 30 cm) cocks it (use held
        // until it is loaded or the trigger is let go); loaded, the right trigger shoots
        // along the right hand.
        let crossbow = playing && items[0].as_ref().is_some_and(|s| &*s.item == "minecraft:crossbow");
        let loaded = crossbow && items[0].as_ref().and_then(|s| s.data()).is_some_and(|d| !d.charged.is_empty());
        let held = bow_held || crossbow;
        BOW_HELD.store(held, std::sync::atomic::Ordering::Relaxed);
        BOW_STRING_LEFT.store(bow_held && !crossbow, std::sync::atomic::Ordering::Relaxed);
        // The crossbow's trigger is the right one, the bow's string hand's the left.
        let string_trigger = if crossbow { 1 } else { 0 };
        let trigger = horizon_xr::pad().is_some_and(|p| p.triggers[string_trigger] > 0.5);
        let pressed = trigger && !TRIGGER.swap(trigger, std::sync::atomic::Ordering::Relaxed);
        if !trigger {
            TRIGGER.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let hands = horizon_xr::last_hands().and_then(|h| {
            Some((h[horizon_xr::Hand::LeftGrip as usize]?.offset, h[horizon_xr::Hand::RightGrip as usize]?.offset))
        });
        let eye = self.eye_position()?;
        let mut bow = BOW.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (left, right) = match hands {
            Some(h) if held => h,
            _ => {
                if bow.0 {
                    bow.0 = false;
                    shared().mouse.push(MouseInput::Button(1, false));
                }
                BOW_DRAWING.store(false, std::sync::atomic::Ordering::Relaxed);
                return None;
            }
        };
        // The crossbow is aimed from the right hand at the left as before; the bow's arrow
        // goes from the left (string) hand through the right (bow) hand.
        let string = if crossbow { left - right } else { right - left };
        if string.length() > 0.02 {
            let d = string.normalize();
            bow.1 = [f64::from(d.x), f64::from(d.y), f64::from(d.z)];
        }
        if crossbow {
            BOW_DRAWING.store(false, std::sync::atomic::Ordering::Relaxed);
            if !bow.0 && pressed && loaded {
                let mut s = shared();
                s.mouse.push(MouseInput::Button(1, true));
                s.mouse.push(MouseInput::Button(1, false));
            } else if !bow.0 && pressed && string.length() < 0.3 {
                bow.0 = true;
                shared().mouse.push(MouseInput::Button(1, true));
            } else if bow.0 && (!trigger || loaded) {
                bow.0 = false;
                shared().mouse.push(MouseInput::Button(1, false));
            }
            return None;
        }
        if string.length() < 0.2 {
            bow.3 = Some(now + Duration::from_millis(500));
        }
        let nocked = bow.3.is_some_and(|t| now < t);
        if !bow.0 && pressed && nocked {
            bow.0 = true;
            shared().mouse.push(MouseInput::Button(1, true));
        } else if bow.0 && !trigger {
            bow.0 = false;
            bow.2 = Some(now + Duration::from_millis(150));
            shared().mouse.push(MouseInput::Button(1, false));
        }
        BOW_DRAWING.store(bow.0 || nocked, std::sync::atomic::Ordering::Relaxed);
        let aiming = bow.0 || bow.2.is_some_and(|t| now < t);
        aiming.then(|| [0, 1, 2].map(|i| eye[i] + bow.1[i] * 10.0))
    }
    pub(super) fn xr_look(&mut self) {
        // A boat's turn (`defer_boat_turn`), a tick's worth over a tick's time.
        {
            let mut turn = BOAT_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let now = std::time::Instant::now();
            let dt = turn.1.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
            turn.1 = Some(now);
            if self.riding.is_none() {
                turn.0 = 0.0;
            } else if turn.0 != 0.0 {
                let step = if turn.0.abs() < 0.01 { turn.0 } else { turn.0 * (dt / 0.05).min(1.0) };
                turn.0 -= step;
                self.yaw += step;
            }
        }
        horizon_xr::set_menu(self.title.is_some() || self.screen != super::Screen::Playing);
        *XR_HEAD.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .zip(horizon_xr::head_look().map(|(yaw, pitch, _)| (yaw, pitch)));
        self.xr_swim();
        self.xr_body();
        // A swing at a mob first, else the hand's own actions (they track every frame).
        let bow = self.xr_bow();
        let tip = self.xr_right_swing();
        // A thrown item or a rod in the hand throws (`xr_throw`) and neither hits nor mines; a
        // trident hits as a sword until it is raised to throw.
        let thrown = Self::xr_thrown_kind(self.visible_hand_items()[0].as_ref().map(|s| &*s.item));
        let (swing, hand) = if bow.is_some() || BOW_HELD.load(std::sync::atomic::Ordering::Relaxed) {
            (None, None)
        } else {
            let throw = self.xr_throw(tip);
            if throw.is_some() || matches!(thrown, Some(Thrown::Item | Thrown::Rod)) || THROWING.load(std::sync::atomic::Ordering::Relaxed) {
                (throw, self.xr_hand_actions(None))
            } else {
                (self.xr_swing(tip), self.xr_hand_actions(tip))
            }
        };
        GESTURE.store(swing.is_some() || hand.is_some(), std::sync::atomic::Ordering::Relaxed);
        if let Some(at) = bow.or(swing).or(hand)
            && let Some(eye) = self.eye_position()
        {
            // Look at the mob being hit until the swing's click lands.
            let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
            let yaw = (-d[0]).atan2(d[2]).to_degrees() as f32;
            let pitch = (-d[1]).atan2(d[0].hypot(d[2])).to_degrees() as f32;
            let yaw = self.yaw + ((yaw - self.yaw + 180.0).rem_euclid(360.0) - 180.0);
            let turn = [f64::from(yaw - self.yaw), f64::from(pitch - self.pitch)].map(|d| d / 0.15);
            self.turn_player(turn[0], turn[1]);
            // The game's own turn: not a turn of the body for the headset's view.
            horizon_xr::sync_yaw(self.yaw);
            return;
        }
        horizon_xr::note_controller_setting(self.settings.controller.enabled);
        // The hotbar on the left wrist: where it is in the HUD's image, and the slot the
        // right controller touches there.
        let hotbar = self.renderer.as_ref().filter(|_| self.title.is_none()).map(|r| {
            let scale = horizon_client::hud::HudBatch::new([r.config.width, r.config.height]).scale;
            let (w, h) = (r.config.width as f32, r.config.height as f32);
            let left = w / 2.0 - 91.0 * scale;
            horizon_xr::Hotbar {
                // Up to the held item's name above the bars (Gui's toolHighlight).
                crop: [left - 2.0 * scale, h - 72.0 * scale, 186.0 * scale, 72.0 * scale],
                slots: [left + 1.0 * scale, h - 22.0 * scale, 20.0 * scale, 22.0 * scale],
            }
        });
        horizon_xr::set_hotbar(hotbar);
        // The slot the right controller is over on the wrist, picked when its trigger goes
        // down there (as QuestCraft does).
        static TRIGGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let trigger = horizon_xr::pad().is_some_and(|p| p.triggers[1] > 0.5);
        let pressed = trigger && !TRIGGER.swap(trigger, std::sync::atomic::Ordering::Relaxed);
        if !trigger {
            TRIGGER.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        if pressed && let Some(slot) = horizon_xr::wrist_slot() {
            self.hotbar_presses.push(slot);
        }
        // The player looks along the right hand; the left trigger's uses go along the left
        // one (`xr_left_use`).
        let playing = self.screen == super::Screen::Playing && self.title.is_none();
        let look_left = false;
        // The crosshairs: the right hand's on the nearer of the block (lying on its face)
        // and the mob the player picks; the left hand's on the block along it.
        let aim = self.eye_position().and_then(|eye| {
            let hit = self.pick();
            let face = hit.map(|hit| glam::Vec3::new(hit.face[0] as f32, hit.face[1] as f32, hit.face[2] as f32));
            let block = hit.map(|hit| hit.point);
            // mob_hit's point is from the mob's feet.
            let mob = self.mob_hit().map(|(i, _, at)| {
                let feet = &self.mobs[i].position;
                [feet.x as f64 + at[0], feet.y as f64 + at[1], feet.z as f64 + at[2]]
            });
            let distance = |p: &[f64; 3]| (0..3).map(|i| (p[i] - eye[i]).powi(2)).sum::<f64>();
            let (at, normal) = match (block, mob) {
                (Some(b), Some(m)) if distance(&m) < distance(&b) => (m, None),
                (Some(b), _) => (b, face),
                (None, m) => (m?, None),
            };
            Some((
                glam::Vec3::new((at[0] - eye[0]) as f32, (at[1] - eye[1]) as f32, (at[2] - eye[2]) as f32),
                normal,
            ))
        });
        let other = self.eye_position().zip(horizon_xr::hand_angles(!look_left)).and_then(|(eye, (yaw, pitch))| {
            let dir = super::pick_view_vector(yaw, pitch);
            let hit = self.clip_ray(eye, dir, self.block_reach)?;
            let at = hit.point;
            Some((
                glam::Vec3::new((at[0] - eye[0]) as f32, (at[1] - eye[1]) as f32, (at[2] - eye[2]) as f32),
                Some(glam::Vec3::new(hit.face[0] as f32, hit.face[1] as f32, hit.face[2] as f32)),
            ))
        });
        let aim = aim.filter(|_| playing);
        let other = other.filter(|_| playing);
        horizon_xr::set_aim_points(if look_left { [aim, other] } else { [other, aim] });
        // Swimming and gliding go where the head looks, every way (up, down, sideways: the
        // left stick's click sprints into a swim, A in the air opens the elytra), and food
        // held at the mouth in water must not point the swim down.
        let head_look = self.player.as_ref().is_some_and(|p| {
            p.fall_flying
                || (p.fluids.was_touching_water
                    && (p.swimming || EATING.load(std::sync::atomic::Ordering::Relaxed)))
        });
        horizon_xr::set_look_head(head_look);
        // Snap turning (the launcher's controls): the body turns at once by the step.
        let snap = horizon_xr::take_snap_turn();
        if snap != 0.0 && playing {
            self.yaw += snap;
        }
        let Some((yaw, pitch)) = horizon_xr::look(self.yaw) else {
            return;
        };
        let turn = [f64::from(yaw - self.yaw), f64::from(pitch - self.pitch)].map(|d| d / 0.15);
        if turn != [0.0; 2] {
            self.turn_player(turn[0], turn[1]);
        }
    }
    /// VR: the right controller points at the menu's panel like a mouse, its trigger
    /// the left button.
    fn xr_pointer(&mut self) {
        static CLICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        static AT: Mutex<[f64; 2]> = Mutex::new([-1.0; 2]);
        let pointer = horizon_xr::pointer();
        let click = pointer.is_some_and(|(_, held)| held);
        let mut s = shared();
        if let (Some((uv, _)), Some(r)) = (pointer, &self.renderer) {
            let at = [f64::from(uv[0]) * f64::from(r.config.width), f64::from(uv[1]) * f64::from(r.config.height)];
            let mut last = AT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if (at[0] - last[0]).abs() + (at[1] - last[1]).abs() > 0.5 {
                *last = at;
                s.mouse.push(MouseInput::Move(at));
            }
        }
        // The right stick, up or down, turns the wheel where it points: about eight notches
        // a second pushed all the way.
        static WHEEL: Mutex<(f32, Option<std::time::Instant>)> = Mutex::new((0.0, None));
        let mut wheel = WHEEL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        let dt = wheel.1.map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.1));
        wheel.1 = Some(now);
        let stick = horizon_xr::pad().filter(|_| pointer.is_some()).map_or(0.0, |p| p.right[1]);
        if stick.abs() > 0.3 {
            wheel.0 += stick * 8.0 * dt;
        } else {
            wheel.0 = 0.0;
        }
        while wheel.0.abs() >= 1.0 {
            let notch = wheel.0.signum();
            wheel.0 -= notch;
            s.mouse.push(MouseInput::Wheel([0.0, notch]));
        }
        drop(wheel);
        if CLICK.swap(click, std::sync::atomic::Ordering::Relaxed) != click {
            // The right grip held makes the click a shift-click (quick move), as the
            // keyboard's Shift does.
            if click && horizon_xr::right_grip_held() {
                self.modifiers = winit::keyboard::ModifiersState::SHIFT;
            } else if !click {
                self.modifiers = winit::keyboard::ModifiersState::empty();
            }
            s.mouse.push(MouseInput::Button(0, click));
            // A click on a text field brings the keyboard back after the headset hid it.
            self.touch.ime_retap |= click;
        }
    }
    /// VR: a use (the left trigger) goes along the left controller, for that moment only:
    /// the player turns to it, picks again, uses, and turns back, so the right hand keeps
    /// aiming and attacking meanwhile. Food at the mouth is eaten wherever the hand points.
    pub(super) fn xr_left_use(&mut self, use_now: impl FnOnce(&mut Self)) {
        let angles = horizon_xr::hand_angles(true)
            .filter(|_| {
                !EATING.load(std::sync::atomic::Ordering::Relaxed)
                    && !THROWING.load(std::sync::atomic::Ordering::Relaxed)
                    && !BOW_HELD.load(std::sync::atomic::Ordering::Relaxed)
            });
        let Some((yaw, pitch)) = angles else {
            use_now(self);
            return;
        };
        let (old_yaw, old_pitch) = (self.yaw, self.pitch);
        let (old_hit, old_mob) = (self.tick_hit.take(), self.tick_mob.take());
        self.yaw = old_yaw + ((yaw - old_yaw + 180.0).rem_euclid(360.0) - 180.0);
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.tick_hit = Some(self.pick());
        self.tick_mob = Some(self.mob_hit());
        use_now(self);
        self.yaw = old_yaw;
        self.pitch = old_pitch;
        self.tick_hit = old_hit;
        self.tick_mob = old_mob;
    }
    /// VR: the buttons that are no controller button. X throws one item, and Y while X is
    /// held the rest of the stack: the held item in play (Q, Control+Q), in a menu the
    /// carried stack (clicks outside the window) or else the slot pointed at. In a menu the
    /// right grip puts one item of the carried stack where the pointer is (a right click).
    /// In play the left grip alone swaps the held item with the left hand's (F), both grips
    /// together show or hide the debug screen (F3).
    fn xr_buttons(&mut self) {
        use std::sync::atomic::{AtomicU8, Ordering};
        use winit::keyboard::{KeyCode, ModifiersState};
        // Last frame's buttons: 1 X, 2 Y, 4 left grip, 8 right grip; 16 both grips were
        // held during this left grip's press.
        static HELD: AtomicU8 = AtomicU8::new(0);
        let Some(pad) = horizon_xr::pad() else {
            return;
        };
        let now = u8::from(pad.x)
            | u8::from(pad.y) << 1
            | u8::from(pad.left_grip) << 2
            | u8::from(pad.right_grip) << 3
            | u8::from(pad.b) << 5;
        let before = HELD.load(Ordering::Relaxed);
        let pressed = |bit: u8| now & bit != 0 && before & bit == 0;
        let released = |bit: u8| now & bit == 0 && before & bit != 0;
        let mut chord = before & 16;
        if now & 12 == 12 {
            chord = 16;
        }
        let in_menu = self.title.is_none() && self.screen != super::Screen::Playing;
        let container = in_menu && matches!(self.screen, super::Screen::Inventory | super::Screen::Creative);
        CONTAINER_SCREEN.store(container, Ordering::Relaxed);
        let playing = self.title.is_none() && self.screen == super::Screen::Playing;
        // B: the chat, open on the menu's panel in the room (the headset's keyboard comes up
        // for it, `touch_ime`), and B again closes it. Until B is let go it is not the pad's.
        let chat = self.title.is_none() && self.screen == super::Screen::Chat;
        // It opens as B is let go: the headset's keyboard, coming up with B still held, took
        // its release for its own "back" and went away with the chat.
        static CHAT_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if pressed(32) && playing {
            CHAT_BUTTON.store(true, Ordering::Relaxed);
            CHAT_ARMED.store(true, Ordering::Relaxed);
        } else if released(32) && CHAT_ARMED.swap(false, Ordering::Relaxed) {
            if playing {
                self.open_chat_method("");
            }
        } else if pressed(32) && chat {
            CHAT_BUTTON.store(true, Ordering::Relaxed);
            self.touch_back();
        } else if now & 32 == 0 {
            CHAT_BUTTON.store(playing || chat, Ordering::Relaxed);
        }
        let throw = if pressed(1) {
            Some(false)
        } else if pad.x && pressed(2) {
            Some(true)
        } else {
            None
        };
        if let Some(all) = throw {
            if in_menu && self.carried.is_some() {
                // A click outside the window throws the carried stack: the right button one
                // item, the left all of it.
                let back = horizon_xr::pointer().zip(self.renderer.as_ref()).map(|((uv, _), r)| {
                    [f64::from(uv[0]) * f64::from(r.config.width), f64::from(uv[1]) * f64::from(r.config.height)]
                });
                let mut s = shared();
                s.mouse.push(MouseInput::Move([1.0, 1.0]));
                let button = if all { 0 } else { 1 };
                s.mouse.push(MouseInput::Button(button, true));
                s.mouse.push(MouseInput::Button(button, false));
                if let Some(back) = back {
                    s.mouse.push(MouseInput::Move(back));
                }
            } else if in_menu || playing {
                let modifiers = self.modifiers;
                self.modifiers = if all { ModifiersState::CONTROL } else { ModifiersState::empty() };
                let _ = self.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ);
                self.modifiers = modifiers;
            }
        }
        // In an inventory or a container, where the pointer is:
        // - B quick-moves (Shift + click): crafts the whole result, or sends the stack to the
        //   other side, the fuel to the furnace's fuel slot and so on;
        // - both grips move every stack of that item to the other side (Shift + double click);
        // - the left grip alone takes half the stack (right click), the right grip alone puts
        //   one item of the carried stack down (right click).
        // Y closes an inventory or container (the pad's button reaches no screen with the
        // pointer), as Escape does.
        if container && pressed(2) && !pad.x {
            Y_LATCH.store(true, Ordering::Relaxed);
            let _ = self.key_pressed(KeyCode::Escape, KeyCode::Escape);
        }
        if container {
            let mut clicks: Vec<(u8, bool)> = Vec::new();
            if pressed(32) {
                clicks = vec![(0, true), (0, false)];
            } else if now & 12 == 12 && before & 12 != 12 {
                clicks = vec![(0, true), (0, false), (0, true), (0, false)];
            }
            if !clicks.is_empty() {
                self.modifiers = ModifiersState::SHIFT;
                SHIFT_ONCE.store(true, Ordering::Relaxed);
                let mut s = shared();
                for (button, down) in clicks {
                    s.mouse.push(MouseInput::Button(button, down));
                }
            }
            let right_click = (released(4) && chord == 0 && self.carried.is_none())
                || (released(8) && chord == 0 && self.carried.is_some());
            if right_click {
                let mut s = shared();
                s.mouse.push(MouseInput::Button(1, true));
                s.mouse.push(MouseInput::Button(1, false));
            }
        }
        if playing && now & 12 == 12 && before & 12 != 12 {
            use horizon_client::debug_keys::KeyAction;
            self.physical_keys.insert(KeyCode::F3);
            self.debug_key(KeyCode::F3, KeyAction::Press);
            self.physical_keys.remove(&KeyCode::F3);
            self.debug_key(KeyCode::F3, KeyAction::Release);
        }
        if released(4) && playing && chord == 0 {
            // key.swapOffhand (F): the held item to the left hand and back.
            let _ = self.key_pressed(KeyCode::KeyF, KeyCode::KeyF);
        }
        if now & 4 == 0 && now & 8 == 0 {
            chord = 0;
        }
        HELD.store(now | chord, Ordering::Relaxed);
    }
    pub(super) fn android_input_frame(&mut self, event_loop: &dyn super::event_loop::EventLoopCtl) {
        if horizon_xr::active() {
            self.xr_pointer();
            self.xr_buttons();
        }
        use winit::{
            application::ApplicationHandler,
            dpi::PhysicalPosition,
            event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
        };
        let escapes = std::mem::take(&mut shared().escapes);
        for _ in 0..escapes {
            self.touch_back();
        }
        let (keyboard, mouse) = take_used();
        if keyboard || mouse {
            self.touch.hardware = true;
            self.pad.in_use = false;
        }
        if mouse {
            self.touch.mouse = true;
        }
        let Some(window_id) = self.window.as_ref().map(|w| w.id()) else {
            take_mouse();
            return;
        };
        let device_id = DeviceId::dummy();
        for input in take_mouse() {
            let in_menu = self.pad_in_menu();
            let event = match input {
                MouseInput::Move(p) if in_menu => WindowEvent::CursorMoved {
                    device_id,
                    position: PhysicalPosition::new(p[0], p[1]),
                },
                MouseInput::Move(_) => continue,
                MouseInput::Delta(d) => {
                    if !in_menu {
                        self.mouse.on_move(0.0, 0.0, d[0], d[1], true, true);
                        self.last_input = std::time::Instant::now();
                    }
                    continue;
                }
                MouseInput::Button(i, down) => WindowEvent::MouseInput {
                    device_id,
                    state: if down { ElementState::Pressed } else { ElementState::Released },
                    button: match i {
                        0 => MouseButton::Left,
                        1 => MouseButton::Right,
                        2 => MouseButton::Middle,
                        3 => MouseButton::Back,
                        _ => MouseButton::Forward,
                    },
                },
                MouseInput::Wheel(w) => WindowEvent::MouseWheel {
                    device_id,
                    delta: MouseScrollDelta::LineDelta(w[0], w[1]),
                    phase: TouchPhase::Moved,
                },
            };
            // VR's gestures click for the hands (a swing, a strike, food at the mouth): no
            // mouse took over from the controllers, whose sticks and buttons stay held.
            let gesture = horizon_xr::active() && !in_menu && matches!(input, MouseInput::Button(..));
            let was = self.pad.menu.synthetic;
            self.pad.menu.synthetic = gesture || was;
            self.handle_window_event(event_loop, window_id, event, super::input_trace::InputOrigin::User);
            self.pad.menu.synthetic = was;
        }
        if SHIFT_ONCE.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.modifiers = winit::keyboard::ModifiersState::empty();
        }
        // The pointer is captured for turning the view, and let go in menus.
        let capture = self.touch.mouse && self.focused && self.player.is_some() && !self.pad_in_menu();
        if self.touch.pointer_captured != Some(capture) {
            self.touch.pointer_captured = Some(capture);
            super::android_ime::pointer_capture(capture);
        }
    }
}
