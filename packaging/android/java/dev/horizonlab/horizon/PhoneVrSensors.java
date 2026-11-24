package dev.horizonlab.horizon;

import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;

/**
 * Phone VR's orientation sensors, read through the framework's SensorManager.
 * Every phone-VR app out there (Cardboard, PhoneVR, RiftCat) registers here: the
 * sensor HAL has to honor this path, where the raw NDK event queue of the first
 * try silently stopped delivering on some firmware (MediaTek). The listener hands
 * every reading over to the native cascade, which picks the freshest source and,
 * at the end of it, fuses the raw gyroscope and accelerometer itself.
 */
public class PhoneVrSensors implements SensorEventListener {
    public static native void phoneVrSensor(int type, long timestamp, float[] values);

    private final SensorManager manager;
    private final Sensor[] sensors;

    public PhoneVrSensors(SensorManager manager, int[] types) {
        this.manager = manager;
        this.sensors = new Sensor[types.length];
        for (int i = 0; i < types.length; i++) {
            this.sensors[i] = manager.getDefaultSensor(types[i]);
        }
    }

    /** Register the sensors the phone has; false when it has none of them. */
    public boolean start(int rateUs) {
        boolean any = false;
        for (Sensor sensor : sensors) {
            if (sensor != null && manager.registerListener(this, sensor, rateUs)) {
                any = true;
            }
        }
        return any;
    }

    @Override
    public void onSensorChanged(SensorEvent event) {
        phoneVrSensor(event.sensor.getType(), event.timestamp, event.values);
    }

    @Override
    public void onAccuracyChanged(Sensor sensor, int accuracy) {}
}
