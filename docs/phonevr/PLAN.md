# Phone VR: план и точки интеграции

Наша цель — VR Horizon на обычном Android-телефоне: экран в Cardboard-очках показывает
SBS-стерео, голова управляется гироскопом телефона, Joy-Con работают как VR-контроллеры.

База — HorizonCraft-VR (уже форк Android-порта Horizon). В нём VR сделан через OpenXR
под Quest; телефонная сборка без OpenXR стартует плоской. Мы заменяем источник позы и
цель рендера, не трогая игровую логику.

## Что где (карта кода)

|Узел|Где сейчас|Что делаем|
|---|---|---|
|OpenXR-сессия, поза головы, свапчейны глаз|`crates/horizon-xr/src/lib.rs`|Новый крейт `horizon-phonevr`: та же роль, но экран телефона. Интерфейсы `horizon-xr` сохраняем.|
|Блит глаза в цель|`crates/horizon-xr/src/blit.rs`|Блит в экранную SBS-поверхность (левый глаз — левая половина, правый — правая).|
|Рендер с двумя глазами (`Globals::stereo_*`)|`crates/horizon-client-render/src/gfx.rs`|Переиспользуем: per-eye путь уже есть.|
|Single-pass stereo (multiview)|`crates/horizon-client-render/src/gfx/stereo.rs`|Не трогаем (опция `HORIZON_XR_STEREO`).|
|Контроллеры → геймпад|`crates/horizon-xr/src/pad.rs`|Источник: Joy-Con от Android (события gamepad'ов NativeActivity).|
|Ввод на Android (тач, клавиатура)|`crates/horizon-client/src/game/android_input.rs`|Добавляем канал ввода от Joy-Con после фазы A.|
|Точка включения VR|`crates/horizon-client/src/android_main.rs`|Флаг `HORIZON_PHONEVR=1`; читаем `horizon-vr.txt`.|
|Java-часть (setup, загрузка файлов игры)|`packaging/android/java/dev/horizonlab/horizon/`|Сервис сенсоров (`SensorManager`: rotation vector), позже — IMU Joy-Con.|
|Манифест|`packaging/android/AndroidManifest.xml`|Убрана Quest-специфика OpenXR; добавлены sensor/bluetooth features.|

## Фазы

### Фаза 0 — база (текущая)
- Манифест без OpenXR; собственный CI (`.github/workflows/android-apk.yml`) собирает
  запускаемый APK. Проверяем на телефоне: игра стартует плоской, файлы качаются в приложении.

### Фаза A — SBS + голова
- Готово: `crates/horizon-phonevr` — Android-сенсор (game) rotation vector через NDK
  `ASensor*` без JNI, поток событий на ALooper, `recenter()`, при `HORIZON_PHONEVR=1`
  стартует из `android_main` и раз в секунду пишет позу в лог (проверка по adb).
- Готово: рендер (`gfx.rs`: `render_phone`): вместо OpenXR-свапчейна каждый глаз рисуется
  в оффскрин-таргет (полширины окна) с камерой от `frame.yaw/pitch`, половины копируются
  в свапчейн окна бок о бок; голова в камеру — `horizon_phonevr::view_rotation()` через
  `xr_look` (только в игре, меню не вертится с телефоном). Включатель: `HORIZON_PHONEVR=1`
  или строка `phonevr=1` в `horizon-vr.txt`; четверть-поворот ландшафта правится
  `HORIZON_PHONEVR_TURN` (по умолчанию -1).
- Переключение: `HORIZON_PHONEVR=1`; иначе игра ведёт себя как плоская.

### Фаза B — Joy-Con как контроллеры
- Joy-Con парсятся Android'ом как отдельные HID-геймпады (Android 11+).
- События NativeActivity мапим в существующий геймпад-слой игры (по образцу `pad.rs`):
  стик — движение/камера, кнопки — удары/блоки/прыжок.

### Фаза C — руки 3DoF
- Гироскоп каждого Joy-Con → ориентация руки в мире (как «виртуальные трекеры» Driver4VR:
  поза фиксируется у плеча, ориентация вращает модель).
- Источник IMU: сначала сенсоры Android HID; если телефон не отдаёт IMU геймпада —
  служба чтения BLE HID в Java-части (позже).

### Фаза D — полировка
- Коррекция дисторсии линз (модель Brown, параметры Cardboard из QR-кода).
- HUD и меню как панели в мире (`Panel::Head`/`Panel::Room` из `horizon-xr`).
- Цель 72 Гц на средних телефонах, рескейл рендера (`RESOLUTION_SCALE`).

## Честные ограничения
- Трекинг 3DoF: рукам хватит гироскопов Joy-Con, позиционного трекинга нет.
- Vulkan обязателен (GLES-фолбэк для VR непригоден — нужны двухслойные цели).
- Horizon юридически неофициален; нужна купленная Java Edition.
