# Vrminefon — Minecraft Java VR на телефоне

Нативный VR-клиент Minecraft: Java Edition 26.3, работающий **целиком на Android-телефоне**
(без ПК и без Quest): SBS-стерео на экране телефона в Cardboard-очках, поворот головы по
гироскопу телефона, Nintendo Switch Joy-Con в роли VR-контроллеров (по схеме, похожей на
Driver4VR, — «виртуальные трекеры»).

Проект форкнут с сохранением лицензий и атрибуции авторов:

- [Horizon](https://github.com/horizon-rust/horizon) — Minecraft Java 26.3, переписанный на Rust (авторы Horizon, MIT).
- [HorizonCraft-VR](https://github.com/Boboetto/HorizonCraft-VR) — VR-слой для Quest 3, по образцу
  которого строится наш телефонный VR (адаптация: экран телефона + сенсоры вместо OpenXR).
- Android-порт Horizon (Rayan-XOO/Starling) — NativeActivity, Vulkan, загрузка файлов игры в приложении.

Это неофициальный фанатский проект, не связанный с Mojang и Microsoft. Нужна своя купленная
копия Minecraft: Java Edition — APK не содержит файлов Minecraft.

## Статус

| Фаза | Что | Статус |
| --- | --- | --- |
| 0 | База: форк собирается, плоский APK на телефоне | готовится (CI: android-apk.yml) |
| A | SBS-стерео + голова по гироскопу телефона | в плане |
| B | Joy-Con как контроллеры (движение, удары, блоки) | в плане |
| C | Руки в VR по гироскопам Joy-Con (3DoF, стиль Driver4VR) | в плане |
| D | Коррекция линз по Cardboard QR, панели меню в мире | в плане |

Дизайн и точки интеграции: [docs/phonevr/PLAN.md](docs/phonevr/PLAN.md).
Сборка APK локально: [ANDROID.md](ANDROID.md). CI собирает APK и выкладывает его артефактом.

## Сборка APK локально (Linux)

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk
# Android SDK: cmdline-tools, затем
sdkmanager "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018" "platform-tools"
export ANDROID_HOME=~/android/sdk
PROFILE=release ./packaging/android/build_apk.sh   # -> target/android/horizon.apk
```

APK подписан debug-ключом; ставится через `adb install -r` или открытием файла на телефоне
(нужны Android 8.0+, arm64, желательно Android 11+ для Joy-Con по Bluetooth).
