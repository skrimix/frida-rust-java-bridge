use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
        || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64")
    {
        return;
    }

    let ndk = env::var("ANDROID_NDK_HOME")
        .or_else(|_| env::var("ANDROID_NDK_ROOT"))
        .expect("ANDROID_NDK_HOME or ANDROID_NDK_ROOT must be set");

    let clang_root = PathBuf::from(ndk).join("toolchains/llvm/prebuilt/linux-x86_64/lib/clang");

    let mut versions: Vec<_> = fs::read_dir(&clang_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();

    versions.sort_by_key(|e| e.file_name());

    let version = versions.last().unwrap().file_name();
    let builtins_dir = clang_root.join(version).join("lib/linux");

    println!("cargo:rustc-link-search=native={}", builtins_dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.builtins-aarch64-android");
    println!("cargo:rustc-link-arg=-Wl,--no-undefined");
}
