package ru.vrminefon.bridge

import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import kotlin.math.asin
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Head orientation from the raw gyroscope + accelerometer pair, the
 * complementary filter Google Cardboard's head tracker uses: the gyro
 * integrates freely and the gravity vector only pulls pitch and roll back,
 * so drift stays bounded; yaw keeps drifting and recentering takes it out.
 */
class HeadFusion : SensorEventListener {
    private val q = FloatArray(4)
    private var lastGyroNs = 0L
    private var lastAccelNs = 0L
    private val gravity = FloatArray(3)
    private var gravitySeeded = false

    @Volatile
    var started = false
        private set

    @Synchronized
    fun recenter() {
        val f = floatArrayOf(0f, 0f, -1f)
        rotateVector(q, f)
        val yaw = atan2(f[0], -f[2])
        val un = floatArrayOf(0f, 0f, sin(-yaw / 2), cos(-yaw / 2))
        val next = FloatArray(4)
        mul(un, q, next)
        normalize(next)
        System.arraycopy(next, 0, q, 0, 4)
    }

    /**
     * The current pose in the remote wire format: (qx, qy, qz, qw), unit
     * length. Never hands out NaN, an infinity or (0, 0, 0, 0) — the runtime
     * fits the head/controller pose from these, and a degenerate quaternion
     * turns xrLocateViews into XR_ERROR_RUNTIME_FAILURE while the reader keeps
     * the last frame (the black screen). Invalid state is repaired here, so a
     * single bad sensor sample cannot poison the stream for the rest of it.
     */
    @Synchronized
    fun quaternion(): FloatArray {
        sanitize(q)
        return q.copyOf()
    }

    fun start(sensorManager: SensorManager): Boolean {
        // A restart (stop then start, a Joy-Con dropping and returning) must not
        // leave us registered twice: Android throws IllegalArgumentException on
        // a duplicate listener and that would take the whole sender thread down.
        sensorManager.unregisterListener(this)
        val gyro = sensorManager.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
        val accel = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
        synchronized(this) {
            q[0] = 0f; q[1] = 0f; q[2] = 0f; q[3] = 1f
            lastGyroNs = 0; lastAccelNs = 0
            gravitySeeded = false
        }
        if (gyro != null) {
            sensorManager.registerListener(this, gyro, SensorManager.SENSOR_DELAY_GAME)
        }
        if (accel != null) {
            sensorManager.registerListener(this, accel, SensorManager.SENSOR_DELAY_GAME)
        }
        started = gyro != null && accel != null
        return started
    }

    fun stop(sensorManager: SensorManager) {
        sensorManager.unregisterListener(this)
        started = false
    }

    override fun onSensorChanged(event: SensorEvent) {
        synchronized(this) {
            when (event.sensor.type) {
                Sensor.TYPE_GYROSCOPE -> {
                    if (lastGyroNs != 0L && lastAccelNs > lastGyroNs) {
                        integrate(event.values, (lastAccelNs - lastGyroNs) / 1e9f)
                    }
                    lastGyroNs = event.timestamp
                }
                Sensor.TYPE_ACCELEROMETER -> {
                    // NaN and inf fail every comparison below, so they never
                    // reach the gravity blend or the leveling step.
                    val reliable = event.accuracy != SensorManager.SENSOR_STATUS_UNRELIABLE
                    val axa = event.values[0]; val aya = event.values[1]; val aza = event.values[2]
                    if (reliable && axa.isFinite() && aya.isFinite() && aza.isFinite()) {
                        lowPass(axa, aya, aza)
                    }
                    if (lastGyroNs != 0L && lastAccelNs < lastGyroNs && gravitySeeded) {
                        level((lastGyroNs - lastAccelNs) / 1e9f)
                    }
                    lastAccelNs = event.timestamp
                }
            }
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    private fun integrate(omega: FloatArray, dt: Float) {
        if (dt <= 0f || dt > 0.25f || !omega[0].isFinite() || !omega[1].isFinite() || !omega[2].isFinite()) return
        val w = omega
        val dqx = 0.5f * (w[0] * q[3] + w[1] * q[2] - w[2] * q[1])
        val dqy = 0.5f * (-w[0] * q[2] + w[1] * q[3] + w[2] * q[0])
        val dqz = 0.5f * (w[0] * q[1] - w[1] * q[0] + w[2] * q[3])
        val dqw = 0.5f * (-w[0] * q[0] - w[1] * q[1] - w[2] * q[2])
        q[0] += dqx * dt
        q[1] += dqy * dt
        q[2] += dqz * dt
        q[3] += dqw * dt
        normalize(q)
    }

    private fun level(dt: Float) {
        if (dt <= 0f || dt > 0.25f) return
        val g = sqrt(gravity[0] * gravity[0] + gravity[1] * gravity[1] + gravity[2] * gravity[2])
        // NaN fails every comparison, so a poisoned gravity vector stops here
        // instead of flowing into the correction quaternion below.
        if (!(g >= 5f && g <= 15f)) return
        val gx = gravity[0] / g
        val gy = gravity[1] / g
        val gz = gravity[2] / g

        val pred = floatArrayOf(0f, -1f, 0f)
        rotateVector(conj(q), pred)

        val ax = pred[1] * gz - pred[2] * gy
        val ay = pred[2] * gx - pred[0] * gz
        val az = pred[0] * gy - pred[1] * gx
        val s = sqrt(ax * ax + ay * ay + az * az)
        // A NaN here fails the comparison too; the guard is strict on purpose.
        if (!(s > 1e-6f)) return
        val dot = (pred[0] * gx + pred[1] * gy + pred[2] * gz).coerceIn(-1f, 1f)
        val angle = asin(s.coerceIn(-1f, 1f))
        val half = angle * ALPHA * dt * 0.5f
        val sh = sin(half)
        val ch = cos(half)
        var cx = ax / s * sh
        var cy = ay / s * sh
        var cz = az / s * sh
        if (dot < 0f) { cx = -cx; cy = -cy; cz = -cz }
        val next = FloatArray(4)
        mul(floatArrayOf(cx, cy, cz, ch), q, next)
        normalize(next)
        System.arraycopy(next, 0, q, 0, 4)
    }

    /**
     * Seeds or updates the gravity estimate. Samples the platform has already
     * flagged unreliable are dropped, and the sample replaces the estimate
     * outright instead of being blended in when it cannot be trusted yet: a
     * single NaN there would otherwise survive the blend for ~50 samples
     * (LEVEL = 0.02) and take the quaternion with it.
     */
    private fun lowPass(ax: Float, ay: Float, az: Float) {
        if (!gravitySeeded) {
            gravity[0] = ax; gravity[1] = ay; gravity[2] = az
            gravitySeeded = true
        } else {
            gravity[0] = gravity[0] * (1 - LEVEL) + ax * LEVEL
            gravity[1] = gravity[1] * (1 - LEVEL) + ay * LEVEL
            gravity[2] = gravity[2] * (1 - LEVEL) + az * LEVEL
            // A stale NaN already inside the estimate is repaired here too,
            // not blended forwards for another fifty samples.
            if (!gravity[0].isFinite() || !gravity[1].isFinite() || !gravity[2].isFinite()) {
                gravity[0] = ax; gravity[1] = ay; gravity[2] = az
            }
        }
    }

    private fun normalize(v: FloatArray) {
        val n = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3])
        if (n > 1e-8f && n.isFinite()) {
            v[0] /= n; v[1] /= n; v[2] /= n; v[3] /= n
        } else {
            // Non-finite or zero length: the only orientation we can still
            // vouch for is the identity one.
            v[0] = 0f; v[1] = 0f; v[2] = 0f; v[3] = 1f
        }
    }

    /** Identity (0, 0, 0, 1) replaces whatever cannot be a rotation. */
    private fun sanitize(v: FloatArray) {
        val n = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3])
        if (!n.isFinite() || n < 1e-6f) {
            v[0] = 0f; v[1] = 0f; v[2] = 0f; v[3] = 1f
            return
        }
        val k = 1f / n
        v[0] *= k; v[1] *= k; v[2] *= k; v[3] *= k
    }

    private fun conj(v: FloatArray): FloatArray = floatArrayOf(-v[0], -v[1], -v[2], v[3])

    private fun rotateVector(rot: FloatArray, v: FloatArray) {
        val x = rot[0]; val y = rot[1]; val z = rot[2]; val w = rot[3]
        val tx = 2f * (y * v[2] - z * v[1])
        val ty = 2f * (z * v[0] - x * v[2])
        val tz = 2f * (x * v[1] - y * v[0])
        val nx = v[0] + w * tx + (y * tz - z * ty)
        val ny = v[1] + w * ty + (z * tx - x * tz)
        val nz = v[2] + w * tz + (x * ty - y * tx)
        v[0] = nx; v[1] = ny; v[2] = nz
    }

    private fun mul(a: FloatArray, b: FloatArray, out: FloatArray) {
        out[0] = a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1]
        out[1] = a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0]
        out[2] = a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3]
        out[3] = a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2]
    }

    private companion object {
        const val ALPHA = 0.5f
        const val LEVEL = 0.02f
    }
}
