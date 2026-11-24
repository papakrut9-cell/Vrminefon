package ru.vrminefon.bridge

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.hardware.SensorManager
import android.os.IBinder
import android.util.Log
import kotlin.math.sin

/**
 * Streams phone head pose (and later Joy-Con state) to Monado's remote
 * driver over loopback TCP. OpenXR apps pick the devices up from the
 * out-of-process runtime.
 */
class BridgeService : Service() {
    private lateinit var sensorManager: SensorManager
    private val fusion = HeadFusion()
    private val protocol = RemoteProtocol()
    private var sender: Thread? = null
    private var wakeLock: android.os.PowerManager.WakeLock? = null
    @Volatile
    private var running = false

    override fun onCreate() {
        super.onCreate()
        sensorManager = getSystemService(SENSOR_SERVICE) as SensorManager
        val channel = NotificationChannel(CHANNEL_ID, "Bridge", NotificationManager.IMPORTANCE_LOW)
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).createNotificationChannel(channel)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!running) {
            running = true
            startForeground(NOTIFICATION_ID, notification())
            startStream()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        fusion.stop(sensorManager)
        JoyConHub.shutdown()
        protocol.close()
        sender?.interrupt()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    fun recenter() = fusion.recenter()

    private fun notification(): Notification =
        Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("Joy-Con bridge")
            .setContentText("Feeding the OpenXR runtime")
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .build()

    private fun startStream() {
        val port = RemoteProtocol.PORT_DEFAULT
        if (!fusion.start(sensorManager)) {
            log("no gyro+accel pair, cannot drive the head")
            stopSelf()
            return
        }
        val power = getSystemService(POWER_SERVICE) as android.os.PowerManager
        wakeLock = power.newWakeLock(android.os.PowerManager.PARTIAL_WAKE_LOCK, "joycon-bridge:stream")
        wakeLock?.acquire()
        sender = Thread {
            try {
                while (running) {
                    if (!protocol.connected && !protocol.connect(port)) {
                        log("runtime not reachable on $port, retrying")
                        Thread.sleep(500)
                        continue
                    }
                    val q = fusion.quaternion()
                    val head = RemoteProtocol.HeadState(q[0], q[1], q[2], q[3])
                    val left = JoyConHub.compose(JoyConHub.left, IDLE_LEFT, mirrored = false)
                    val right = JoyConHub.compose(JoyConHub.right, IDLE_RIGHT, mirrored = true)
                    if (protocol.send(head, left, right)) {
                        packetsSent++
                        connectedNow = true
                    } else {
                        protocol.close()
                        connectedNow = false
                    }
                    Thread.sleep(SEND_INTERVAL_MS)
                }
            } catch (e: InterruptedException) {
                Thread.currentThread().interrupt()
            }
        }.also { it.start() }
    }

    private fun log(message: String) {
        Log.i(TAG, message)
    }

    companion object {
        private const val TAG = "joycon-bridge"
        private const val CHANNEL_ID = "bridge"
        private const val NOTIFICATION_ID = 1
        private const val SEND_INTERVAL_MS = 8L
        private fun idleController(px: Float, py: Float) = RemoteProtocol.ControllerState(
            qx = 0f, qy = 0f, qz = 0f, qw = 1f,
            px = px, py = py, pz = IDLE_Z,
            trigger = 0f, squeeze = 0f,
            stickX = 0f, stickY = 0f,
            system = false, a = false, b = false,
            triggerClick = false, stickClick = false,
            active = true,
        )

        @Volatile
        var connectedNow: Boolean = false
        @Volatile
        var packetsSent: Long = 0

        private const val IDLE_Y = -0.15f
        private const val IDLE_Z = -0.6f
        private val IDLE_LEFT = idleController(-0.25f, IDLE_Y)
        private val IDLE_RIGHT = idleController(0.25f, IDLE_Y)
    }
}
