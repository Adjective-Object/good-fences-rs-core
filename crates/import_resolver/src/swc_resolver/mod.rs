use anyhow::Error;
use combined_resolver::{CombinedResolver, CombinedResolverCaches};
use common::AHashMap;
use core::fmt;
use ouroboros::self_referencing;
use std::{
    fmt::{Debug, Formatter},
    path::PathBuf,
};
use swc_common::FileName;
use swc_ecma_loader::{
    resolve::{Resolution, Resolve},
    TargetEnv,
};

pub mod combined_resolver;
mod common;
pub mod internal_resolver;
mod node_resolver;
mod pkgjson_rewrites;
mod tsconfig;
mod tsconfig_resolver;
mod util;

// Wrapper for Combined resolver that owns its own caches, instead of
// referencing an externally-owned set of caches.
#[self_referencing]
pub struct MonorepoResolver {
    root_dir: PathBuf,
    caches: CombinedResolverCaches,
    #[borrows(caches, root_dir)]
    #[not_covariant]
    resolver: CombinedResolver<'this>,
}

impl MonorepoResolver {
    pub fn new_resolver(
        root_dir: PathBuf,
        target_env: TargetEnv,
        alias: AHashMap<String, String>,
        preserve_symlinks: bool,
    ) -> Self {
        MonorepoResolverBuilder {
            root_dir,
            caches: CombinedResolverCaches::new(),
            resolver_builder: |caches, root_dir| {
                caches.resolver(root_dir, target_env, alias, preserve_symlinks)
            },
        }
        .build()
    }

    pub fn new_default_resolver(root_dir: PathBuf) -> Self {
        MonorepoResolver::new_resolver(root_dir, TargetEnv::Browser, AHashMap::default(), true)
    }
}

impl Debug for MonorepoResolver {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonorepoResolver")
            .field("root_dir", &self.borrow_root_dir())
            .field("caches", &self.borrow_caches())
            .field("resolver", &"<resolver>".to_owned())
            .finish()
    }
}

impl Resolve for MonorepoResolver {
    fn resolve(&self, specifier: &FileName, referrer: &str) -> Result<Resolution, Error> {
        self.with_resolver(|resolver| resolver.resolve(specifier, referrer))
    }
}
