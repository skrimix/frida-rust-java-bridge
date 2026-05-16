check:
    cargo ndk -t arm64-v8a clippy --features app-process-test

host-test-build:
    cargo ndk -t arm64-v8a test --no-run

build:
    cargo ndk -t arm64-v8a build

build-release:
    cargo ndk -t arm64-v8a build --release

test-fixture-dex:
    mkdir -p test-fixtures/build/classes test-fixtures/dex
    javac --release 8 -d test-fixtures/build/classes test-fixtures/src/frida/java/bridge/rs/test/DexTestSubject.java
    d8 --min-api 26 --output test-fixtures/dex test-fixtures/build/classes/frida/java/bridge/rs/test/DexTestSubject.class

art-test-build:
    cargo ndk -t arm64-v8a build --bin art_test

app-process-test-dex: test-fixture-dex
    rm -rf test-fixtures/build/app-process test-fixtures/build/app-process-dex test-fixtures/app-process-test.jar
    mkdir -p test-fixtures/build/app-process test-fixtures/build/app-process-dex
    javac --release 8 -d test-fixtures/build/app-process test-fixtures/src/frida/java/bridge/rs/test/TestSubject.java test-fixtures/src/frida/java/bridge/rs/test/AppProcessTest.java
    d8 --min-api 26 --output test-fixtures/build/app-process-dex test-fixtures/build/app-process/frida/java/bridge/rs/test/TestSubject.class test-fixtures/build/app-process/frida/java/bridge/rs/test/AppProcessTest.class
    jar cf test-fixtures/app-process-test.jar -C test-fixtures/build/app-process-dex classes.dex

app-process-test-build: app-process-test-dex
    cargo ndk -t arm64-v8a build --features app-process-test --lib

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
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-java-bridge-rs
        adb -s "$serial" push target/aarch64-linux-android/debug/art_test /data/local/tmp/frida-java-bridge-rs/art_test
        adb -s "$serial" shell chmod 755 /data/local/tmp/frida-java-bridge-rs/art_test
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
        if adb -s "$serial" shell "LD_PRELOAD=libart.so LD_LIBRARY_PATH=/apex/com.android.art/lib64:/apex/com.android.runtime/lib64 /data/local/tmp/frida-java-bridge-rs/art_test"; then
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

app-test-deploy device="": app-process-test-build
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
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-java-bridge-rs
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-java-bridge-rs/dex-cache
        adb -s "$serial" push target/aarch64-linux-android/debug/libfrida_java_bridge_rs.so /data/local/tmp/frida-java-bridge-rs/libfrida_java_bridge_rs.so
        adb -s "$serial" push test-fixtures/app-process-test.jar /data/local/tmp/frida-java-bridge-rs/app-process-test.jar
        adb -s "$serial" push test-fixtures/dex/classes.dex /data/local/tmp/frida-java-bridge-rs/dex-test-fixture.dex
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
        if adb -s "$serial" shell "CLASSPATH=/data/local/tmp/frida-java-bridge-rs/app-process-test.jar app_process /system/bin frida.java.bridge.rs.test.AppProcessTest"; then
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

test-build:
    just app-process-test-build

test-deploy device="":
    just app-test-deploy '{{ device }}'

test-run device="":
    just app-test-run '{{ device }}'

test device="":
    just app-test '{{ device }}'

test-all:
    just app-test-all
