extern crate relative_path;
extern crate serde;

mod resolver;
mod tsconfig_paths;

pub use resolver::create_caching_resolver;
pub use tsconfig_paths::resolve_with_extension;
pub use tsconfig_paths::resolve_ts_import;
pub use tsconfig_paths::ResolvedImport;