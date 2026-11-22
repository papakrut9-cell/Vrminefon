package dev.horizonlab.horizon;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Bundle;
import android.text.InputFilter;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

/**
 * The launcher, landscape and on one screen: the skin with its picker and arm model, the
 * name, the account, a big Play button and every setting on the right.
 *
 * The name goes to horizon-player.txt (offline_name.rs: the name used on servers and LAN);
 * the skin, a PNG picked with the system file picker, to horizon-skin.png as a plain 64x64
 * or 64x32 RGBA PNG, and its arm model to horizon-skin.txt (game/local_skin.rs).
 */
public class ProfileActivity extends Activity implements View.OnClickListener {
    private static final int PICK = 7;
    private static final int PICK_DRIVER = 8;
    private static final int MS_LOGIN = 9;
    private File dir;
    private EditText name;
    private TextView message;
    private SkinView preview;
    private Button pick, model, play, driver, systemDriver, microsoft, deviceCode, signOut;
    private TextView driverStatus, accountStatus, accountChip;
    private EditText clientId;
    private LinearLayout codeBox, advancedBox;
    private TextView code, codeHint;
    private Button copy, browser, advanced, mods;
    private String verifyUri = "https://www.microsoft.com/link";
    private boolean slim;
    /** The home page's update badge, player card and the page shown. */
    private TextView badge, homeName, homeAccount;
    private SkinView homeSkin;
    private int shown;

    private static final int PANEL = Ui.CARD;
    private static final int FIELD = Color.rgb(30, 30, 32);

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        dir = new File(SetupActivity.home(this));
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_PAN
                | WindowManager.LayoutParams.SOFT_INPUT_STATE_HIDDEN);
        int p = Ui.dp(this, 12);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.HORIZONTAL);
        root.setBackground(Ui.sunset(this));
        int pad = Ui.dp(this, 16);
        root.setPadding(pad, pad, pad, pad);

        // Left: who plays.
        LinearLayout left = new LinearLayout(this);
        left.setOrientation(LinearLayout.VERTICAL);
        left.setPadding(p, p, p, p);
        left.setBackground(Ui.rounded(this, PANEL, Ui.EDGE, 16));
        LinearLayout head = new LinearLayout(this);
        head.setOrientation(LinearLayout.HORIZONTAL);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = Ui.text(this, Ui.t("Скин и ник", "Skin and name"), 22, Ui.TEXT, true);
        title.setLetterSpacing(0.06f);
        head.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        TextView ver = Ui.text(this, version(), 12, Ui.MUTED, false);
        head.addView(ver);
        left.addView(head);

        LinearLayout who = new LinearLayout(this);
        who.setOrientation(LinearLayout.HORIZONTAL);
        preview = new SkinView(this);
        preview.setBackground(Ui.rounded(this, FIELD, 0, 12));
        int pv = Ui.dp(this, 6);
        preview.setPadding(pv, pv, pv, pv);
        who.addView(preview, new LinearLayout.LayoutParams(0, -1, 1));
        LinearLayout side = new LinearLayout(this);
        side.setOrientation(LinearLayout.VERTICAL);
        side.setGravity(Gravity.CENTER_VERTICAL);
        LinearLayout.LayoutParams sideLp = new LinearLayout.LayoutParams(0, -1, 1.3f);
        sideLp.leftMargin = p;
        side.addView(Ui.text(this, Ui.t("Ник", "Name"), 12, Ui.MUTED, false));
        name = new EditText(this);
        name.setSingleLine(true);
        name.setTextColor(Ui.TEXT);
        name.setHintTextColor(Ui.MUTED);
        name.setHint("Player_123");
        name.setTextSize(16);
        name.setFilters(new InputFilter[] {new InputFilter.LengthFilter(16)});
        name.setPadding(p, Ui.dp(this, 8), p, Ui.dp(this, 8));
        name.setBackground(Ui.rounded(this, FIELD, Ui.EDGE, 8));
        name.setImeOptions(android.view.inputmethod.EditorInfo.IME_ACTION_DONE
                | android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        name.setText(readName());
        name.addTextChangedListener(new android.text.TextWatcher() {
            public void beforeTextChanged(CharSequence t, int a, int b, int c) {}
            public void onTextChanged(CharSequence t, int a, int b, int c) {}
            public void afterTextChanged(android.text.Editable e) {
                showHomeName();
            }
        });
        side.addView(name);
        pick = Ui.button(this, Ui.t("Скин…", "Skin…"), false);
        pick.setOnClickListener(this);
        side.addView(pick, Ui.gap(this, 8));
        slim = readModel();
        model = Ui.button(this, "", false);
        model.setOnClickListener(this);
        side.addView(model, Ui.gap(this, 6));
        accountChip = Ui.text(this, "", 12, Ui.MUTED, false);
        side.addView(accountChip, Ui.gap(this, 6));
        who.addView(side, sideLp);
        left.addView(who, new LinearLayout.LayoutParams(-1, 0, 1));

        message = Ui.text(this, "", 12, Ui.WARN, false);
        message.setMaxLines(2);
        play = Ui.button(this, Ui.t("ИГРАТЬ В VR", "PLAY VR"), true);
        play.setTextSize(22);
        play.setLetterSpacing(0.1f);
        play.setMinHeight(Ui.dp(this, 60));
        play.setOnClickListener(this);
        play.setTextColor(Color.BLACK);
        play.setShadowLayer(0, 0, 0, 0);

        // The menu's pages: each section its own scrolling page.
        LinearLayout accountBox = page(), updatesBox = page(), settingsBox = page(), modsBox = page();
        LinearLayout card = accountBox;

        Ui.section(card, Ui.t("Аккаунт Microsoft", "Microsoft account"));
        accountStatus = Ui.text(this, accountLine(), 15, Ui.TEXT, true);
        card.addView(accountStatus);
        card.addView(Ui.text(this, Ui.t(
                "Для лицензионных серверов. Войдите почтой и паролем один раз: дальше вход обновляется сам.",
                "For licensed servers. Sign in with your email and password once: it renews by itself."),
                12, Ui.MUTED, false));
        microsoft = Ui.button(this, Ui.t("Войти через Microsoft", "Sign in with Microsoft"), true);
        microsoft.setOnClickListener(this);
        card.addView(microsoft, Ui.gap(this, 8));
        codeBox = new LinearLayout(this);
        codeBox.setOrientation(LinearLayout.VERTICAL);
        codeBox.setPadding(p, p, p, p);
        codeBox.setBackground(Ui.rounded(this, FIELD, Ui.GREEN_DARK, 10));
        codeBox.setVisibility(View.GONE);
        codeHint = Ui.text(this, "", 13, Ui.MUTED, false);
        codeBox.addView(codeHint);
        code = Ui.text(this, "", 28, Ui.TEXT, true);
        code.setGravity(Gravity.CENTER);
        code.setTypeface(android.graphics.Typeface.MONOSPACE, android.graphics.Typeface.BOLD);
        code.setLetterSpacing(0.12f);
        code.setTextIsSelectable(true);
        codeBox.addView(code, Ui.gap(this, 4));
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        copy = Ui.button(this, Ui.t("Скопировать код", "Copy code"), false);
        copy.setOnClickListener(this);
        browser = Ui.button(this, Ui.t("Открыть браузер", "Open browser"), true);
        browser.setOnClickListener(this);
        row.addView(copy, new LinearLayout.LayoutParams(0, -2, 1));
        LinearLayout.LayoutParams half2 = new LinearLayout.LayoutParams(0, -2, 1);
        half2.leftMargin = Ui.dp(this, 8);
        row.addView(browser, half2);
        codeBox.addView(row, Ui.gap(this, 8));
        card.addView(codeBox, Ui.gap(this, 8));
        signOut = Ui.button(this, Ui.t("Выйти из аккаунта", "Sign out"), false);
        signOut.setOnClickListener(this);
        card.addView(signOut, Ui.gap(this, 8));
        advanced = Ui.button(this, Ui.t("Другой способ входа ▾", "Other sign-in ▾"), false);
        advanced.setOnClickListener(this);
        card.addView(advanced, Ui.gap(this, 8));
        advancedBox = new LinearLayout(this);
        advancedBox.setOrientation(LinearLayout.VERTICAL);
        advancedBox.addView(Ui.text(this, Ui.t(
                "Вход по коду через свой Azure client ID, одобренный Mojang (aka.ms/mce-reviewappid). Нужен, только если вход выше не работает.",
                "Code sign-in with your own Azure client ID approved by Mojang (aka.ms/mce-reviewappid). Only needed if the sign-in above fails."),
                12, Ui.MUTED, false));
        clientId = new EditText(this);
        clientId.setSingleLine(true);
        clientId.setTextColor(Ui.TEXT);
        clientId.setHintTextColor(Ui.MUTED);
        clientId.setHint("00000000-0000-0000-0000-000000000000");
        clientId.setText(Msa.clientId(this));
        clientId.setPadding(p, p, p, p);
        clientId.setBackground(Ui.rounded(this, FIELD, Ui.EDGE, 8));
        clientId.setImeOptions(android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        advancedBox.addView(clientId, Ui.gap(this, 6));
        deviceCode = Ui.button(this, Ui.t("Войти по коду", "Sign in with a code"), false);
        deviceCode.setOnClickListener(this);
        advancedBox.addView(deviceCode, Ui.gap(this, 6));
        advancedBox.setVisibility(View.GONE);
        card.addView(advancedBox, Ui.gap(this, 6));

        // Horizon VR's own settings and updates.
        card = updatesBox;
        Ui.section(card, Ui.t("Обновления", "Updates"));
        final TextView updateStatus = Ui.text(this, "HorizonCraft VR v" + Updater.installed(this)
                + Ui.t(" · проверка…", " · checking…"), 14, Ui.TEXT, true);
        card.addView(updateStatus);
        final Button update = Ui.button(this, "", true);
        update.setVisibility(View.GONE);
        card.addView(update, Ui.gap(this, 6));
        Updater.check(this, new Updater.Found() {
            public void found(final Updater.Release release) {
                if (release == null) {
                    updateStatus.setText("HorizonCraft VR v" + Updater.installed(ProfileActivity.this)
                            + Ui.t(" · последняя версия", " · up to date"));
                    return;
                }
                updateStatus.setText(Ui.t("Доступна новая версия ", "New version available: ") + release.version);
                badge.setText(Ui.t("Обновление ", "Update ") + release.version);
                badge.setVisibility(View.VISIBLE);
                update.setText(Ui.t("Обновить до ", "Update to ") + release.version);
                update.setVisibility(View.VISIBLE);
                update.setOnClickListener(new View.OnClickListener() {
                    public void onClick(View v) {
                        update.setEnabled(false);
                        Updater.install(ProfileActivity.this, release, new Updater.Progress() {
                            public void status(String text, boolean finished) {
                                update.setText(text);
                                if (finished) update.setEnabled(true);
                            }
                        });
                    }
                });
            }
        });

        card = settingsBox;
        Ui.section(card, Ui.t("Настройки VR", "VR settings"));
        card.addView(Ui.text(this, Ui.t(
                "Эти настройки подобраны для Quest 3: 72 Гц с SpaceWarp, ровно и резко. Лучше их не менять; если что-то изменили, кнопка ниже вернёт рекомендуемые.",
                "These settings are tuned for the Quest 3: 72 Hz with SpaceWarp, smooth and sharp. Best left as they are; if you changed something, the button below brings back the recommended ones."),
                13, Ui.WARN, false));
        final Button resetButton = Ui.button(this, Ui.t("Вернуть рекомендуемые настройки", "Restore recommended settings"), true);
        card.addView(resetButton, Ui.gap(this, 8));
        final Button paceButton = Ui.button(this, "", false);
        final Button msaaButton = Ui.button(this, "", false);
        final Button scaleButton = Ui.button(this, "", false);
        final Button boostButton = Ui.button(this, "", false);
        final Button warpButton = Ui.button(this, "", false);
        final boolean[] pace = {vrSetting("pace", 1) != 0};
        msaaOn = vrSetting("msaa", 1) != 0;
        boostOn = vrSetting("boost", 1) != 0;
        warpOn = vrSetting("spacewarp", 1) != 0;
        vrScale = vrSetting("scale", 0);
        // One mode, Quality: a sharp image with the chunks it can draw smoothly. Applied once
        // (the game's options can still be changed in the game afterwards).
        // With MSAA the edges are smooth at 1.0x: 1.4x on top of it was four times the samples
        // of 1.0x, and the game fell to a third of 90 Hz.
        // (`preset`: the defaults' version; an older file takes the new ones once.)
        if (!"quality".equals(vrMode()) || vrSetting("preset", 0) < 4) {
            vrScale = qualityScale();
            applyMode("quality", pace[0]);
        }
        final TextView summary = Ui.text(this, "", 13, Ui.TEXT, true);
        final Runnable showVr = new Runnable() {
            public void run() {
                paceButton.setText(Ui.t("Ровные кадры: ", "Frame pacing: ") + (pace[0] ? "Auto" : "Off"));
                msaaButton.setText("Antialiasing MSAA: " + (msaaOn ? "4x" : "Off"));
                scaleButton.setText(String.format(java.util.Locale.ROOT, Ui.t("Разрешение: %.2fx", "Resolution: %.2fx"), vrScale));
                boostButton.setText(Ui.t("Ускорение: ", "Performance boost: ") + (boostOn ? "On" : "Off"));
                warpButton.setText("SpaceWarp: " + (warpOn ? "On" : "Off"));
                summary.setText(String.format(java.util.Locale.ROOT,
                        "Quality · %.2fx · 72 Hz · 6 chunks · anisotropic 8x%s", vrScale, msaaOn ? " · MSAA 4x" : ""));
            }
        };
        showVr.run();
        resetButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                msaaOn = true;
                boostOn = true;
                warpOn = true;
                pace[0] = true;
                vrScale = qualityScale();
                applyMode("quality", true);
                showVr.run();
                resetButton.setText(Ui.t("Рекомендуемые настройки ✓", "Recommended settings ✓"));
            }
        });
        paceButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                pace[0] = !pace[0];
                saveVr(vrScale, 72, pace[0]);
                showVr.run();
            }
        });
        card.addView(summary);
        msaaButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                msaaOn = !msaaOn;
                vrScale = qualityScale();
                saveVr(vrScale, 72, pace[0]);
                showVr.run();
            }
        });
        scaleButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                // The next step up, then round again: sharper costs frames.
                float[] steps = msaaOn ? new float[] {1.0f, 1.15f, 1.3f} : new float[] {1.2f, 1.4f};
                float next = steps[0];
                for (float step : steps) {
                    if (step > vrScale + 0.01f) {
                        next = step;
                        break;
                    }
                }
                vrScale = next;
                saveVr(vrScale, 72, pace[0]);
                showVr.run();
            }
        });
        warpButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                warpOn = !warpOn;
                saveVr(vrScale, 72, pace[0]);
                showVr.run();
            }
        });
        boostButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                boostOn = !boostOn;
                saveVr(vrScale, 72, pace[0]);
                showVr.run();
            }
        });
        card.addView(scaleButton, Ui.gap(this, 6));
        card.addView(Ui.text(this, Ui.t(
                "Разрешение: выше — резче, но меньше кадров.",
                "Resolution: higher is sharper, but draws fewer frames. Try a step up or down to find what your eyes prefer."),
                12, Ui.MUTED, false));
        card.addView(boostButton, Ui.gap(this, 6));
        card.addView(Ui.text(this, Ui.t(
                "Ускорение: процессор и графика шлема на максимуме. Больше кадров, шлем теплее, батарея садится быстрее.",
                "Performance boost: the headset's CPU and GPU at their highest clocks. More frames; the headset runs warmer and the battery drains faster."),
                12, Ui.MUTED, false));
        card.addView(warpButton, Ui.gap(this, 6));
        card.addView(Ui.text(this, Ui.t(
                "SpaceWarp: игра рисует половину кадров, шлем досчитывает остальные. Плавнее при большей дальности.",
                "SpaceWarp: the game draws half the frames and the headset makes up the rest from how everything moves. Smoother with more chunks; turn it off if you see warping."),
                12, Ui.MUTED, false));
        card.addView(paceButton, Ui.gap(this, 6));
        card.addView(msaaButton, Ui.gap(this, 6));
        card.addView(Ui.text(this, Ui.t(
                "MSAA сглаживает края блоков вдали. Если игра не запускается или тормозит, выключите его.",
                "MSAA smooths the shimmering edges of far blocks. If the game does not start or runs slowly, turn it off."),
                12, Ui.MUTED, false));
        card.addView(Ui.text(this, Ui.t(
                "Ровные кадры: если игра не успевает, она рисует каждый второй кадр ровно, без рывков.",
                "Frame pacing: when the game cannot keep up, it draws every second frame evenly instead of unevenly, without micro-stutter."),
                12, Ui.MUTED, false));

        // Horizon VR: no Vulkan driver choice (the headset's runtime always uses the system's)
        // and no screen cutout setting (the headset has no cutout).
        card = modsBox;
        Ui.section(card, Ui.t("Моды", "Mods"));
        card.addView(Ui.text(this, Ui.t(
                ".hnmod со сборкой для Android или .hmod. Или положите файл в Android/data/dev.horizonlab.horizon/files/mods/.",
                "A .hnmod with an Android build, or a .hmod. Or drop the file into Android/data/dev.horizonlab.horizon/files/mods/."),
                12, Ui.MUTED, false));
        mods = Ui.button(this, Ui.t("Установить мод", "Install a mod"), false);
        mods.setOnClickListener(this);
        card.addView(mods, Ui.gap(this, 8));
        packsBox = new LinearLayout(this);
        packsBox.setOrientation(LinearLayout.VERTICAL);
        card.addView(packsBox, Ui.gap(this, 4));
        fillPacks();

        // The layout: a menu down the left (icons and names), the pages beside it. The home
        // page is the night landscape with the player's card and Play at the bottom, and the
        // update badge at the top right.
        root.setPadding(0, 0, 0, 0);
        root.setBackground(Ui.sunset(this));
        final FrameLayout content = new FrameLayout(this);
        FrameLayout home = new FrameLayout(this);
        // Top: the name and, on the right, the badge for a new version.
        LinearLayout top = new LinearLayout(this);
        top.setOrientation(LinearLayout.HORIZONTAL);
        top.setGravity(Gravity.CENTER_VERTICAL);
        top.setPadding(Ui.dp(this, 28), Ui.dp(this, 22), Ui.dp(this, 22), 0);
        LinearLayout brandBox = new LinearLayout(this);
        brandBox.setOrientation(LinearLayout.VERTICAL);
        TextView brand = Ui.text(this, "HorizonCraft VR", 34, Ui.TEXT, true);
        brand.setLetterSpacing(0.02f);
        brand.setShadowLayer(Ui.dp(this, 8), 0, Ui.dp(this, 2), Color.argb(160, 0, 0, 0));
        brandBox.addView(brand);
        TextView tagline = Ui.text(this, "Minecraft: Java Edition 26.3 · Meta Quest 3", 14, Ui.MUTED, false);
        brandBox.addView(tagline);
        top.addView(brandBox, new LinearLayout.LayoutParams(0, -2, 1));
        badge = new TextView(this);
        badge.setTextSize(14);
        badge.setTextColor(Color.WHITE);
        badge.setTypeface(android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL));
        badge.setGravity(Gravity.CENTER);
        badge.setPadding(Ui.dp(this, 16), Ui.dp(this, 8), Ui.dp(this, 16), Ui.dp(this, 8));
        badge.setBackground(Ui.pressable(this, Ui.GREEN, 0, 20));
        badge.setCompoundDrawablePadding(Ui.dp(this, 8));
        badge.setCompoundDrawables(Ui.icon(this, Ui.Glyph.UPDATES, Color.WHITE, 18), null, null, null);
        badge.setVisibility(View.GONE);
        top.addView(badge, new LinearLayout.LayoutParams(-2, -2));
        home.addView(top, new FrameLayout.LayoutParams(-1, -2, Gravity.TOP));
        // Bottom: the player's card and Play.
        LinearLayout bar = new LinearLayout(this);
        bar.setOrientation(LinearLayout.HORIZONTAL);
        bar.setGravity(Gravity.CENTER_VERTICAL);
        bar.setBackground(Ui.rounded(this, Ui.CARD, Ui.EDGE, 20));
        bar.setElevation(Ui.dp(this, 8));
        bar.setPadding(Ui.dp(this, 14), Ui.dp(this, 14), Ui.dp(this, 14), Ui.dp(this, 14));
        homeSkin = new SkinView(this);
        homeSkin.setBackground(Ui.rounded(this, FIELD, 0, 12));
        int hs = Ui.dp(this, 6);
        homeSkin.setPadding(hs, hs, hs, hs);
        bar.addView(homeSkin, new LinearLayout.LayoutParams(Ui.dp(this, 56), Ui.dp(this, 84)));
        LinearLayout info = new LinearLayout(this);
        info.setOrientation(LinearLayout.VERTICAL);
        info.setPadding(Ui.dp(this, 14), 0, Ui.dp(this, 14), 0);
        homeName = Ui.text(this, "", 18, Ui.TEXT, true);
        info.addView(homeName);
        homeAccount = Ui.text(this, "", 13, Ui.MUTED, false);
        info.addView(homeAccount);
        info.addView(message);
        bar.addView(info, new LinearLayout.LayoutParams(0, -2, 1));
        play.setText(Ui.t("  ИГРАТЬ", "  PLAY"));
        play.setTextSize(22);
        play.setLetterSpacing(0.12f);
        play.setCompoundDrawables(Ui.icon(this, Ui.Glyph.PLAY, Color.WHITE, 22), null, null, null);
        play.setPadding(Ui.dp(this, 28), 0, Ui.dp(this, 32), 0);
        play.setBackground(Ui.pressable(this, Ui.GREEN, 0, 14));
        play.setTextColor(Color.WHITE);
        bar.addView(play, new LinearLayout.LayoutParams(Ui.dp(this, 240), Ui.dp(this, 68)));
        FrameLayout.LayoutParams barLp = new FrameLayout.LayoutParams(-1, -2, Gravity.BOTTOM);
        int bm = Ui.dp(this, 22);
        barLp.setMargins(bm, bm, bm, bm);
        home.addView(bar, barLp);
        left.setPadding(Ui.dp(this, 24), Ui.dp(this, 20), Ui.dp(this, 24), Ui.dp(this, 24));
        left.setBackground(Ui.rounded(this, Ui.CARD, Ui.EDGE, 20));
        FrameLayout skinPage = new FrameLayout(this);
        int sm = Ui.dp(this, 20);
        skinPage.setPadding(sm, sm, sm, sm);
        skinPage.addView(left, new FrameLayout.LayoutParams(-1, -1));
        final View[] pages = {home, skinPage, wrap(settingsBox), wrap(controlsPage()), wrap(modsBox), wrap(accountBox), wrap(updatesBox)};
        final Ui.Glyph[] glyphs = {Ui.Glyph.HOME, Ui.Glyph.SKIN, Ui.Glyph.SETTINGS, Ui.Glyph.CONTROLS, Ui.Glyph.MODS, Ui.Glyph.ACCOUNT, Ui.Glyph.UPDATES};
        final String[] names = {
            Ui.t("Главная", "Home"),
            Ui.t("Скин", "Skin"),
            Ui.t("Настройки", "Settings"),
            Ui.t("Управление", "Controls"),
            Ui.t("Моды и паки", "Mods & packs"),
            Ui.t("Аккаунт", "Account"),
            Ui.t("Обновления", "Updates"),
        };
        for (View page : pages) content.addView(page, new FrameLayout.LayoutParams(-1, -1));
        LinearLayout menu = new LinearLayout(this);
        menu.setOrientation(LinearLayout.VERTICAL);
        menu.setBackgroundColor(Color.argb(235, 16, 20, 27));
        menu.setPadding(Ui.dp(this, 12), Ui.dp(this, 18), Ui.dp(this, 12), Ui.dp(this, 12));
        menu.setElevation(Ui.dp(this, 6));
        LinearLayout logo = new LinearLayout(this);
        logo.setOrientation(LinearLayout.HORIZONTAL);
        logo.setGravity(Gravity.CENTER_VERTICAL);
        logo.setPadding(Ui.dp(this, 10), 0, 0, Ui.dp(this, 16));
        View mark = new View(this);
        mark.setBackground(Ui.rounded(this, Ui.GREEN, 0, 6));
        logo.addView(mark, new LinearLayout.LayoutParams(Ui.dp(this, 22), Ui.dp(this, 22)));
        TextView logoText = Ui.text(this, "HorizonCraft", 17, Ui.TEXT, true);
        logoText.setPadding(Ui.dp(this, 10), 0, 0, 0);
        logo.addView(logoText);
        menu.addView(logo);
        final TextView[] entries = new TextView[pages.length];
        final Runnable refresh = new Runnable() {
            public void run() {
                for (int i = 0; i < pages.length; i++) {
                    boolean on = i == shown;
                    pages[i].setVisibility(on ? View.VISIBLE : View.GONE);
                    entries[i].setTextColor(on ? Ui.TEXT : Ui.MUTED);
                    entries[i].setCompoundDrawables(Ui.icon(ProfileActivity.this, glyphs[i], on ? Ui.GREEN : Ui.MUTED, 22), null, null, null);
                    entries[i].setBackground(on
                            ? Ui.pressable(ProfileActivity.this, Ui.STONE, 0, 12)
                            : Ui.pressable(ProfileActivity.this, Color.argb(0, 16, 20, 27), 0, 12));
                }
            }
        };
        for (int i = 0; i < pages.length; i++) {
            final int index = i;
            TextView entry = Ui.text(this, names[i], 15, Ui.MUTED, true);
            entry.setGravity(Gravity.CENTER_VERTICAL);
            entry.setCompoundDrawablePadding(Ui.dp(this, 14));
            entry.setPadding(Ui.dp(this, 14), 0, Ui.dp(this, 10), 0);
            entry.setClickable(true);
            entry.setFocusable(true);
            entry.setOnClickListener(new View.OnClickListener() {
                public void onClick(View v) {
                    shown = index;
                    refresh.run();
                }
            });
            entries[i] = entry;
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, Ui.dp(this, 50));
            lp.topMargin = Ui.dp(this, 4);
            menu.addView(entry, lp);
        }
        View spacer = new View(this);
        menu.addView(spacer, new LinearLayout.LayoutParams(-1, 0, 1));
        TextView menuVersion = Ui.text(this, "HorizonCraft VR " + version(), 12, Ui.MUTED, false);
        menuVersion.setPadding(Ui.dp(this, 10), 0, 0, 0);
        menu.addView(menuVersion);
        badge.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                shown = pages.length - 1;
                refresh.run();
            }
        });
        root.addView(menu, new LinearLayout.LayoutParams(Ui.dp(this, 220), -1));
        root.addView(content, new LinearLayout.LayoutParams(0, -1, 1));
        refresh.run();
        setContentView(root);
        hideBars();
        refreshAccount();
        updateModel();
        showPreview();
    }

    /** The name on the home page's card: the typed one, else the game's own default. */
    private void showHomeName() {
        if (homeName == null) return;
        String n = Msa.signedIn(this);
        if (n.isEmpty()) n = name.getText().toString().trim();
        homeName.setText(n.isEmpty() ? "Player" : n);
    }

    @Override
    public void onWindowFocusChanged(boolean focus) {
        super.onWindowFocusChanged(focus);
        if (focus) hideBars();
    }

    private void hideBars() {
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            android.view.WindowInsetsController c = getWindow().getInsetsController();
            if (c != null) {
                c.hide(android.view.WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        }
    }

    /** A page of the menu: a padded column, scrolled by `wrap`. */
    private LinearLayout page() {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(Ui.dp(this, 24), Ui.dp(this, 8), Ui.dp(this, 24), Ui.dp(this, 24));
        return box;
    }

    private View wrap(LinearLayout box) {
        ScrollView scroll = new ScrollView(this);
        scroll.setVerticalScrollBarEnabled(false);
        // A dark panel over the stone bricks, so the text reads.
        box.setBackground(Ui.rounded(this, Ui.CARD, Ui.EDGE, 20));
        FrameLayout frame = new FrameLayout(this);
        int m = Ui.dp(this, 20);
        frame.setPadding(m, m, m, m);
        frame.addView(box, new FrameLayout.LayoutParams(-1, -2));
        scroll.addView(frame);
        return scroll;
    }

    private LinearLayout packsBox;

    /** What is installed, folder by folder, each file with a button to remove it. */
    private void fillPacks() {
        if (packsBox == null) return;
        packsBox.removeAllViews();
        String[][] folders = {
            {"mods", Ui.t("Моды (.hnmod)", "Mods (.hnmod)")},
            {"horizon-mods", Ui.t("Моды контента (.hmod)", "Content mods (.hmod)")},
            {"resourcepacks", Ui.t("Ресурспаки", "Resource packs")},
            {"shaderpacks", Ui.t("Шейдеры", "Shader packs")},
        };
        for (String[] folder : folders) {
            Ui.section(packsBox, folder[1]);
            File[] files = new File(dir, folder[0]).listFiles();
            if (files != null) java.util.Arrays.sort(files);
            int shownFiles = 0;
            for (final File f : files == null ? new File[0] : files) {
                if (f.getName().startsWith(".")) continue;
                shownFiles++;
                LinearLayout row = new LinearLayout(this);
                row.setOrientation(LinearLayout.HORIZONTAL);
                row.setGravity(Gravity.CENTER_VERTICAL);
                row.setPadding(Ui.dp(this, 14), Ui.dp(this, 6), Ui.dp(this, 6), Ui.dp(this, 6));
                row.setBackground(Ui.rounded(this, FIELD, 0, 10));
                LinearLayout names = new LinearLayout(this);
                names.setOrientation(LinearLayout.VERTICAL);
                names.addView(Ui.text(this, f.getName(), 14, Ui.TEXT, true));
                long kb = f.isDirectory() ? 0 : Math.max(1, f.length() / 1024);
                names.addView(Ui.text(this, f.isDirectory() ? Ui.t("папка", "folder") : (kb >= 1024 ? kb / 1024 + " MB" : kb + " KB"), 12, Ui.MUTED, false));
                row.addView(names, new LinearLayout.LayoutParams(0, -2, 1));
                final Button remove = Ui.button(this, Ui.t("Удалить", "Remove"), false);
                remove.setMinHeight(Ui.dp(this, 38));
                remove.setOnClickListener(new View.OnClickListener() {
                    public void onClick(View v) {
                        // A second press removes it: no file goes on one stray click.
                        if (remove.getTag() == null) {
                            remove.setTag(Boolean.TRUE);
                            remove.setText(Ui.t("Точно удалить?", "Remove for good?"));
                            remove.setTextColor(Ui.WARN);
                            return;
                        }
                        deleteAll(f);
                        fillPacks();
                    }
                });
                row.addView(remove, new LinearLayout.LayoutParams(-2, -2));
                packsBox.addView(row, Ui.gap(this, 6));
            }
            if (shownFiles == 0) packsBox.addView(Ui.text(this, Ui.t("Пусто", "Nothing here yet"), 13, Ui.MUTED, false));
        }
        packsBox.addView(Ui.text(this, Ui.t(
                "Ресурспаки и шейдеры включаются в игре (Настройки → Пакеты ресурсов). Файлы можно положить в Android/data/dev.horizonlab.horizon/files/ через приложение Файлы.",
                "Resource packs and shaders are turned on in the game (Options → Resource Packs). Files can also be copied into Android/data/dev.horizonlab.horizon/files/ with the Files app."),
                12, Ui.MUTED, false), Ui.gap(this, 12));
    }

    private static void deleteAll(File f) {
        File[] inside = f.listFiles();
        if (inside != null) for (File c : inside) deleteAll(c);
        f.delete();
    }

    @Override
    protected void onResume() {
        super.onResume();
        // A mod installed meanwhile (ModImportActivity) shows in the list.
        fillPacks();
    }

    /** horizon-controls.txt in the game folder: the controls page's choices (android_main). */
    private File controlsFile() {
        return new File(dir, "horizon-controls.txt");
    }

    /** The physical buttons, in horizon-xr's `BUTTON_NAMES` order. */
    private static final String[] BUTTON_KEYS = {"a", "b", "x", "y", "lstick", "rstick", "lgrip", "rgrip"};

    private static String buttonName(int i) {
        String[] names = {"A", "B", "X", "Y", Ui.t("Клик левого стика", "Left stick click"),
            Ui.t("Клик правого стика", "Right stick click"), Ui.t("Левый хват", "Left grip"), Ui.t("Правый хват", "Right grip")};
        return names[i];
    }

    /** The controls page: each job's button (a press gives it the next one, swapped with
     * the job that had it, so every job keeps a button), the sticks and the turning. */
    private LinearLayout controlsPage() {
        final LinearLayout box = page();
        final int[] buttons = {0, 1, 2, 3, 4, 5, 6, 7};
        final boolean[] swap = {false};
        final int[] snap = {0};
        try {
            for (String line : new String(Files.readAllBytes(controlsFile().toPath()), StandardCharsets.UTF_8).split("\n")) {
                String[] kv = line.trim().split("=", 2);
                if (kv.length != 2) continue;
                if (kv[0].equals("sticks")) swap[0] = kv[1].trim().equals("swap");
                else if (kv[0].equals("snap")) snap[0] = Integer.parseInt(kv[1].trim());
                else {
                    int job = java.util.Arrays.asList(BUTTON_KEYS).indexOf(kv[0].trim());
                    int button = java.util.Arrays.asList(BUTTON_KEYS).indexOf(kv[1].trim());
                    if (job >= 0 && button >= 0) buttons[job] = button;
                }
            }
        } catch (Exception ignored) {
            // No file yet: the default controls.
        }
        final String[] jobs = {
            Ui.t("Прыжок", "Jump"),
            Ui.t("Чат / назад · быстрый перенос в инвентаре", "Chat / back · quick move in inventories"),
            Ui.t("Выбросить предмет", "Drop item"),
            Ui.t("Инвентарь", "Inventory"),
            Ui.t("Бег", "Sprint"),
            Ui.t("Присесть", "Sneak"),
            Ui.t("Сменить руку · половина стака в меню", "Swap hands · half a stack in menus"),
            Ui.t("Один предмет в меню · Shift-клик", "Place one in menus · shift-click"),
        };
        Ui.section(box, Ui.t("Кнопки", "Buttons"));
        box.addView(Ui.text(this, Ui.t(
                "Нажмите на кнопку справа, чтобы дать действию другую кнопку. Действие, у которого она была, получит прежнюю. Применяется при следующем запуске игры.",
                "Press the button on the right to give that action another button; the action that had it takes the old one. Applies the next time the game starts."),
                12, Ui.MUTED, false));
        final Button[] choice = new Button[jobs.length];
        final Button swapButton = Ui.button(this, "", false);
        final Button snapButton = Ui.button(this, "", false);
        final Runnable save = new Runnable() {
            public void run() {
                StringBuilder out = new StringBuilder();
                for (int i = 0; i < buttons.length; i++) {
                    if (buttons[i] != i) out.append(BUTTON_KEYS[i]).append('=').append(BUTTON_KEYS[buttons[i]]).append('\n');
                }
                if (swap[0]) out.append("sticks=swap\n");
                if (snap[0] > 0) out.append("snap=").append(snap[0]).append('\n');
                try {
                    write(controlsFile(), out.toString().getBytes(StandardCharsets.UTF_8));
                } catch (Exception e) {
                    message.setText(Ui.t("Не удалось сохранить.", "Could not save."));
                }
                for (int i = 0; i < choice.length; i++) {
                    choice[i].setText(buttonName(buttons[i]));
                    choice[i].setTextColor(buttons[i] != i ? Ui.GREEN : Ui.TEXT);
                }
                swapButton.setText(swap[0]
                        ? Ui.t("Ходьба: правый стик, поворот: левый", "Walk: right stick · turn: left stick")
                        : Ui.t("Ходьба: левый стик, поворот: правый", "Walk: left stick · turn: right stick"));
                snapButton.setText(snap[0] == 0
                        ? Ui.t("Поворот: плавный", "Turning: smooth")
                        : Ui.t("Поворот: рывком ", "Turning: snap ") + snap[0] + "°");
            }
        };
        for (int i = 0; i < jobs.length; i++) {
            final int job = i;
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.HORIZONTAL);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.addView(Ui.text(this, jobs[i], 14, Ui.TEXT, false), new LinearLayout.LayoutParams(0, -2, 1));
            choice[i] = Ui.button(this, "", false);
            choice[i].setOnClickListener(new View.OnClickListener() {
                public void onClick(View v) {
                    int next = (buttons[job] + 1) % buttons.length;
                    for (int other = 0; other < buttons.length; other++) {
                        if (buttons[other] == next) buttons[other] = buttons[job];
                    }
                    buttons[job] = next;
                    save.run();
                }
            });
            row.addView(choice[i], new LinearLayout.LayoutParams(Ui.dp(this, 200), -2));
            box.addView(row, Ui.gap(this, 6));
        }
        box.addView(Ui.text(this, Ui.t(
                "Курки всегда: правый — атака и выбор, левый — использовать. Кнопка меню — пауза.",
                "The triggers stay as they are: right attacks and picks, left uses. The menu button pauses."),
                12, Ui.MUTED, false), Ui.gap(this, 6));
        Ui.section(box, Ui.t("Стики и поворот", "Sticks and turning"));
        swapButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                swap[0] = !swap[0];
                save.run();
            }
        });
        box.addView(swapButton, Ui.gap(this, 4));
        box.addView(Ui.text(this, Ui.t("Для левшей: ходьба правым стиком.", "For left-handed players: walk with the right stick."), 12, Ui.MUTED, false));
        snapButton.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                int[] steps = {0, 30, 45, 90};
                int at = 0;
                for (int i = 0; i < steps.length; i++) if (steps[i] == snap[0]) at = i;
                snap[0] = steps[(at + 1) % steps.length];
                save.run();
            }
        });
        box.addView(snapButton, Ui.gap(this, 8));
        box.addView(Ui.text(this, Ui.t(
                "Поворот рывком меньше укачивает: стик в сторону поворачивает сразу на шаг.",
                "Snap turning is easier on the stomach: the stick pushed aside turns at once by the step."),
                12, Ui.MUTED, false));
        final Button reset = Ui.button(this, Ui.t("Вернуть управление по умолчанию", "Restore the default controls"), false);
        reset.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                for (int i = 0; i < buttons.length; i++) buttons[i] = i;
                swap[0] = false;
                snap[0] = 0;
                save.run();
            }
        });
        box.addView(reset, Ui.gap(this, 16));
        save.run();
        return box;
    }

    /** horizon-vr.txt in the game folder: the VR settings the game reads at start (android_main). */
    private File vrFile() {
        return new File(SetupActivity.home(this), "horizon-vr.txt");
    }

    private float vrSetting(String key, float fallback) {
        try (java.io.BufferedReader r = new java.io.BufferedReader(new java.io.FileReader(vrFile()))) {
            String line;
            while ((line = r.readLine()) != null) {
                String[] kv = line.split("=", 2);
                if (kv.length == 2 && kv[0].trim().equals(key)) return Float.parseFloat(kv[1].trim());
            }
        } catch (Exception ignored) {
        }
        return fallback;
    }

    /** The mode saved in horizon-vr.txt, or null before one was chosen. */
    private String vrMode() {
        try (java.io.BufferedReader r = new java.io.BufferedReader(new java.io.FileReader(vrFile()))) {
            String line;
            while ((line = r.readLine()) != null) {
                if (line.startsWith("mode=")) return line.substring(5).trim();
            }
        } catch (Exception ignored) {
        }
        return null;
    }

    /**
     * A mode: the headset's resolution and refresh rate (horizon-vr.txt, read by the game at
     * start) and the game's heaviest options (options.txt in the game folder).
     */
    private void applyMode(String mode, boolean pace) {
        float scale;
        int hz, distance, simulation, biome;
        String clouds, particles, entities;
        // Anisotropic filtering keeps far, slanted block faces sharp (textureFiltering 2).
        int filtering = mode.equals("performance") ? 0 : 2;
        int anisotropy = mode.equals("quality") ? 3 : 2;
        // All three play smoothly at 72 Hz: Quality spends the GPU on a sharper image and keeps
        // fewer chunks, Performance spends it on chunks with a softer image, Balanced sits between.
        if (mode.equals("performance")) {
            scale = 0.8f; hz = 72; distance = 12; simulation = 8; clouds = "\"false\""; particles = "1"; biome = 1; entities = "1.0";
        } else if (mode.equals("quality")) {
            // Low game options under the sharp image: no clouds, fewer particles, plain biome
            // blending, a short simulation distance.
            scale = vrScale; hz = 72; distance = 6; simulation = 5; clouds = "\"false\""; particles = "1"; biome = 1; entities = "1.0";
        } else {
            scale = 1.0f; hz = 72; distance = 9; simulation = 8; clouds = "\"fast\""; particles = "0"; biome = 2; entities = "1.0";
        }
        try (java.io.FileWriter w = new java.io.FileWriter(vrFile())) {
            w.write(String.format(java.util.Locale.ROOT, "mode=%s\nscale=%.2f\nhz=%d\npace=%d\nmsaa=%d\nboost=%d\nspacewarp=%d\npreset=4\n", mode, scale, hz, pace ? 1 : 0, msaaOn ? 1 : 0, boostOn ? 1 : 0, warpOn ? 1 : 0));
        } catch (Exception ignored) {
        }
        java.util.LinkedHashMap<String, String> set = new java.util.LinkedHashMap<>();
        set.put("renderDistance", String.valueOf(distance));
        set.put("simulationDistance", String.valueOf(simulation));
        set.put("renderClouds", clouds);
        set.put("particles", particles);
        set.put("biomeBlendRadius", String.valueOf(biome));
        set.put("entityDistanceScaling", entities);
        set.put("mipmapLevels", "4");
        set.put("textureFiltering", String.valueOf(filtering));
        set.put("maxAnisotropyBit", String.valueOf(anisotropy));
        File options = new File(SetupActivity.home(this), "options.txt");
        java.util.List<String> lines = new java.util.ArrayList<>();
        try (java.io.BufferedReader r = new java.io.BufferedReader(new java.io.FileReader(options))) {
            String line;
            while ((line = r.readLine()) != null) lines.add(line);
        } catch (Exception ignored) {
        }
        for (java.util.Map.Entry<String, String> e : set.entrySet()) {
            boolean found = false;
            for (int i = 0; i < lines.size(); i++) {
                if (lines.get(i).startsWith(e.getKey() + ":")) {
                    lines.set(i, e.getKey() + ":" + e.getValue());
                    found = true;
                }
            }
            if (!found) lines.add(e.getKey() + ":" + e.getValue());
        }
        try (java.io.FileWriter w = new java.io.FileWriter(options)) {
            for (String line : lines) w.write(line + "\n");
        } catch (Exception ignored) {
        }
    }

    /** VR's Application SpaceWarp (horizon-vr.txt `spacewarp=`), on by default. */
    private boolean warpOn = true;

    /** VR's MSAA (horizon-vr.txt `msaa=`), on by default. */
    private boolean msaaOn = true;

    /** The headset's CPU and GPU at their highest clocks (horizon-vr.txt `boost=`). */
    private boolean boostOn = true;
    /** The eyes' resolution (horizon-vr.txt `scale=`). */
    private float vrScale = 1.3f;

    /** Quality's resolution to start from: 1.3x with MSAA, 1.4x without. */
    private float qualityScale() {
        return msaaOn ? 1.3f : 1.4f;
    }

    private void saveVr(float scale, int hz, boolean pace) {
        try (java.io.FileWriter w = new java.io.FileWriter(vrFile())) {
            w.write(String.format(java.util.Locale.ROOT, "mode=quality\nscale=%.2f\nhz=%d\npace=%d\nmsaa=%d\nboost=%d\nspacewarp=%d\npreset=4\n", scale, hz, pace ? 1 : 0, msaaOn ? 1 : 0, boostOn ? 1 : 0, warpOn ? 1 : 0));
        } catch (Exception ignored) {
        }
    }

    private String version() {
        try {
            return "v" + getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (Exception e) {
            return "";
        }
    }

    private String readName() {
        try {
            for (String l : new String(Files.readAllBytes(new File(dir, "horizon-player.txt").toPath()), StandardCharsets.UTF_8).split("\n")) {
                if (l.startsWith("name:")) return l.substring(5).trim();
            }
        } catch (Exception e) {
            // No name yet: the game makes one.
        }
        return "";
    }

    private boolean readModel() {
        try {
            return new String(Files.readAllBytes(new File(dir, "horizon-skin.txt").toPath()), StandardCharsets.UTF_8).contains("model:slim");
        } catch (Exception e) {
            return false;
        }
    }

    private void updateModel() {
        model.setText(Ui.t("Руки: ", "Arms: ") + (slim ? Ui.t("тонкие (Alex)", "slim (Alex)") : Ui.t("обычные (Steve)", "classic (Steve)")));
    }

    static boolean validName(String n) {
        return n.matches("[A-Za-z0-9_]{3,16}");
    }

    private void refreshAccount() {
        String n = Msa.signedIn(this);
        accountStatus.setText(accountLine());
        signOut.setVisibility(n.isEmpty() ? View.GONE : View.VISIBLE);
        microsoft.setVisibility(n.isEmpty() ? View.VISIBLE : View.GONE);
        accountChip.setText(n.isEmpty() ? Ui.t("● Без лицензии (офлайн)", "● Offline (no license)") : Ui.t("● Лицензия: ", "● Licensed: ") + n);
        accountChip.setTextColor(n.isEmpty() ? Ui.MUTED : Ui.GREEN);
        if (homeAccount != null) {
            homeAccount.setText(accountChip.getText());
            homeAccount.setTextColor(n.isEmpty() ? Ui.MUTED : Ui.GREEN);
        }
        showHomeName();
    }

    /** Saves the name and arm model; false (with a message) for a bad name. */
    private boolean saveProfile() {
        String n = name.getText().toString().trim();
        if (!n.isEmpty() && !validName(n)) {
            message.setTextColor(Ui.WARN);
            message.setText(Ui.t("Ник: 3–16 символов, только латиница, цифры и _.", "Name: 3–16 characters, letters, digits and _ only."));
            return false;
        }
        try {
            if (!n.isEmpty() && !n.equals(readName())) {
                write(new File(dir, "horizon-player.txt"), ("name:" + n + "\n").getBytes(StandardCharsets.UTF_8));
            }
            saveModel();
            return true;
        } catch (Exception e) {
            message.setText(Ui.t("Не удалось сохранить.", "Could not save."));
            return false;
        }
    }

    private void saveModel() throws Exception {
        if (readModel() == slim && new File(dir, "horizon-skin.txt").isFile()) return;
        write(new File(dir, "horizon-skin.txt"), ("model:" + (slim ? "slim" : "classic") + "\n").getBytes(StandardCharsets.UTF_8));
    }

    @Override
    protected void onPause() {
        super.onPause();
        // Leaving for the game or the file picker keeps what was typed.
        String n = name.getText().toString().trim();
        if (n.isEmpty() || validName(n)) saveProfile();
    }

    public void onClick(View v) {
        if (v == pick) {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            i.addCategory(Intent.CATEGORY_OPENABLE);
            i.setType("image/*");
            startActivityForResult(i, PICK);
        } else if (v == model) {
            slim = !slim;
            updateModel();
            showPreview();
            try {
                saveModel();
            } catch (Exception ignored) {
            }
        } else if (v == play) {
            if (!saveProfile()) return;
            // Left by an older build's "PLAY 2D": the game would start without VR.
            new java.io.File(getFilesDir(), "launch-2d").delete();
            Intent i = new Intent(this, HorizonActivity.class);
            i.addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT);
            startActivity(i);
            finish();
        } else if (v == driver) {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            i.addCategory(Intent.CATEGORY_OPENABLE);
            i.setType("*/*");
            startActivityForResult(i, PICK_DRIVER);
        } else if (v == systemDriver) {
            Turnip.useSystem(this);
            driverStatus.setText(Turnip.status(this));
            message.setTextColor(Ui.GREEN);
            message.setText(Ui.t("Системный драйвер. Перезапустите игру.", "System driver. Restart the game."));
        } else if (v == copy) {
            android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            if (cm != null) cm.setPrimaryClip(android.content.ClipData.newPlainText("code", code.getText()));
            copy.setText(Ui.t("Скопировано ✓", "Copied ✓"));
        } else if (v == browser) {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(verifyUri)));
            } catch (Exception e) {
                message.setText(Ui.t("Нет браузера. Откройте ", "No browser. Open ") + verifyUri);
            }
        } else if (v == advanced) {
            advancedBox.setVisibility(advancedBox.getVisibility() == View.VISIBLE ? View.GONE : View.VISIBLE);
        } else if (v == microsoft) {
            try {
                startActivityForResult(new Intent(this, MsaLoginActivity.class), MS_LOGIN);
            } catch (Exception e) {
                advancedBox.setVisibility(View.VISIBLE);
                message.setTextColor(Ui.WARN);
                message.setText(Ui.t("Нет WebView на телефоне: войдите по коду ниже.", "No WebView on this phone: sign in with a code below."));
            }
        } else if (v == deviceCode) {
            startDeviceCode();
        } else if (v == signOut) {
            Msa.cancel();
            Msa.signOut(this);
            MsaLoginActivity.forgetCookies();
            codeBox.setVisibility(View.GONE);
            microsoft.setEnabled(true);
            deviceCode.setEnabled(true);
            refreshAccount();
            message.setTextColor(Ui.GREEN);
            message.setText(Ui.t("Вышли из аккаунта.", "Signed out."));
        } else if (v == mods) {
            startActivity(new Intent(this, ModImportActivity.class));
        }
    }

    private final Msa.Listener listener = new Msa.Listener() {
        public void onCode(String userCode, String uri) {
            verifyUri = uri;
            code.setText(userCode);
            codeHint.setText(Ui.t("Откройте ", "Open ") + uri.replace("https://", "")
                    + Ui.t(", введите код и войдите. Этот экран сам поймёт, что вы вошли.",
                    ", enter the code and sign in. This screen notices by itself."));
            copy.setText(Ui.t("Скопировать код", "Copy code"));
            codeBox.setVisibility(View.VISIBLE);
            deviceCode.setText(Ui.t("Ждём вход…", "Waiting for sign-in…"));
            message.setText("");
        }

        public void onDone(String n) {
            codeBox.setVisibility(View.GONE);
            resetButtons();
            refreshAccount();
            message.setTextColor(Ui.GREEN);
            message.setText(Ui.t("Вход выполнен: ", "Signed in: ") + n);
        }

        public void onError(String error) {
            codeBox.setVisibility(View.GONE);
            resetButtons();
            message.setTextColor(Ui.WARN);
            message.setText(error);
        }
    };

    private void resetButtons() {
        microsoft.setText(Ui.t("Войти через Microsoft", "Sign in with Microsoft"));
        microsoft.setEnabled(true);
        deviceCode.setText(Ui.t("Войти по коду", "Sign in with a code"));
        deviceCode.setEnabled(true);
    }

    private void startDeviceCode() {
        String id = clientId.getText().toString().trim();
        if (!id.matches("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")) {
            message.setTextColor(Ui.WARN);
            message.setText(Ui.t("Вставьте свой Azure client ID.", "Paste your own Azure client ID."));
            return;
        }
        try {
            Msa.saveClientId(this, id);
        } catch (Exception e) {
            message.setText(Ui.t("Не удалось сохранить client ID.", "Could not save the client ID."));
            return;
        }
        message.setTextColor(Ui.WARN);
        message.setText(Ui.t("Запрашиваем код…", "Requesting a code…"));
        deviceCode.setEnabled(false);
        microsoft.setEnabled(false);
        Msa.start(this, id, listener);
    }

    private String cutoutMode() {
        try {
            return new String(Files.readAllBytes(new File(getFilesDir(), "cutout.txt").toPath()), StandardCharsets.UTF_8).trim();
        } catch (Exception e) {
            return "short";
        }
    }

    private String cutoutLabel() {
        String m = cutoutMode();
        if (m.equals("never")) return Ui.t("Вырез: не заходить под камеру", "Cutout: stay clear of the camera");
        if (m.equals("always")) return Ui.t("Вырез: всегда под камеру", "Cutout: draw under the camera");
        return Ui.t("Вырез: по краям экрана", "Cutout: short edges");
    }

    private String accountLine() {
        String n = Msa.signedIn(this);
        if (n.isEmpty()) return Ui.t("Не подключён", "Not connected");
        return Ui.t("Аккаунт: ", "Account: ") + n;
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == MS_LOGIN) {
            String c = data == null ? null : data.getStringExtra("code");
            if (result != RESULT_OK || c == null) {
                String err = data == null ? null : data.getStringExtra("error");
                if (err != null && !err.isEmpty()) {
                    message.setTextColor(Ui.WARN);
                    message.setText(Ui.t("Вход отменён: ", "Sign-in cancelled: ") + err);
                }
                return;
            }
            message.setTextColor(Ui.WARN);
            message.setText(Ui.t("Входим в Xbox и Minecraft…", "Signing in to Xbox and Minecraft…"));
            microsoft.setEnabled(false);
            microsoft.setText(Ui.t("Входим…", "Signing in…"));
            Msa.startWithCode(this, c, listener);
            return;
        }
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        if (request == PICK_DRIVER) {
            message.setTextColor(Ui.WARN);
            try {
                String n = Turnip.install(this, data.getData());
                driverStatus.setText(Turnip.status(this));
                message.setTextColor(Ui.GREEN);
                message.setText(Ui.t("Драйвер установлен: ", "Driver installed: ") + n
                        + Ui.t(". Полностью закройте игру и откройте снова.", ". Fully close the game and open it again."));
            } catch (Exception e) {
                message.setText(Ui.t("В архиве нет .so драйвера (нужен zip с libvulkan_freedreno.so).",
                        "The archive has no driver .so (a zip with libvulkan_freedreno.so is needed)."));
            }
            return;
        }
        if (request != PICK) return;
        message.setTextColor(Ui.WARN);
        try (InputStream in = getContentResolver().openInputStream(data.getData())) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
                if (out.size() > 4 << 20) throw new Exception("too big");
            }
            byte[] png = normalizeSkin(out.toByteArray());
            if (png == null) {
                message.setText(Ui.t("Нужен скин: PNG 64×64 или 64×32 (HD 128/256… тоже подойдёт).",
                        "A skin is needed: a 64×64 or 64×32 PNG (HD 128/256… works too)."));
                return;
            }
            write(new File(dir, "horizon-skin.png"), png);
            saveModel();
            message.setTextColor(Ui.GREEN);
            message.setText(Ui.t("Скин применён.", "Skin applied."));
            showPreview();
        } catch (Exception e) {
            message.setText(Ui.t("Не удалось прочитать файл.", "Could not read the file."));
        }
    }

    /**
     * The picked image as the plain PNG the game reads: 8-bit RGBA, 64x64 or 64x32 (an HD skin
     * is scaled down pixel for pixel), and a legacy skin's fully opaque hat made see-through
     * (vanilla's SkinTextureDownloader.doNotchTransparencyHack). Null when it is no skin.
     */
    static byte[] normalizeSkin(byte[] bytes) {
        BitmapFactory.Options o = new BitmapFactory.Options();
        o.inScaled = false;
        o.inPremultiplied = false;
        o.inPreferredConfig = Bitmap.Config.ARGB_8888;
        Bitmap b = BitmapFactory.decodeByteArray(bytes, 0, bytes.length, o);
        if (b == null) return null;
        int w = b.getWidth(), h = b.getHeight();
        if (w < 64 || w > 1024 || w % 64 != 0 || (h != w && h * 2 != w)) return null;
        if (b.getConfig() != Bitmap.Config.ARGB_8888) b = b.copy(Bitmap.Config.ARGB_8888, true);
        if (w != 64) b = Bitmap.createScaledBitmap(b, 64, h * 64 / w, false);
        int[] px = new int[64 * b.getHeight()];
        b.getPixels(px, 0, 64, 0, 0, 64, b.getHeight());
        if (b.getHeight() == 32) {
            boolean opaque = true;
            for (int y = 0; y < 16 && opaque; y++) {
                for (int x = 32; x < 64; x++) {
                    if ((px[y * 64 + x] >>> 24) < 128) {
                        opaque = false;
                        break;
                    }
                }
            }
            if (opaque) for (int y = 0; y < 16; y++) for (int x = 32; x < 64; x++) px[y * 64 + x] = 0;
        }
        Bitmap clean = Bitmap.createBitmap(64, b.getHeight(), Bitmap.Config.ARGB_8888);
        clean.setPremultiplied(false);
        clean.setPixels(px, 0, 64, 0, 0, 64, b.getHeight());
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        if (!clean.compress(Bitmap.CompressFormat.PNG, 100, out)) return null;
        return out.toByteArray();
    }

    private static void write(File f, byte[] bytes) throws Exception {
        File tmp = new File(f.getPath() + ".tmp");
        try (FileOutputStream out = new FileOutputStream(tmp)) {
            out.write(bytes);
        }
        if (!tmp.renameTo(f)) {
            f.delete();
            if (!tmp.renameTo(f)) throw new Exception("rename");
        }
    }

    private void showPreview() {
        File f = new File(dir, "horizon-skin.png");
        Bitmap skin = null;
        if (f.isFile()) {
            BitmapFactory.Options o = new BitmapFactory.Options();
            o.inScaled = false;
            o.inPreferredConfig = Bitmap.Config.ARGB_8888;
            skin = BitmapFactory.decodeFile(f.getPath(), o);
            if (skin != null && (skin.getWidth() != 64 || (skin.getHeight() != 64 && skin.getHeight() != 32))) {
                // An older file the game cannot use either: clean it like a new pick.
                try {
                    byte[] png = normalizeSkin(Files.readAllBytes(f.toPath()));
                    if (png != null) {
                        write(f, png);
                        skin = BitmapFactory.decodeByteArray(png, 0, png.length, o);
                    } else {
                        skin = null;
                    }
                } catch (Exception e) {
                    skin = null;
                }
            }
        }
        preview.set(skin, slim);
        if (homeSkin != null) homeSkin.set(skin, slim);
    }
}
