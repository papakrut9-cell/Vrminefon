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
        // Unregisters the per-device Joy-Con IMU listeners (the platform keeps a
        // receiver per SensorManager, so a forgotten listener leaks it) and
        // resets the hub, so a re-attached Joy-Con or a restarted service starts
        // from a clean slate.
        JoyConHub.shutdown()
        protocol.close()
        sender?.let {
            it.interrupt()
            // Wake the sender out of Thread.sleep(8) and give it a moment to
            // leave before the process (and the socket) goes away.
            runCatching { it.join(500) }
        }
        sender = null
        connectedNow = false
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
            // Unregister whatever half of the pair did register before stopping.
            fusion.stop(sensorManager)
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
                        // Not sent yet: an interrupted sleep means stop, and the
                        // retry loop must not keep unwinding all the way out.
                        if (!sleepUnlessStopped(SEND_RETRY_MS)) break
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
                    if (!sleepUnlessStopped(SEND_INTERVAL_MS)) break
                }
            } catch (e: InterruptedException) {
                Thread.currentThread().interrupt()
            } catch (e: Exception) {
                // A sensor or socket surprise must not leave the bridge silently
                // dead with the notification still up; log it and stop cleanly.
                log("sender stopped: $e")
                connectedNow = false
                stopSelf()
            }
        }.also { it.start() }
    }

    /** Sleeps the sender between packets; false means it was asked to stop. */
    private fun sleepUnlessStopped(ms: Long): Boolean =
        try {
            Thread.sleep(ms)
            running
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        }

    private fun log(message: String) {
        Log.i(TAG, message)
    }

    companion object {
        private const val TAG = "joycon-bridge"
        private const val CHANNEL_ID = "bridge"
        private const val NOTIFICATION_ID = 1
        private const val SEND_INTERVAL_MS = 8L
        private const val SEND_RETRY_MS = 500L
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
