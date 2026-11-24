package ru.vrminefon.bridge;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.AccessibilityServiceInfo;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.accessibility.AccessibilityEvent;

/**
 * Global input catcher: with only one window focused at a time, this
 * service is what lets a background bridge observe Joy-Con keys and stick
 * motion while a VR app sits in the foreground. Events are observed, not
 * consumed, so the game still receives them untouched.
 */
public class JoyConAccessibility extends AccessibilityService {
    @Override
    protected void onServiceConnected() {
        AccessibilityServiceInfo info = getServiceInfo();
        if (info != null) {
            info.flags |= AccessibilityServiceInfo.FLAG_REQUEST_FILTER_KEY_EVENTS
                    | AccessibilityServiceInfo.FLAG_SEND_MOTION_EVENTS;
            setServiceInfo(info);
        }
        JoyConHub.INSTANCE.onServiceConnected();
    }

    @Override
    public boolean onUnbind(android.content.Intent intent) {
        JoyConHub.INSTANCE.onServiceDisconnected();
        return super.onUnbind(intent);
    }

    @Override
    protected boolean onKeyEvent(KeyEvent event) {
        if (event != null) {
            JoyConHub.INSTANCE.onKeyEvent(event);
        }
        return false;
    }

    @Override
    public void onMotionEvent(MotionEvent event) {
        if (event != null) {
            JoyConHub.INSTANCE.onMotionEvent(event);
        }
    }

    @Override
    public void onAccessibilityEvent(AccessibilityEvent event) {
    }

    @Override
    public void onInterrupt() {
    }
}
