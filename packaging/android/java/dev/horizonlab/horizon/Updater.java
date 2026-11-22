package dev.horizonlab.horizon;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * HorizonCraft VR's updates: asks GitHub for the latest release, and installs its APK over this
 * one with the system's package installer (same signing key, so worlds and settings stay).
 */
final class Updater {
    static final String RELEASES = "https://github.com/Boboetto/HorizonCraft-VR/releases/latest";
    private static final String API = "https://api.github.com/repos/Boboetto/HorizonCraft-VR/releases/latest";
    private static final String DONE = "dev.horizonlab.horizon.UPDATE_INSTALLED";

    /** A newer release: its version and APK. */
    static final class Release {
        final String version, apk;
        final long size;

        Release(String version, String apk, long size) {
            this.version = version;
            this.apk = apk;
            this.size = size;
        }
    }

    interface Found {
        /** On the main thread: the newer release, or null when this one is the latest (or offline). */
        void found(Release release);
    }

    interface Progress {
        /** On the main thread: a status line for the button. */
        void status(String text, boolean finished);
    }

    private Updater() {}

    static String installed(Context c) {
        try {
            return c.getPackageManager().getPackageInfo(c.getPackageName(), 0).versionName;
        } catch (Exception e) {
            return "0";
        }
    }

    /** Whether `a` (like "0.3.1" or "v0.3.1") is a later version than `b`. */
    static boolean newer(String a, String b) {
        String[] x = a.replaceFirst("^v", "").split("[.-]");
        String[] y = b.replaceFirst("^v", "").split("[.-]");
        for (int i = 0; i < Math.max(x.length, y.length); i++) {
            int p = i < x.length ? number(x[i]) : 0;
            int q = i < y.length ? number(y[i]) : 0;
            if (p != q) return p > q;
        }
        return false;
    }

    private static int number(String s) {
        try {
            return Integer.parseInt(s.replaceAll("[^0-9]", ""));
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    private static int installedCode(Context c) {
        try {
            return c.getPackageManager().getPackageInfo(c.getPackageName(), 0).versionCode;
        } catch (Exception e) {
            return 0;
        }
    }

    /**
     * Test builds: the dev server's version.json ({"version", "code", "size"}) and its
     * horizon.apk, newer when its versionCode is higher (the URL carries the code, so no
     * cached older APK comes back).
     */
    private static Release checkDevServer(Context c) throws Exception {
        HttpURLConnection http = (HttpURLConnection) new URL(DevServer.URL + "/version.json").openConnection();
        http.setConnectTimeout(5000);
        http.setReadTimeout(5000);
        http.setUseCaches(false);
        if (http.getResponseCode() != 200) return null;
        JSONObject json = new JSONObject(read(http.getInputStream()));
        int code = json.optInt("code", 0);
        if (code <= installedCode(c)) return null;
        return new Release(json.optString("version", "test " + code),
                DevServer.URL + "/horizon.apk?v=" + code, json.optLong("size", 0));
    }

    static void check(final Context c, final Found found) {
        final Handler main = new Handler(Looper.getMainLooper());
        new Thread(new Runnable() {
            public void run() {
                Release release = null;
                try {
                    if (!DevServer.URL.isEmpty()) {
                        release = checkDevServer(c);
                        throw new Exception("test build: no GitHub check");
                    }
                    HttpURLConnection http = (HttpURLConnection) new URL(API).openConnection();
                    http.setConnectTimeout(8000);
                    http.setReadTimeout(8000);
                    http.setRequestProperty("Accept", "application/vnd.github+json");
                    http.setRequestProperty("User-Agent", "HorizonCraft-VR-launcher");
                    if (http.getResponseCode() == 200) {
                        JSONObject json = new JSONObject(read(http.getInputStream()));
                        String tag = json.optString("tag_name", "");
                        JSONArray assets = json.optJSONArray("assets");
                        for (int i = 0; assets != null && i < assets.length(); i++) {
                            JSONObject a = assets.getJSONObject(i);
                            if (a.optString("name").endsWith(".apk") && newer(tag, installed(c))) {
                                release = new Release(tag, a.optString("browser_download_url"), a.optLong("size"));
                            }
                        }
                    }
                } catch (Exception ignored) {
                }
                final Release r = release;
                main.post(new Runnable() {
                    public void run() {
                        found.found(r);
                    }
                });
            }
        }).start();
    }

    private static String read(InputStream in) throws java.io.IOException {
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        byte[] buf = new byte[1 << 14];
        int n;
        while ((n = in.read(buf)) != -1) out.write(buf, 0, n);
        in.close();
        return out.toString("UTF-8");
    }

    /** Downloads the release's APK, then hands it to the package installer, which asks to confirm. */
    static void install(final Activity a, final Release release, final Progress progress) {
        final Handler main = new Handler(Looper.getMainLooper());
        if (Build.VERSION.SDK_INT >= 26 && !a.getPackageManager().canRequestPackageInstalls()) {
            progress.status(Ui.t("Разрешите установку и нажмите снова", "Allow installs, then tap again"), true);
            a.startActivity(new Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + a.getPackageName())));
            return;
        }
        new Thread(new Runnable() {
            public void run() {
                File apk = new File(a.getCacheDir(), "update.apk");
                try {
                    HttpURLConnection http = (HttpURLConnection) new URL(release.apk).openConnection();
                    http.setUseCaches(false);
                    http.setConnectTimeout(15000);
                    http.setReadTimeout(30000);
                    http.setRequestProperty("User-Agent", "HorizonCraft-VR-launcher");
                    long total = release.size > 0 ? release.size : http.getContentLength();
                    try (InputStream in = http.getInputStream(); OutputStream out = new FileOutputStream(apk)) {
                        byte[] buf = new byte[1 << 16];
                        long done = 0;
                        int n, shown = -1;
                        while ((n = in.read(buf)) != -1) {
                            out.write(buf, 0, n);
                            done += n;
                            final int percent = total > 0 ? (int) (done * 100 / total) : 0;
                            if (percent != shown) {
                                shown = percent;
                                main.post(new Runnable() {
                                    public void run() {
                                        progress.status(Ui.t("Загрузка ", "Downloading ") + percent + "%", false);
                                    }
                                });
                            }
                        }
                    }
                    PackageInstaller installer = a.getPackageManager().getPackageInstaller();
                    PackageInstaller.SessionParams params =
                            new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                    params.setAppPackageName(a.getPackageName());
                    int id = installer.createSession(params);
                    try (PackageInstaller.Session session = installer.openSession(id)) {
                        try (InputStream in = new FileInputStream(apk);
                             OutputStream out = session.openWrite("horizon-vr", 0, apk.length())) {
                            byte[] buf = new byte[1 << 16];
                            int n;
                            while ((n = in.read(buf)) != -1) out.write(buf, 0, n);
                            session.fsync(out);
                        }
                        listen(a, progress);
                        int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? 0x02000000 : 0);
                        PendingIntent done = PendingIntent.getBroadcast(a, id,
                                new Intent(DONE).setPackage(a.getPackageName()), flags);
                        session.commit(done.getIntentSender());
                    }
                    main.post(new Runnable() {
                        public void run() {
                            progress.status(Ui.t("Подтвердите установку", "Confirm the install"), false);
                        }
                    });
                } catch (final Exception e) {
                    apk.delete();
                    main.post(new Runnable() {
                        public void run() {
                            progress.status(Ui.t("Ошибка: ", "Failed: ") + e.getMessage(), true);
                        }
                    });
                }
            }
        }).start();
    }

    /** The installer's answer: its confirmation screen to show, or how it ended. */
    private static void listen(final Activity a, final Progress progress) {
        BroadcastReceiver receiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context c, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        a.startActivity(confirm);
                    }
                    return;
                }
                try {
                    a.unregisterReceiver(this);
                } catch (Exception ignored) {
                }
                if (status != PackageInstaller.STATUS_SUCCESS) {
                    String why = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                    progress.status(Ui.t("Не установлено: ", "Not installed: ") + (why != null ? why : status), true);
                }
            }
        };
        IntentFilter filter = new IntentFilter(DONE);
        if (Build.VERSION.SDK_INT >= 33) {
            a.registerReceiver(receiver, filter, 4 /* Context.RECEIVER_NOT_EXPORTED */);
        } else {
            a.registerReceiver(receiver, filter);
        }
    }
}
