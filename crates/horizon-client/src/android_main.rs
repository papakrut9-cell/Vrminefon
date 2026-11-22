//! Horizon on Android: the crate root of `horizon-android` (a NativeActivity
//! cdylib, ANDROID.md). It sits beside `main.rs` so `mod game;` compiles the
//! same game; this file only replaces `main` with `android_main`.
#![cfg(target_os = "android")]
#![allow(dead_code)]
mod android_logs;
mod android_msa;
mod android_setup;
use horizon_client::game;

use horizon_client::{crash, logging};
use winit::platform::android::activity::AndroidApp;

static MOD_API_LEVEL: [u8; 27] = horizon_mods::api_level::MARK;

/// Called by android-activity on its own thread once the activity starts.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    std::hint::black_box(&MOD_API_LEVEL);
    // The game's home: the app's external files folder
    // (/sdcard/Android/data/dev.horizonlab.horizon/files), which a file manager or
    // `adb push` can reach; internal storage when there is none.
    let home = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .unwrap_or_else(|| "/data/local/tmp/horizon".into());
    let _ = std::fs::create_dir_all(&home);
    // SAFETY: nothing else runs yet; the game reads these on this thread.
    unsafe {
        std::env::set_var("HORIZON_HOME", &home);
        if std::env::var_os("HORIZON_RESOURCES").is_none() {
            std::env::set_var("HORIZON_RESOURCES", home.join("resources"));
        }
        std::env::set_var("HOME", &home);
        if let Some(internal) = app.internal_data_path() {
            std::env::set_var("HORIZON_CACHE", internal.join("horizon-cache"));
        }
        std::env::set_var("TMPDIR", app.internal_data_path().unwrap_or(home.clone()).join("tmp"));
        // `spikes.txt` in the game folder (a number of milliseconds) turns on HORIZON_SPIKES:
        // frames slower than that log their timings, without a debug build.
        if let Ok(text) = std::fs::read_to_string(home.join("spikes.txt"))
            && text.trim().parse::<f32>().is_ok()
        {
            std::env::set_var("HORIZON_SPIKES", text.trim());
        }
    }
    let _ = std::fs::create_dir_all(home.join("..").join("cache"));
    let _ = std::env::set_current_dir(&home);
    // The folders players drop things into, ready in the Files app's Horizon root.
    for dir in ["saves", "resourcepacks", "mods", "shaderpacks"] {
        let _ = std::fs::create_dir_all(home.join(dir));
    }
    if let Some(internal) = app.internal_data_path() {
        msaa_fallback(&internal, &home.join("horizon-vr.txt"));
    }
    // The launcher's VR settings (ProfileActivity): `scale=` and `hz=` lines in horizon-vr.txt.
    if let Ok(text) = std::fs::read_to_string(home.join("horizon-vr.txt")) {
        if text.lines().any(|l| l.trim() == "pace=0") && std::env::var_os("HORIZON_XR_PACE").is_none() {
            // SAFETY: still before the game starts its threads.
            unsafe { std::env::set_var("HORIZON_XR_PACE", "off") };
        }
        if text.lines().any(|l| l.trim() == "boost=1") && std::env::var_os("HORIZON_XR_PERF").is_none() {
            // SAFETY: still before the game starts its threads.
            unsafe { std::env::set_var("HORIZON_XR_PERF", "boost") };
        }
        if text.lines().any(|l| l.trim() == "spacewarp=0")
            && std::env::var_os("HORIZON_XR_SPACEWARP").is_none()
        {
            // SAFETY: still before the game starts its threads.
            unsafe { std::env::set_var("HORIZON_XR_SPACEWARP", "0") };
        }
        if text.lines().any(|l| l.trim() == "msaa=0") && std::env::var_os("HORIZON_XR_MSAA").is_none() {
            // SAFETY: still before the game starts its threads.
            unsafe { std::env::set_var("HORIZON_XR_MSAA", "off") };
        }
        for (key, var) in [("scale", "HORIZON_XR_SCALE"), ("hz", "HORIZON_XR_HZ")] {
            let value = text
                .lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_string());
            if let Some(value) = value.filter(|v| v.parse::<f32>().is_ok())
                && std::env::var_os(var).is_none()
            {
                // SAFETY: still before the game starts its threads.
                unsafe { std::env::set_var(var, value) };
            }
        }
    }
    // The launcher's controls page (ProfileActivity): `job=button`, `sticks=swap` and `snap=`
    // lines in horizon-controls.txt, read by horizon-xr.
    if let Ok(text) = std::fs::read_to_string(home.join("horizon-controls.txt"))
        && std::env::var_os("HORIZON_XR_CONTROLS").is_none()
    {
        let joined = text.lines().map(str::trim).filter(|l| l.contains('=')).collect::<Vec<_>>().join(",");
        // SAFETY: still before the game starts its threads.
        unsafe { std::env::set_var("HORIZON_XR_CONTROLS", joined) };
    }
    // Before logging starts its thread: these become environment variables.
    let remote_env = android_logs::apply_remote_env();
    logging::init(&home, "Render thread");
    android_logs::start(&home);
    if !remote_env.is_empty() {
        log::info!("Horizon: settings from the development server: {remote_env:?}");
    }
    log::info!("Horizon {} (Android) home {}", crash::version(), home.display());
    // Development: the derived pack made on the device beside the installed one (vr-env.txt).
    if std::env::var("HORIZON_DERIVE_TEST").as_deref() == Ok("1") {
        let home = home.clone();
        let _ = std::thread::Builder::new()
            .name("Derive test".into())
            .spawn(move || crate::android_setup::derive_test(&home));
    }
    crash::install(home.clone(), "client");
    if let Some(internal) = app.internal_data_path() {
        apply_vulkan_driver(&internal);
        apply_safe_mode(&internal);
        android_msa::prepare(&internal, &home);
    }
    // VR: OpenXR attaches to this activity; the renderer opens its device through it.
    // "PLAY 2D" (the profile screen) leaves `launch-2d`: the flat game, no VR. The
    // headset's panel offers HDR (scRGB), whose path drew the sun as a black square and
    // then the world black on Quest: the panel gets the plain frame.
    let flat = app
        .internal_data_path()
        .is_some_and(|internal| internal.join("launch-2d").exists());
    if flat && std::env::var_os("HORIZON_HDR_OUTPUT").is_none() {
        // SAFETY: still before the game starts its threads.
        unsafe { std::env::set_var("HORIZON_HDR_OUTPUT", "none") };
    }
    if let Some(internal) = app.internal_data_path()
        && !flat
    {
        // SAFETY: android-activity's JavaVM and activity, valid for the whole process.
        unsafe {
            horizon_xr::set_android_context(app.vm_as_ptr(), app.activity_as_ptr(), internal);
        }
    }
    let _ = game::ANDROID_APP.set(app.clone());
    // A panic here is already in the crash report (crash::install). Catch it so the activity is
    // still finished below: a NativeActivity whose loop died shows a dead window otherwise.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(game::run_with)) {
        Ok(Err(error)) => {
            log::error!("Horizon: {error}");
            crash::report_error(&home, "client", &*error);
        }
        Ok(Ok(())) => clear_launch_marks(&app),
        Err(_) => log::error!("Horizon: the game thread panicked"),
    }
    logging::flush();
    // Quit Game only leaves the native loop. Finish the activity or the next
    // launch resumes a dead window (black, then it minimizes).
    game::finish_activity();
}

/// `vulkan-driver.txt` in internal storage (Profile -> Graphics driver, experimental) names a
/// custom Vulkan driver `.so` (Turnip). The renderer opens it itself and goes back to the system
/// driver when it can't (wgpu-hal `vulkan/instance.rs`).
///
/// A launch that dies before the game has been up for a while leaves `vulkan-driver.pending`
/// behind; the next launch sees it, turns the custom driver off and leaves a note for the
/// profile screen, so a bad driver can never lock the game out.
fn apply_vulkan_driver(internal: &std::path::Path) {
    let choice = internal.join("vulkan-driver.txt");
    let pending = internal.join("vulkan-driver.pending");
    if pending.exists() {
        let _ = std::fs::remove_file(&pending);
        if choice.exists() {
            log::warn!("Horizon: the custom Vulkan driver did not start last time; back to the system driver");
            let _ = std::fs::rename(&choice, internal.join("vulkan-driver.failed"));
            return;
        }
    }
    let Ok(text) = std::fs::read_to_string(&choice) else {
        return;
    };
    let path = text.trim();
    if path.is_empty() {
        return;
    }
    if !std::path::Path::new(path).is_file() {
        log::warn!("Horizon: Vulkan driver missing: {path}");
        return;
    }
    let _ = std::fs::write(&pending, b"1");
    log::info!("Horizon: custom Vulkan driver {path}");
    // SAFETY: still before any other thread starts.
    unsafe { std::env::set_var("HORIZON_VULKAN_LIB", path) };
    let pending = pending.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(25));
        let _ = std::fs::remove_file(pending);
    });
}

/// The player quit on purpose: however short the launch, it did not fail. Without this, opening
/// the game and quitting twice within 25 seconds turned safe start on.
fn clear_launch_marks(app: &AndroidApp) {
    if let Some(internal) = app.internal_data_path() {
        let _ = std::fs::remove_file(internal.join("launch.pending"));
        let _ = std::fs::remove_file(internal.join("vulkan-driver.pending"));
    }
}

/// MSAA that leaves the eyes black while the game runs (GitHub issue #2: Quest 3S and
/// Quest 2, music but no picture): two starts in a row with MSAA on that end within a minute
/// turn it off in the launcher's settings (`msaa=0` in horizon-vr.txt, which its button shows).
fn msaa_fallback(internal: &std::path::Path, settings: &std::path::Path) {
    let text = std::fs::read_to_string(settings).unwrap_or_default();
    if text.lines().any(|l| l.trim() == "msaa=0") {
        return;
    }
    let pending = internal.join("msaa.pending");
    let quick_ends: u32 = std::fs::read_to_string(&pending)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    if quick_ends >= 2 {
        let mut lines: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().starts_with("msaa="))
            .map(str::to_owned)
            .collect();
        lines.push("msaa=0".to_owned());
        let _ = std::fs::write(settings, lines.join("\n") + "\n");
        let _ = std::fs::remove_file(&pending);
        log::warn!("Horizon VR: MSAA off after {quick_ends} starts with it that ended within a minute");
        return;
    }
    let _ = std::fs::write(&pending, format!("{}", quick_ends + 1));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let _ = std::fs::remove_file(pending);
    });
}

/// Safe start: a launch that died within its first 25 seconds (with the system driver)
/// leaves `launch.pending`; after two such launches in a row the next ones start on
/// OpenGL ES with GPU occlusion culling off. Some drivers (Adreno 740 on certain firmware,
/// Mali on old Android) fail inside Vulkan before the first frame. `safe-mode.txt` in
/// internal storage keeps it on until the profile screen's "Try Vulkan again" deletes it.
fn apply_safe_mode(internal: &std::path::Path) {
    let pending = internal.join("launch.pending");
    let forced = internal.join("safe-mode.txt");
    let failures: u32 = std::fs::read_to_string(&pending)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    let safe = forced.exists() || failures >= 2;
    if failures >= 2 && !forced.exists() {
        let _ = std::fs::write(&forced, b"Vulkan failed to start twice; delete this file to try it again\n");
    }
    if safe && std::env::var_os("HORIZON_GRAPHICS_API").is_none() {
        log::warn!("Horizon: safe start (OpenGL ES, no GPU culling) after {failures} failed launches");
        // SAFETY: still before any other thread starts.
        unsafe {
            std::env::set_var("HORIZON_GRAPHICS_API", "opengl");
            std::env::set_var("HORIZON_GPU_CULL", "0");
        }
    }
    let _ = std::fs::write(&pending, format!("{}", failures + 1));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(25));
        let _ = std::fs::remove_file(pending);
    });
}
