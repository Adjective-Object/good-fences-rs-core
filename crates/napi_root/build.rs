extern crate napi_build;

fn main() {
    napi_build::setup();

    // On macOS, the linker strips #[ctor] static constructors from dependent
    // rlibs (good_fences_napi, unused_finder_napi) because they appear as dead
    // code in the final cdylib. Force inclusion of all archive members.
    // Linux/Windows handle static initializers differently and don't need this.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" || target_os == "ios" {
        println!("cargo:rustc-link-arg=-Wl,-all_load");
    }
}
