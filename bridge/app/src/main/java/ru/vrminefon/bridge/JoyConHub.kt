package ru.vrminefon.bridge

import android.hardware.Sensor
import android.hardware.SensorManager
import android.util.Log
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import kotlin.math.sin

/**
 * Live per-hand Joy-Con state, filled by the accessibility catcher (keys and
 * stick motion) and by each device's own IMU (orientation, same complementary
 * fusion as the head). The bridge sender reads from here each tick.
 */
object JoyConHub {
    class Hand {
        val fusion = HeadFusion()

        @Volatile var stickX = 0f
        @Volatile var stickY = 0f
        @Volatile var trigger = 0f
        @Volatile var squeeze = 0f
        @Volatile var a = false
        @Volatile var b = false
        @Volatile var triggerClick = false
        @Volatile var stickClick = false
        @Volatile var system = false
        @Volatile var alive = false

        var sensorManager: SensorManager? = null
    }

    val left = Hand()
    val right = Hand()

    @Volatile var ready = false
        private set

    private val handByDevice = HashMap<Int, Hand>()

    fun onServiceConnected() {
        ready = true
        refreshDevices()
    }

    fun onServiceDisconnected() {
        ready = false
    }

    @Synchronized
    fun refreshDevices() {
        // Re-attached or recharging Joy-Con come back with new device ids: drop
        // the stale entries so the map cannot grow without bound over a session.
        // @Synchronized is reentrant and attach() is called only from here and
        // from behind this same object lock, so there is no lock-order cycle.
        val live = HashSet<Int>(InputDevice.getDeviceIds().toList())
        handByDevice.keys.retainAll(live)
        for (id in live) {
            val device = InputDevice.getDevice(id) ?: continue
            attach(device)
        }
        // A Joy-Con that unpaired must fall back to the idle pose instead of
        // keeping the last IMU orientation for ever.
        for (hand in arrayOf(left, right)) {
            hand.alive = handByDevice.containsValue(hand)
        }
    }

    // Caller holds the object lock (refreshDevices or shutdown).
    private fun attach(device: InputDevice) {
        val name = device.name.lowercase()
        val hand = when {
            name.contains("joy-con (l") -> left
            name.contains("joy-con (r") -> right
            name.contains("joy-con") -> left
            name.contains("l") && device.sources and InputDevice.SOURCE_GAMEPAD != 0 -> left
            name.contains("r") && device.sources and InputDevice.SOURCE_GAMEPAD != 0 -> right
            else -> return
        }
        handByDevice[device.id] = hand
        hand.alive = true
        if (hand.sensorManager == null) {
            hand.sensorManager = device.sensorManager
            val gyro = hand.sensorManager?.getDefaultSensor(Sensor.TYPE_GYROSCOPE) != null
            val accel = hand.sensorManager?.getDefaultSensor(Sensor.TYPE_ACCELEROMETER) != null
            Log.i("joycon-bridge", "Joy-Con ${if (hand === left) "L" else "R"} attached: imu gyro=$gyro accel=$accel")
            hand.sensorManager?.let { hand.fusion.start(it) }
        }
    }

    @Synchronized
    fun shutdown() {
        unregisterHandSensors()
        ready = false
    }

    /**
     * Unregisters the fusion listeners from each device's SensorManager (the
     * platform keeps one receiver per manager, so leaving them registered leaks
     * the bridge's hold on the Joy-Con IMUs) and lets the next attach start a
     * fresh registration. The object lock is reentrant, so shutdown() may hold
     * it already.
     */
    @Synchronized
    fun unregisterHandSensors() {
        for (hand in arrayOf(left, right)) {
            hand.sensorManager?.let { hand.fusion.stop(it) }
            hand.sensorManager = null
            hand.alive = false
        }
        handByDevice.clear()
    }

    fun onKeyEvent(event: KeyEvent) {
        val hand = hand(event.deviceId) ?: return
        if (event.repeatCount > 0) return
        val down = event.action == KeyEvent.ACTION_DOWN
        when (event.keyCode) {
            KeyEvent.KEYCODE_BUTTON_A, KeyEvent.KEYCODE_BUTTON_X -> hand.a = down
            KeyEvent.KEYCODE_BUTTON_B, KeyEvent.KEYCODE_BUTTON_Y -> hand.b = down
            KeyEvent.KEYCODE_BUTTON_L1, KeyEvent.KEYCODE_BUTTON_R1 -> {
                hand.triggerClick = down
                hand.trigger = if (down) 1f else 0f
            }
            KeyEvent.KEYCODE_BUTTON_L2, KeyEvent.KEYCODE_BUTTON_R2 -> hand.squeeze = if (down) 1f else 0f
            KeyEvent.KEYCODE_BUTTON_THUMBL, KeyEvent.KEYCODE_BUTTON_THUMBR -> hand.stickClick = down
            KeyEvent.KEYCODE_BUTTON_SELECT, KeyEvent.KEYCODE_BUTTON_START -> hand.system = down
        }
    }

    fun compose(hand: Hand, idle: RemoteProtocol.ControllerState, mirrored: Boolean): RemoteProtocol.ControllerState {
        if (!hand.alive) {
            val t = (System.nanoTime() % 10_000_000_000L) / 1e9f
            val phase = sin(2f * Math.PI.toFloat() * t / 4f)
            return if (mirrored) idle.copy(px = idle.px + 0.05f * phase)
            else idle.copy(py = idle.py + 0.05f * phase)
        }
        val q = hand.fusion.quaternion()
        // The hub's own last line of defence for Joy-Con hand poses: the
        // fusion already repairs its state, but a NaN axis, an infinity or the
        // zero quaternion (0, 0, 0, 0) must never reach the wire, or
        // xrLocateViews fails with XR_ERROR_RUNTIME_FAILURE (black screen).
        val qx: Float; val qy: Float; val qz: Float; val qw: Float
        val n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]
        if (q[0].isFinite() && q[1].isFinite() && q[2].isFinite() && q[3].isFinite() &&
            n2.isFinite() && n2 > 1e-12f
        ) {
            val n = kotlin.math.sqrt(n2)
            qx = q[0] / n; qy = q[1] / n; qz = q[2] / n; qw = q[3] / n
        } else {
            qx = 0f; qy = 0f; qz = 0f; qw = 1f
        }
        return RemoteProtocol.ControllerState(
            qx = qx, qy = qy, qz = qz, qw = qw,
            px = idle.px, py = idle.py, pz = idle.pz,
            trigger = if (hand.trigger.isFinite()) hand.trigger else 0f,
            squeeze = if (hand.squeeze.isFinite()) hand.squeeze else 0f,
            stickX = if (hand.stickX.isFinite()) hand.stickX else 0f,
            stickY = if (hand.stickY.isFinite()) hand.stickY else 0f,
            system = hand.system,
            a = hand.a,
            b = hand.b,
            triggerClick = hand.triggerClick,
            stickClick = hand.stickClick,
            active = true,
        )
    }

    fun onMotionEvent(event: MotionEvent) {
        val hand = hand(event.deviceId) ?: return
        val joystick = event.source and InputDevice.SOURCE_JOYSTICK != 0
        val gamepad = event.source and InputDevice.SOURCE_GAMEPAD != 0
        if (!joystick && !gamepad) return
        var x = event.getAxisValue(MotionEvent.AXIS_X)
        var y = event.getAxisValue(MotionEvent.AXIS_Y)
        if (x == 0f && y == 0f) {
            x = event.getAxisValue(MotionEvent.AXIS_Z)
            y = event.getAxisValue(MotionEvent.AXIS_RZ)
        }
        if (event.action == MotionEvent.ACTION_MOVE || event.action == MotionEvent.ACTION_HOVER_MOVE) {
            hand.stickX = x.coerceIn(-1f, 1f)
            hand.stickY = y.coerceIn(-1f, 1f)
        }
    }
}
