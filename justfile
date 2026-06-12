art-selftest-lib:
    cargo ndk -t arm64-v8a build -p frida-rust-java-bridge-art-selftest

apk-perform-test-lib:
    just art-selftest-lib

apk-perform-test-apk: apk-perform-test-lib
    #!/usr/bin/env bash
    set -euo pipefail
    sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
    if [[ -z "$sdk" ]]; then
        echo "ANDROID_HOME or ANDROID_SDK_ROOT must point at an Android SDK." >&2
        exit 1
    fi
    android_jar="$(find "$sdk/platforms" -maxdepth 2 -name android.jar | sort -V | tail -1)"
    if [[ -z "$android_jar" ]]; then
        echo "No android.jar found under $sdk/platforms." >&2
        exit 1
    fi
    build_dir="target/test-fixtures/apk-perform"
    apk_path="target/test-fixtures/apk-perform-test.apk"
    key="target/test-fixtures/apk-perform-debug.keystore"
    rm -rf "$build_dir" "$apk_path"
    mkdir -p "$build_dir/classes" "$build_dir/dex"
    mapfile -t sources < <(find tests/fixtures/apk/src -name '*.java' | sort)
    javac --release 8 -cp "$android_jar" -d "$build_dir/classes" "${sources[@]}"
    mapfile -t classes < <(find "$build_dir/classes" -name '*.class' | sort)
    d8 --min-api 26 --output "$build_dir/dex" "${classes[@]}"
    aapt2 link \
        -I "$android_jar" \
        --manifest tests/fixtures/apk/AndroidManifest.xml \
        --min-sdk-version 26 \
        --target-sdk-version 36 \
        -o "$build_dir/base.apk"
    cp "$build_dir/base.apk" "$build_dir/unsigned.apk"
    zip -q -X -j "$build_dir/unsigned.apk" "$build_dir/dex/classes.dex"
    mkdir -p "$build_dir/lib/arm64-v8a"
    cp target/aarch64-linux-android/debug/libfrida_rust_java_bridge_art_selftest.so "$build_dir/lib/arm64-v8a/libfrida_rust_java_bridge_art_selftest.so"
    (
        cd "$build_dir"
        zip -q -X unsigned.apk lib/arm64-v8a/libfrida_rust_java_bridge_art_selftest.so
    )
    if [[ ! -f "$key" ]]; then
        keytool -genkeypair \
            -keystore "$key" \
            -storepass android \
            -keypass android \
            -alias androiddebugkey \
            -keyalg RSA \
            -keysize 2048 \
            -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US" >/dev/null
    fi
    zipalign -f 4 "$build_dir/unsigned.apk" "$build_dir/aligned.apk"
    apksigner sign \
        --ks "$key" \
        --ks-pass pass:android \
        --key-pass pass:android \
        --out "$apk_path" \
        "$build_dir/aligned.apk"

apk-perform-test-build: apk-perform-test-apk

apk-perform-test-deploy device="": apk-perform-test-build
    #!/usr/bin/env bash
    set -euo pipefail
    package="frida.rust.java.bridge.performtest"
    apk_path="target/test-fixtures/apk-perform-test.apk"
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just apk-perform-test-deploy <serial>' or 'just apk-perform-test-deploy all'." >&2
            exit 1
        fi
    fi
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '==> Deploying APK perform test to %s: %s (%s), SDK %s\n' "$serial" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
        if ! install_output="$(adb -s "$serial" install -r -t "$apk_path" 2>&1)"; then
            if [[ "$install_output" == *"INSTALL_FAILED_UPDATE_INCOMPATIBLE"* ]]; then
                adb -s "$serial" uninstall "$package" >/dev/null || true
                adb -s "$serial" install -r -t "$apk_path"
            else
                printf '%s\n' "$install_output" >&2
                exit 1
            fi
        else
            printf '%s\n' "$install_output"
        fi
        adb -s "$serial" shell am force-stop "$package" || true
    done

apk-perform-test-run device="":
    #!/usr/bin/env bash
    set -euo pipefail
    package="frida.rust.java.bridge.performtest"
    authority="frida.rust.java.bridge.performtest.status"
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just apk-perform-test-run <serial>' or 'just apk-perform-test-run all'." >&2
            exit 1
        fi
    fi
    passed=()
    failed=()
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        label="$serial: ${model:-unknown} (${name:-unknown}), SDK ${sdk:-unknown}"
        printf '==> Running APK perform test on %s\n' "$label"
        adb -s "$serial" shell am force-stop "$package" || true
        if ! adb -s "$serial" shell pm clear "$package" >/dev/null 2>&1; then
            adb -s "$serial" shell content call --uri "content://$authority" --method reset >/dev/null 2>&1 || true
            adb -s "$serial" shell am force-stop "$package" || true
        fi
        agent="libfrida_rust_java_bridge_art_selftest.so=/data/data/$package/files/apk-perform-status.txt"
        set +e
        start_output="$(timeout 15s adb -s "$serial" shell am start -S --attach-agent-bind "$agent" -n "$package/.EarlyPerformActivity" 2>&1)"
        start_status="$?"
        set -e
        if [[ "$start_status" -ne 0 && "$start_status" -ne 124 ]]; then
            failed+=("$label [start failed: $start_output]")
            printf '==> APK perform test start failed on %s:\n%s\n' "$label" "$start_output" >&2
            continue
        fi
        if [[ "$start_status" -eq 124 ]]; then
            printf '==> APK perform test start command timed out on %s; polling status anyway\n' "$label" >&2
        fi
        status="missing"
        content_output=""
        for _ in {1..50}; do
            content_output="$(adb -s "$serial" shell content call --uri "content://$authority" --method status 2>&1 || true)"
            if [[ "$content_output" == *"status=ok"* ]]; then
                status="ok"
                break
            fi
            if [[ "$content_output" == *"status=error:"* ]]; then
                status="$content_output"
                break
            fi
            sleep 0.2
        done
        if [[ "$status" == "ok" ]]; then
            passed+=("$label")
        else
            failed+=("$label [status ${status//$'\n'/ } output ${content_output//$'\n'/ }]")
            printf '==> APK perform test failed on %s with status %s\n' "$label" "$status" >&2
            printf '%s\n' "$content_output" >&2
        fi
        adb -s "$serial" shell am force-stop "$package" || true
    done
    printf '\nAPK perform test summary:\n'
    if [[ "${#passed[@]}" -gt 0 ]]; then
        printf '  passed:\n'
        for result in "${passed[@]}"; do
            printf '    %s\n' "$result"
        done
    fi
    if [[ "${#failed[@]}" -gt 0 ]]; then
        printf '  failed:\n'
        for result in "${failed[@]}"; do
            printf '    %s\n' "$result"
        done
        exit 1
    fi

apk-perform-test device="":
    #!/usr/bin/env bash
    set -euo pipefail
    just apk-perform-test-deploy '{{ device }}'
    just apk-perform-test-run '{{ device }}'

apk-perform-test-all:
    just apk-perform-test all

check:
    cargo ndk -t arm64-v8a clippy -p frida-rust-java-bridge --all-features
    cargo ndk -t arm64-v8a clippy -p frida-rust-java-bridge-art-selftest
    cargo ndk -t arm64-v8a build --examples

host-test:
    cargo test --lib

test-suite device="":
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    just check
    just host-test
    just unit-test "$device"
    just app-test "$device"
    just apk-perform-test "$device"
    just art-test "$device"

test-suite-all:
    just test-suite all

unit-test-build:
    cargo ndk -t arm64-v8a test --lib --no-run

unit-test-run device="":
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just unit-test-run <serial>' or 'just unit-test-run all'." >&2
            exit 1
        fi
    fi
    passed=()
    failed=()
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        label="$serial: ${model:-unknown} (${name:-unknown}), SDK ${sdk:-unknown}"
        printf '==> Running Android unit tests on %s\n' "$label"
        if cargo ndk -t arm64-v8a test --lib -- --adb-serial "$serial"; then
            passed+=("$label")
        else
            status="$?"
            failed+=("$label [exit $status]")
            printf '==> Android unit tests failed on %s with exit %s\n' "$label" "$status" >&2
        fi
    done
    printf '\nAndroid unit test summary:\n'
    if [[ "${#passed[@]}" -gt 0 ]]; then
        printf '  passed:\n'
        for result in "${passed[@]}"; do
            printf '    %s\n' "$result"
        done
    fi
    if [[ "${#failed[@]}" -gt 0 ]]; then
        printf '  failed:\n'
        for result in "${failed[@]}"; do
            printf '    %s\n' "$result"
        done
        exit 1
    fi

unit-test device="":
    just unit-test-run '{{ device }}'

unit-test-all:
    just unit-test all

build:
    cargo ndk -t arm64-v8a build

build-release:
    cargo ndk -t arm64-v8a build --release

test-fixture-dex:
    mkdir -p target/test-fixtures/build/classes target/test-fixtures/dex
    javac --release 8 -d target/test-fixtures/build/classes tests/fixtures/app-process/src/frida/rust/java/bridge/test/DexTestSubject.java
    d8 --min-api 26 --output target/test-fixtures/dex target/test-fixtures/build/classes/frida/rust/java/bridge/test/DexTestSubject.class

art-test-build:
    rm -f target/aarch64-linux-android/debug/deps/art_bootstrap-*
    cargo ndk -t arm64-v8a build --test art_bootstrap

app-process-test-dex: test-fixture-dex
    rm -rf target/test-fixtures/build/app-process target/test-fixtures/build/app-process-dex target/test-fixtures/app-process-test.jar
    mkdir -p target/test-fixtures/build/app-process target/test-fixtures/build/app-process-dex
    javac --release 8 -d target/test-fixtures/build/app-process tests/fixtures/app-process/src/frida/rust/java/bridge/test/TestSubjectBase.java tests/fixtures/app-process/src/frida/rust/java/bridge/test/TestSubject.java tests/fixtures/app-process/src/frida/rust/java/bridge/test/MisleadingClassLoader.java tests/fixtures/app-process/src/frida/rust/java/bridge/test/AppProcessTest.java
    d8 --min-api 26 --output target/test-fixtures/build/app-process-dex target/test-fixtures/build/app-process/frida/rust/java/bridge/test/TestSubjectBase.class target/test-fixtures/build/app-process/frida/rust/java/bridge/test/TestSubject.class target/test-fixtures/build/app-process/frida/rust/java/bridge/test/MisleadingClassLoader.class target/test-fixtures/build/app-process/frida/rust/java/bridge/test/AppProcessTest.class
    jar cf target/test-fixtures/app-process-test.jar -C target/test-fixtures/build/app-process-dex classes.dex

app-test-build: app-process-test-dex art-selftest-lib
    @true

devices:
    #!/usr/bin/env bash
    set -euo pipefail
    mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
    for device in "${devices[@]}"; do
        model="$(adb -s "$device" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$device" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$device" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '%s\t%s (%s)\tSDK %s\n' "$device" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
    done

art-test-deploy device="": art-test-build
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just art-test-deploy <serial>' or 'just art-test-deploy all'." >&2
            exit 1
        fi
    fi
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '==> Deploying native ART test to %s: %s (%s), SDK %s\n' "$serial" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-rust-java-bridge
        test_binary="$(find target/aarch64-linux-android/debug/deps -maxdepth 1 -type f -name 'art_bootstrap-*' -perm -111 | sort | tail -1)"
        if [[ -z "$test_binary" ]]; then
            echo "Built art_bootstrap test binary was not found." >&2
            exit 1
        fi
        adb -s "$serial" push "$test_binary" /data/local/tmp/frida-rust-java-bridge/art_bootstrap
        adb -s "$serial" shell chmod 755 /data/local/tmp/frida-rust-java-bridge/art_bootstrap
    done

art-test-run device="":
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just art-test-run <serial>' or 'just art-test-run all'." >&2
            exit 1
        fi
    fi
    passed=()
    failed=()
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        label="$serial: ${model:-unknown} (${name:-unknown}), SDK ${sdk:-unknown}"
        printf '==> Running native ART test on %s\n' "$label"
        if adb -s "$serial" shell "LD_PRELOAD=libart.so LD_LIBRARY_PATH=/apex/com.android.art/lib64:/apex/com.android.runtime/lib64 /data/local/tmp/frida-rust-java-bridge/art_bootstrap"; then
            passed+=("$label")
        else
            status="$?"
            failed+=("$label [exit $status]")
            printf '==> native ART test failed on %s with exit %s\n' "$label" "$status" >&2
        fi
    done
    printf '\nnative ART test summary:\n'
    if [[ "${#passed[@]}" -gt 0 ]]; then
        printf '  passed:\n'
        for result in "${passed[@]}"; do
            printf '    %s\n' "$result"
        done
    fi
    if [[ "${#failed[@]}" -gt 0 ]]; then
        printf '  failed:\n'
        for result in "${failed[@]}"; do
            printf '    %s\n' "$result"
        done
        exit 1
    fi

art-test device="":
    #!/usr/bin/env bash
    set -euo pipefail
    just art-test-deploy '{{ device }}'
    just art-test-run '{{ device }}'

art-test-all:
    just art-test all

app-test-deploy device="": app-test-build
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just app-test-deploy <serial>' or 'just app-test-deploy all'." >&2
            exit 1
        fi
    fi
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '==> Deploying app_process test to %s: %s (%s), SDK %s\n' "$serial" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-rust-java-bridge
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-rust-java-bridge/dex-cache
        adb -s "$serial" push target/aarch64-linux-android/debug/libfrida_rust_java_bridge_art_selftest.so /data/local/tmp/frida-rust-java-bridge/libfrida_rust_java_bridge_art_selftest.so
        adb -s "$serial" push target/test-fixtures/app-process-test.jar /data/local/tmp/frida-rust-java-bridge/app-process-test.jar
        adb -s "$serial" push target/test-fixtures/dex/classes.dex /data/local/tmp/frida-rust-java-bridge/dex-test-fixture.dex
    done

app-test-run device="":
    #!/usr/bin/env bash
    set -euo pipefail
    device='{{ device }}'
    if [[ "$device" == "all" ]]; then
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
    elif [[ -n "$device" ]]; then
        devices=("$device")
    else
        mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')
        if [[ "${#devices[@]}" -eq 0 ]]; then
            echo "No connected adb devices found." >&2
            exit 1
        fi
        if [[ "${#devices[@]}" -gt 1 ]]; then
            echo "Multiple adb devices connected. Run 'just app-test-run <serial>' or 'just app-test-run all'." >&2
            exit 1
        fi
    fi
    passed=()
    failed=()
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        label="$serial: ${model:-unknown} (${name:-unknown}), SDK ${sdk:-unknown}"
        printf '==> Running app_process test on %s\n' "$label"
        if adb -s "$serial" shell "CLASSPATH=/data/local/tmp/frida-rust-java-bridge/app-process-test.jar app_process /system/bin frida.rust.java.bridge.test.AppProcessTest"; then
            passed+=("$label")
        else
            status="$?"
            failed+=("$label [exit $status]")
            printf '==> app_process test failed on %s with exit %s\n' "$label" "$status" >&2
        fi
    done
    printf '\napp_process test summary:\n'
    if [[ "${#passed[@]}" -gt 0 ]]; then
        printf '  passed:\n'
        for result in "${passed[@]}"; do
            printf '    %s\n' "$result"
        done
    fi
    if [[ "${#failed[@]}" -gt 0 ]]; then
        printf '  failed:\n'
        for result in "${failed[@]}"; do
            printf '    %s\n' "$result"
        done
        exit 1
    fi

app-test device="":
    #!/usr/bin/env bash
    set -euo pipefail
    just app-test-deploy '{{ device }}'
    just app-test-run '{{ device }}'

app-test-all:
    just app-test all
