fn main() {
    // Google Play requires native libraries in apps targeting Android 15+
    // (API 35+) to be 16 KB page-size compatible. NDK r28+ aligns by default,
    // but passing the flag explicitly keeps the build correct on r27 too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
    tauri_build::build()
}
