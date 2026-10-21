#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Err(error) = quickjs_bundler_harness_fuzz::check(data) {
        panic!("{error:#}");
    }
});
