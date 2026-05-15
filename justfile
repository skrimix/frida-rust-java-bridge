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

app-process-smoke-dex:
    rm -rf smoke-fixtures/build/app-process smoke-fixtures/build/app-process-dex smoke-fixtures/app-process-smoke.jar
    mkdir -p smoke-fixtures/build/app-process smoke-fixtures/build/app-process-dex
    javac --release 8 -d smoke-fixtures/build/app-process smoke-fixtures/src/frida/java/bridge/rs/smoke/SmokeSubject.java smoke-fixtures/src/frida/java/bridge/rs/smoke/AppProcessSmoke.java
    d8 --min-api 26 --output smoke-fixtures/build/app-process-dex smoke-fixtures/build/app-process/frida/java/bridge/rs/smoke/SmokeSubject.class smoke-fixtures/build/app-process/frida/java/bridge/rs/smoke/AppProcessSmoke.class
    jar cf smoke-fixtures/app-process-smoke.jar -C smoke-fixtures/build/app-process-dex classes.dex

app-process-smoke-build: app-process-smoke-dex
    cargo ndk -t arm64-v8a build --features app-process-smoke --lib

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

smoke-deploy device="": smoke-build
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
            echo "Multiple adb devices connected. Run 'just smoke-deploy <serial>' or 'just smoke-deploy all'." >&2
            exit 1
        fi
    fi
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '==> Deploying to %s: %s (%s), SDK %s\n' "$serial" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-java-bridge-rs
        adb -s "$serial" push target/aarch64-linux-android/debug/art_smoke /data/local/tmp/frida-java-bridge-rs/art_smoke
        adb -s "$serial" shell chmod 755 /data/local/tmp/frida-java-bridge-rs/art_smoke
    done

smoke-run device="":
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
            echo "Multiple adb devices connected. Run 'just smoke-run <serial>' or 'just smoke-run all'." >&2
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
        printf '==> Running smoke on %s\n' "$label"
        if adb -s "$serial" shell "LD_PRELOAD=libart.so LD_LIBRARY_PATH=/apex/com.android.art/lib64:/apex/com.android.runtime/lib64 /data/local/tmp/frida-java-bridge-rs/art_smoke"; then
            passed+=("$label")
        else
            status="$?"
            failed+=("$label [exit $status]")
            printf '==> Smoke failed on %s with exit %s\n' "$label" "$status" >&2
        fi
    done
    printf '\nSmoke summary:\n'
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

smoke device="":
    #!/usr/bin/env bash
    set -euo pipefail
    just smoke-deploy '{{ device }}'
    just smoke-run '{{ device }}'

smoke-all:
    just smoke all

app-smoke-deploy device="": app-process-smoke-build
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
            echo "Multiple adb devices connected. Run 'just app-smoke-deploy <serial>' or 'just app-smoke-deploy all'." >&2
            exit 1
        fi
    fi
    for serial in "${devices[@]}"; do
        model="$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r' || true)"
        name="$(adb -s "$serial" shell getprop ro.product.device 2>/dev/null | tr -d '\r' || true)"
        sdk="$(adb -s "$serial" shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r' || true)"
        printf '==> Deploying app_process smoke to %s: %s (%s), SDK %s\n' "$serial" "${model:-unknown}" "${name:-unknown}" "${sdk:-unknown}"
        adb -s "$serial" shell mkdir -p /data/local/tmp/frida-java-bridge-rs
        adb -s "$serial" push target/aarch64-linux-android/debug/libfrida_java_bridge_rs.so /data/local/tmp/frida-java-bridge-rs/libfrida_java_bridge_rs.so
        adb -s "$serial" push smoke-fixtures/app-process-smoke.jar /data/local/tmp/frida-java-bridge-rs/app-process-smoke.jar
    done

app-smoke-run device="":
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
            echo "Multiple adb devices connected. Run 'just app-smoke-run <serial>' or 'just app-smoke-run all'." >&2
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
        printf '==> Running app_process smoke on %s\n' "$label"
        if adb -s "$serial" shell "CLASSPATH=/data/local/tmp/frida-java-bridge-rs/app-process-smoke.jar app_process /system/bin frida.java.bridge.rs.smoke.AppProcessSmoke"; then
            passed+=("$label")
        else
            status="$?"
            failed+=("$label [exit $status]")
            printf '==> app_process smoke failed on %s with exit %s\n' "$label" "$status" >&2
        fi
    done
    printf '\napp_process smoke summary:\n'
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

app-smoke device="":
    #!/usr/bin/env bash
    set -euo pipefail
    just app-smoke-deploy '{{ device }}'
    just app-smoke-run '{{ device }}'

app-smoke-all:
    just app-smoke all
