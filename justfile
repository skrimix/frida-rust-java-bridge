check:
    cargo ndk -t arm64-v8a clippy

test-build:
    cargo ndk -t arm64-v8a test --no-run

build:
    cargo ndk -t arm64-v8a build

build-release:
    cargo ndk -t arm64-v8a build --release

smoke-fixture-dex:
    mkdir -p smoke-fixtures/build/classes smoke-fixtures/dex
    javac --release 8 -d smoke-fixtures/build/classes smoke-fixtures/src/frida/java/bridge/rs/smoke/SmokeSubject.java
    d8 --min-api 26 --output smoke-fixtures/dex smoke-fixtures/build/classes/frida/java/bridge/rs/smoke/SmokeSubject.class

smoke-build: smoke-fixture-dex
    cargo ndk -t arm64-v8a build --bin art_smoke

smoke-deploy: smoke-build
    adb shell mkdir -p /data/local/tmp/frida-java-bridge-rs
    adb push target/aarch64-linux-android/debug/art_smoke /data/local/tmp/frida-java-bridge-rs/art_smoke
    adb shell chmod 755 /data/local/tmp/frida-java-bridge-rs/art_smoke

smoke-run:
    adb shell "LD_PRELOAD=libart.so LD_LIBRARY_PATH=/apex/com.android.runtime/lib64:/apex/com.android.art/lib64 /data/local/tmp/frida-java-bridge-rs/art_smoke"

smoke: smoke-deploy smoke-run
