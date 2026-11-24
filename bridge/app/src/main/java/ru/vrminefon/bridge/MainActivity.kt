package ru.vrminefon.bridge

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Button

class MainActivity : Activity() {
    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (16 * resources.displayMetrics.density).toInt()
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
        }
        val title = TextView(this).apply {
            text = "Joy-Con bridge"
            textSize = 20f
        }
        status = TextView(this).apply { text = initialStatus() }
        val start = Button(this).apply {
            text = "Старт"
            setOnClickListener { launch(); refresh() }
        }
        val stop = Button(this).apply {
            text = "Стоп"
            setOnClickListener { stopService(Intent(this@MainActivity, BridgeService::class.java)); refresh() }
        }
        box.addView(title)
        box.addView(status)
        box.addView(start)
        box.addView(stop)
        setContentView(box)
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    private val ticker = android.os.Handler(android.os.Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            refresh()
            ticker.postDelayed(this, 500)
        }
    }

    override fun onResume() {
        super.onResume()
        tick.run()
    }

    override fun onPause() {
        super.onPause()
        ticker.removeCallbacks(tick)
    }

    private fun launch() {
        startForegroundService(Intent(this, BridgeService::class.java))
    }

    private fun refresh() {
        val gyro = (getSystemService(SENSOR_SERVICE) as android.hardware.SensorManager)
            .getDefaultSensor(android.hardware.Sensor.TYPE_GYROSCOPE)
        val gamepads = android.view.InputDevice.getDeviceIds()
            .map { android.view.InputDevice.getDevice(it) }
            .filterNotNull()
            .filter {
                it.sources and android.view.InputDevice.SOURCE_GAMEPAD != 0 ||
                    it.sources and android.view.InputDevice.SOURCE_JOYSTICK != 0
            }
            .joinToString("; ") { it.name }
        status.text = buildString {
            appendLine("Runtime: ${if (BridgeService.connectedNow) "подключён" else "не подключён"}; пакетов: ${BridgeService.packetsSent}")
            appendLine("Гироскоп: ${if (gyro != null) "есть" else "нет"}")
            appendLine("Геймпады: ${gamepads.ifEmpty { "нет (подари их в настройках Bluetooth)" }}")
            appendLine("«Старт» запускает поток позы в Monado; в шторке появится уведомление — так Android держит сервис живым")
        }
    }

    private fun initialStatus(): String = "..."
}
