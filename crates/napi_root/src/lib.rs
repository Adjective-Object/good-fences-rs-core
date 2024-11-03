// These are disabled in 'test' because napi indirectly references
// symbols from node, which would cause the test binaries to fail to link.
#[cfg(not(test))]
mod _module {
    pub use good_fences_napi::*;
    pub use unused_finder_napi::*;
}
#[cfg(not(test))]
pub use _module::*;
