package ru.vrminefon.bridge

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.hardware.SensorManager
import android.os.IBinder
import android.util.Log

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
        fusion.stop(sensorManager)
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
        sender = Thread {
            while (running) {
                if (!protocol.connected && !protocol.connect(port)) {
                    log("runtime not reachable on $port, retrying")
                    Thread.sleep(500)
                    continue
                }
                val q = fusion.quaternion()
                val head = RemoteProtocol.HeadState(q[0], q[1], q[2], q[3])
                val none = IDLE_CONTROLLER
                if (!protocol.send(head, none, none)) {
                    protocol.close()
                }
                Thread.sleep(SEND_INTERVAL_MS)
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
        private val IDLE_CONTROLLER = RemoteProtocol.ControllerState(
            qx = 0f, qy = 0f, qz = 0f, qw = 1f,
            px = 0.2f, py = -0.5f, pz = -0.2f,
            trigger = 0f, squeeze = 0f,
            stickX = 0f, stickY = 0f,
            system = false, a = false, b = false,
            triggerClick = false, stickClick = false,
            active = true,
        )
    }
}
