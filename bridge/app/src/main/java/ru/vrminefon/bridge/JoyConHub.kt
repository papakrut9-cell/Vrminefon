package ru.vrminefon.bridge

import android.hardware.SensorManager
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

    fun refreshDevices() {
        for (id in InputDevice.getDeviceIds()) {
            val device = InputDevice.getDevice(id) ?: continue
            attach(device)
        }
    }

    fun hand(deviceId: Int): Hand? = handByDevice[deviceId]

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
            hand.sensorManager?.let { hand.fusion.start(it) }
        }
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
        return RemoteProtocol.ControllerState(
            qx = q[0], qy = q[1], qz = q[2], qw = q[3],
            px = idle.px, py = idle.py, pz = idle.pz,
            trigger = hand.trigger,
            squeeze = hand.squeeze,
            stickX = hand.stickX,
            stickY = hand.stickY,
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
