package ru.vrminefon.bridge

import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket

/**
 * Wire format of Monado's remote driver (src/xrt/drivers/remote/r_interface.h),
 * ABI aarch64: a stream of fixed `struct r_remote_data` packets, no length
 * prefix, little-endian, 376 bytes each:
 *
 *   uint64 header;                     // "mndrmt3\0" as bytes
 *   r_head_data head;                  // 128: views[2] (fov+pose+pad = 12 floats),
 *                                      //      center pose (quat + vec3),
 *                                      //      bool valid + 3 pad
 *   r_remote_controller_data left;     // 120
 *   r_remote_controller_data right;    // 120
 */
class RemoteProtocol {
    private var socket: Socket? = null
    private var out: OutputStream? = null

    val connected: Boolean
        get() = socket?.isConnected == true && !socket!!.isClosed

    @Synchronized
    fun connect(port: Int): Boolean {
        runCatching {
            socket?.close()
            socket = Socket()
            socket!!.connect(InetSocketAddress("127.0.0.1", port), 1000)
            socket!!.tcpNoDelay = true
            out = socket!!.getOutputStream()
        }
        return connected
    }

    @Synchronized
    fun close() {
        runCatching {
            out?.close()
            socket?.close()
        }
        socket = null
        out = null
    }

    fun send(state: HeadState, left: ControllerState, right: ControllerState): Boolean {
        val stream = out ?: return false
        val b = BUFFER.get()
        b.rewind()
        b.put(HEADER)

        putHead(b, state)

        putController(b, PACK_LEFT, left)
        putController(b, PACK_RIGHT, right)

        return try {
            stream.write(b.array(), 0, b.position())
            stream.flush()
            true
        } catch (e: Exception) {
            false
        }
    }

    private fun putHead(b: java.nio.ByteBuffer, s: HeadState) {
        for (i in 0..1) {
            val off = PACK_VIEWS[i]
            b.putFloat(off, FOV[0]).putFloat(off + 4, FOV[1])
            b.putFloat(off + 8, FOV[2]).putFloat(off + 12, FOV[3])
            b.putFloat(off + 16, s.qx).putFloat(off + 20, s.qy)
            b.putFloat(off + 24, s.qz).putFloat(off + 28, s.qw)
            b.putFloat(off + 32, 0f).putFloat(off + 36, 0f).putFloat(off + 40, 0f)
            b.putInt(off + 44, 0)
        }
        b.putFloat(PACK_CENTER + 0, s.qx)
        b.putFloat(PACK_CENTER + 4, s.qy)
        b.putFloat(PACK_CENTER + 8, s.qz)
        b.putFloat(PACK_CENTER + 12, s.qw)
        b.putFloat(PACK_CENTER + 16, 0f)
        b.putFloat(PACK_CENTER + 20, 0f)
        b.putFloat(PACK_CENTER + 24, 0f)
        b.put(PACK_VALID, 1)
    }

    private fun putController(b: java.nio.ByteBuffer, off: Int, c: ControllerState) {
        b.putFloat(off + 0, c.qx).putFloat(off + 4, c.qy)
        b.putFloat(off + 8, c.qz).putFloat(off + 12, c.qw)
        b.putFloat(off + 16, c.px).putFloat(off + 20, c.py).putFloat(off + 24, c.pz)
        b.putFloat(off + 28, 0f).putFloat(off + 32, 0f).putFloat(off + 36, 0f)
        b.putFloat(off + 40, 0f).putFloat(off + 44, 0f).putFloat(off + 48, 0f)
        for (i in 0..4) b.putFloat(off + 52 + 4 * i, 0f)
        b.putFloat(off + 72, c.trigger)
        b.putFloat(off + 76, c.squeeze)
        b.putFloat(off + 80, 0f)
        b.putFloat(off + 84, c.stickX).putFloat(off + 88, c.stickY)
        b.putFloat(off + 92, 0f)
        b.putFloat(off + 96, 0f).putFloat(off + 100, 0f)
        var bools = 0
        if (c.active) bools = bools or (1 shl 1)
        if (c.system) bools = bools or (1 shl 2)
        if (c.a) bools = bools or (1 shl 4)
        if (c.b) bools = bools or (1 shl 6)
        if (c.triggerClick) bools = bools or (1 shl 8)
        if (c.stickClick) bools = bools or (1 shl 10)
        for (i in 0..15) b.put(off + 104 + i, if (bools and (1 shl i) != 0) 1 else 0)
    }

    companion object {
        private val BUFFER = object : ThreadLocal<java.nio.ByteBuffer>() {
            override fun initialValue(): java.nio.ByteBuffer {
                val b = java.nio.ByteBuffer.allocate(SIZE)
                b.order(java.nio.ByteOrder.LITTLE_ENDIAN)
                return b
            }
        }

        const val PORT_DEFAULT = 4242

        const val SIZE = 376
        private val HEADER = "mndrmt3\u0000".toByteArray(Charsets.US_ASCII)

        private val PACK_VIEWS = intArrayOf(8, 56)
        private const val PACK_CENTER = 104
        private const val PACK_VALID = 132
        private const val PACK_LEFT = 136
        private const val PACK_RIGHT = 256

        private val FOV =
            floatArrayOf(
                Math.toRadians(-50.0).toFloat(),
                Math.toRadians(50.0).toFloat(),
                Math.toRadians(50.0).toFloat(),
                Math.toRadians(-50.0).toFloat(),
            )
    }

    data class HeadState(val qx: Float, val qy: Float, val qz: Float, val qw: Float)

    data class ControllerState(
        val qx: Float,
        val qy: Float,
        val qz: Float,
        val qw: Float,
        val px: Float,
        val py: Float,
        val pz: Float,
        val trigger: Float,
        val squeeze: Float,
        val stickX: Float,
        val stickY: Float,
        val system: Boolean,
        val a: Boolean,
        val b: Boolean,
        val triggerClick: Boolean,
        val stickClick: Boolean,
        val active: Boolean,
    )
}
