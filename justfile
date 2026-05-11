check:
    cargo ndk -t arm64-v8a clippy

build:
    cargo ndk -t arm64-v8a build

build-release:
    cargo ndk -t arm64-v8a build --release