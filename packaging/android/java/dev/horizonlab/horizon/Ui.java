package dev.horizonlab.horizon;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.LinearGradient;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RadialGradient;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.Shader;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.graphics.drawable.StateListDrawable;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.util.Locale;

/**
 * Shared look of HorizonCraft VR's setup and launcher screens: a dark, modern theme with one
 * green accent, rounded cards and buttons, line icons and a night landscape, all drawn as
 * vectors at the panel's full resolution (no scaled-up bitmaps, so it stays sharp in the
 * headset). Everything is drawn here: no game art.
 */
final class Ui {
    static final int BG = Color.rgb(13, 16, 22);
    /** Cards and panels, a little see-through over the landscape. */
    static final int CARD = Color.argb(236, 22, 27, 35);
    /** Fields, secondary buttons, the selected menu entry. */
    static final int STONE = Color.rgb(33, 40, 51);
    /** Outlines. */
    static final int EDGE = Color.rgb(48, 58, 72);
    /** The accent: Play, the selected entry's marker, what is on. */
    static final int GREEN = Color.rgb(46, 175, 92);
    static final int GREEN_DARK = Color.rgb(30, 120, 64);
    static final int TEXT = Color.rgb(236, 240, 245);
    static final int MUTED = Color.rgb(146, 156, 170);
    static final int WARN = Color.rgb(242, 184, 64);
    static final int SHADOW = Color.rgb(0, 0, 0);

    private Ui() {}

    static boolean ru() {
        return "ru".equals(Locale.getDefault().getLanguage());
    }

    static String t(String ru, String en) {
        return ru() ? ru : en;
    }

    static int dp(Context c, float v) {
        return (int) (v * c.getResources().getDisplayMetrics().density + 0.5f);
    }

    static int shade(int color, float by) {
        return Color.argb(Color.alpha(color),
                Math.max(0, Math.min(255, (int) (Color.red(color) * by))),
                Math.max(0, Math.min(255, (int) (Color.green(color) * by))),
                Math.max(0, Math.min(255, (int) (Color.blue(color) * by))));
    }

    static int alpha(int color, int a) {
        return Color.argb(a, Color.red(color), Color.green(color), Color.blue(color));
    }

    /** A rounded panel: `fill`, an outline when `stroke` is not 0, corners of `radius` dp. */
    static Drawable rounded(Context c, int fill, int stroke, float radius) {
        GradientDrawable d = new GradientDrawable();
        d.setColor(fill);
        d.setCornerRadius(dp(c, radius));
        if (stroke != 0) d.setStroke(Math.max(1, dp(c, 1)), stroke);
        return d;
    }

    /** A pressable background: `fill` with a ripple, lighter under the pointer. */
    static Drawable pressable(Context c, int fill, int stroke, float radius) {
        StateListDrawable states = new StateListDrawable();
        states.addState(new int[] {-android.R.attr.state_enabled}, rounded(c, shade(fill, 0.6f), stroke == 0 ? 0 : shade(stroke, 0.7f), radius));
        states.addState(new int[] {android.R.attr.state_hovered}, rounded(c, shade(fill, 1.25f), stroke == 0 ? 0 : TEXT, radius));
        states.addState(new int[] {android.R.attr.state_focused}, rounded(c, shade(fill, 1.2f), stroke == 0 ? 0 : TEXT, radius));
        states.addState(new int[] {}, rounded(c, fill, stroke, radius));
        return new RippleDrawable(ColorStateList.valueOf(Color.argb(60, 255, 255, 255)), states, rounded(c, Color.WHITE, 0, radius));
    }

    /**
     * The night landscape behind the launcher, drawn at whatever size it is shown: a deep
     * blue sky with a green glow at the horizon, stars, and three ranges of stepped, blocky
     * hills fading into the distance.
     */
    static Drawable sunset(Context c) {
        return new Drawable() {
            private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            private final Path path = new Path();

            @Override
            public void draw(Canvas canvas) {
                Rect b = getBounds();
                float w = b.width(), h = b.height();
                if (w <= 0 || h <= 0) return;
                canvas.save();
                canvas.translate(b.left, b.top);
                paint.setShader(new LinearGradient(0, 0, 0, h,
                        new int[] {Color.rgb(8, 11, 22), Color.rgb(14, 24, 44), Color.rgb(22, 52, 58)},
                        new float[] {0f, 0.55f, 1f}, Shader.TileMode.CLAMP));
                canvas.drawRect(0, 0, w, h, paint);
                // The glow over the hills.
                paint.setShader(new RadialGradient(w * 0.68f, h * 0.78f, Math.max(w, h) * 0.55f,
                        Color.argb(110, 46, 175, 92), Color.argb(0, 46, 175, 92), Shader.TileMode.CLAMP));
                canvas.drawRect(0, 0, w, h, paint);
                paint.setShader(null);
                java.util.Random random = new java.util.Random(26);
                for (int i = 0; i < 140; i++) {
                    float x = random.nextFloat() * w, y = random.nextFloat() * h * 0.6f;
                    float r = (random.nextInt(5) == 0 ? 1.6f : 0.9f) * c.getResources().getDisplayMetrics().density;
                    paint.setColor(Color.argb(90 + random.nextInt(140), 255, 255, 255));
                    canvas.drawCircle(x, y, r, paint);
                }
                // A big, soft moon.
                float moon = h * 0.07f;
                paint.setColor(Color.argb(40, 220, 240, 255));
                canvas.drawCircle(w * 0.2f, h * 0.22f, moon * 1.9f, paint);
                paint.setColor(Color.rgb(226, 236, 242));
                canvas.drawCircle(w * 0.2f, h * 0.22f, moon, paint);
                // Stepped hills: far, middle, near.
                int[] colors = {Color.rgb(20, 40, 52), Color.rgb(14, 28, 36), Color.rgb(9, 16, 22)};
                float[] base = {0.70f, 0.80f, 0.90f};
                float step = Math.max(8f, w / 48f);
                for (int layer = 0; layer < 3; layer++) {
                    java.util.Random hills = new java.util.Random(11 + layer * 7);
                    path.reset();
                    path.moveTo(0, h);
                    float y = h * base[layer];
                    for (float x = 0; x < w + step; x += step) {
                        y = Math.max(h * (base[layer] - 0.1f), Math.min(h * (base[layer] + 0.05f), y + (hills.nextInt(3) - 1) * step * 0.5f));
                        path.lineTo(x, y);
                        path.lineTo(x + step, y);
                    }
                    path.lineTo(w, h);
                    path.close();
                    paint.setColor(colors[layer]);
                    canvas.drawPath(path, paint);
                }
                canvas.restore();
            }

            @Override public void setAlpha(int alpha) {}
            @Override public void setColorFilter(android.graphics.ColorFilter f) {}
            @Override public int getOpacity() { return android.graphics.PixelFormat.OPAQUE; }
        };
    }

    /** The launcher's line icons, drawn at any size. */
    enum Glyph { HOME, SKIN, SETTINGS, CONTROLS, ACCOUNT, UPDATES, MODS, PLAY }

    /** A line icon `size` dp across, in `color`. */
    static Drawable icon(Context c, final Glyph glyph, final int color, float size) {
        final int px = dp(c, size);
        Drawable d = new Drawable() {
            @Override
            public void draw(Canvas canvas) {
                Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
                p.setColor(color);
                p.setStyle(Paint.Style.STROKE);
                p.setStrokeCap(Paint.Cap.ROUND);
                p.setStrokeJoin(Paint.Join.ROUND);
                Rect b = getBounds();
                float u = b.width() / 24f;
                canvas.save();
                canvas.translate(b.left, b.top);
                p.setStrokeWidth(2f * u);
                Path path = new Path();
                switch (glyph) {
                    case HOME:
                        path.moveTo(3 * u, 11 * u);
                        path.lineTo(12 * u, 3.5f * u);
                        path.lineTo(21 * u, 11 * u);
                        path.moveTo(5.5f * u, 9.5f * u);
                        path.lineTo(5.5f * u, 20 * u);
                        path.lineTo(18.5f * u, 20 * u);
                        path.lineTo(18.5f * u, 9.5f * u);
                        path.moveTo(10 * u, 20 * u);
                        path.lineTo(10 * u, 14 * u);
                        path.lineTo(14 * u, 14 * u);
                        path.lineTo(14 * u, 20 * u);
                        break;
                    case SKIN:
                        // A coat hanger.
                        path.moveTo(12 * u, 10 * u);
                        path.lineTo(12 * u, 8.5f * u);
                        path.arcTo(new RectF(9.5f * u, 3 * u, 14.5f * u, 8 * u), 90, -270);
                        path.moveTo(12 * u, 10 * u);
                        path.lineTo(2.5f * u, 18 * u);
                        path.lineTo(21.5f * u, 18 * u);
                        path.close();
                        break;
                    case SETTINGS:
                        // Three sliders.
                        for (int i = 0; i < 3; i++) {
                            float y = (6 + 6 * i) * u, knob = (i == 1 ? 8 : i == 0 ? 15 : 12) * u;
                            path.moveTo(3 * u, y);
                            path.lineTo(knob - 2.5f * u, y);
                            path.moveTo(knob + 2.5f * u, y);
                            path.lineTo(21 * u, y);
                            path.addCircle(knob, y, 2.5f * u, Path.Direction.CW);
                        }
                        break;
                    case CONTROLS:
                        // A Touch controller: the ring and the grip.
                        path.addCircle(12 * u, 8.5f * u, 5.5f * u, Path.Direction.CW);
                        path.addCircle(12 * u, 8.5f * u, 1.6f * u, Path.Direction.CW);
                        path.moveTo(9.5f * u, 13.5f * u);
                        path.lineTo(10 * u, 21 * u);
                        path.lineTo(14 * u, 21 * u);
                        path.lineTo(14.5f * u, 13.5f * u);
                        break;
                    case ACCOUNT:
                        path.addCircle(12 * u, 8 * u, 4 * u, Path.Direction.CW);
                        path.moveTo(4 * u, 21 * u);
                        path.cubicTo(4 * u, 15 * u, 20 * u, 15 * u, 20 * u, 21 * u);
                        break;
                    case UPDATES:
                        path.addArc(new RectF(4 * u, 4 * u, 20 * u, 20 * u), -60, 300);
                        path.moveTo(20.5f * u, 3.5f * u);
                        path.lineTo(20 * u, 8.5f * u);
                        path.lineTo(15 * u, 8 * u);
                        break;
                    case MODS:
                        // A cube.
                        path.moveTo(12 * u, 2.5f * u);
                        path.lineTo(20.5f * u, 7 * u);
                        path.lineTo(20.5f * u, 17 * u);
                        path.lineTo(12 * u, 21.5f * u);
                        path.lineTo(3.5f * u, 17 * u);
                        path.lineTo(3.5f * u, 7 * u);
                        path.close();
                        path.moveTo(3.5f * u, 7 * u);
                        path.lineTo(12 * u, 11.5f * u);
                        path.lineTo(20.5f * u, 7 * u);
                        path.moveTo(12 * u, 11.5f * u);
                        path.lineTo(12 * u, 21.5f * u);
                        break;
                    case PLAY:
                        p.setStyle(Paint.Style.FILL);
                        path.moveTo(7 * u, 4 * u);
                        path.lineTo(20 * u, 12 * u);
                        path.lineTo(7 * u, 20 * u);
                        path.close();
                        break;
                }
                canvas.drawPath(path, p);
                canvas.restore();
            }

            @Override public int getIntrinsicWidth() { return px; }
            @Override public int getIntrinsicHeight() { return px; }
            @Override public void setAlpha(int alpha) {}
            @Override public void setColorFilter(android.graphics.ColorFilter f) {}
            @Override public int getOpacity() { return android.graphics.PixelFormat.TRANSLUCENT; }
        };
        d.setBounds(0, 0, px, px);
        return d;
    }

    /** A full-screen background with a centered card; returns the card to fill. */
    static LinearLayout card(android.app.Activity a) {
        FrameLayout root = new FrameLayout(a);
        root.setBackground(sunset(a));
        android.widget.ScrollView scroll = new android.widget.ScrollView(a);
        scroll.setFillViewport(true);
        FrameLayout center = new FrameLayout(a);
        LinearLayout card = new LinearLayout(a);
        card.setOrientation(LinearLayout.VERTICAL);
        int p = dp(a, 26);
        card.setPadding(p, p, p, p);
        card.setBackground(rounded(a, CARD, EDGE, 20));
        card.setElevation(dp(a, 8));
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                Math.min(dp(a, 560), a.getResources().getDisplayMetrics().widthPixels - dp(a, 32)),
                FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.CENTER);
        lp.setMargins(0, dp(a, 16), 0, dp(a, 16));
        center.addView(card, lp);
        scroll.addView(center);
        root.addView(scroll);
        a.setContentView(root);
        return card;
    }

    static TextView text(Context c, String s, float size, int color, boolean bold) {
        TextView t = new TextView(c);
        t.setText(s);
        t.setTextSize(size);
        t.setTextColor(color);
        t.setTypeface(bold ? Typeface.create("sans-serif-medium", Typeface.NORMAL) : Typeface.create("sans-serif", Typeface.NORMAL));
        t.setLineSpacing(0, 1.2f);
        t.setPadding(0, dp(c, 3), 0, dp(c, 3));
        return t;
    }

    static Button button(Context c, String s, boolean primary) {
        Button b = new Button(c);
        b.setText(s);
        b.setAllCaps(false);
        b.setTextSize(primary ? 17 : 15);
        b.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        b.setTextColor(primary ? Color.WHITE : TEXT);
        b.setBackground(primary ? pressable(c, GREEN, 0, 12) : pressable(c, STONE, EDGE, 12));
        b.setMinHeight(dp(c, primary ? 52 : 44));
        b.setPadding(dp(c, 16), dp(c, 8), dp(c, 16), dp(c, 8));
        b.setStateListAnimator(null);
        return b;
    }

    static LinearLayout.LayoutParams gap(Context c, int top) {
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        lp.topMargin = dp(c, top);
        return lp;
    }

    /** A section header: a small accent caption over a thin line (empty title: the line only). */
    static void section(LinearLayout card, String title) {
        Context c = card.getContext();
        if (!title.isEmpty()) {
            TextView t = text(c, title.toUpperCase(Locale.getDefault()), 12, GREEN, true);
            t.setLetterSpacing(0.1f);
            LinearLayout.LayoutParams tp = gap(c, card.getChildCount() == 0 ? 0 : 20);
            card.addView(t, tp);
        }
        View line = new View(c);
        line.setBackgroundColor(EDGE);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, Math.max(1, dp(c, 1)));
        lp.topMargin = dp(c, title.isEmpty() ? 18 : 4);
        lp.bottomMargin = dp(c, 10);
        card.addView(line, lp);
    }

    /** A rounded progress bar, 0..1000. */
    static final class Bar extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final RectF r = new RectF();
        private int value;

        Bar(Context c) {
            super(c);
            setMinimumHeight(dp(c, 10));
        }

        void set(int permille) {
            value = Math.max(0, Math.min(1000, permille));
            invalidate();
        }

        @Override
        protected void onMeasure(int w, int h) {
            setMeasuredDimension(MeasureSpec.getSize(w), dp(getContext(), 10));
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float radius = getHeight() / 2f;
            paint.setColor(STONE);
            r.set(0, 0, getWidth(), getHeight());
            canvas.drawRoundRect(r, radius, radius, paint);
            if (value > 0) {
                r.set(0, 0, Math.max(getHeight(), getWidth() * value / 1000f), getHeight());
                paint.setColor(GREEN);
                canvas.drawRoundRect(r, radius, radius, paint);
            }
        }
    }
}
