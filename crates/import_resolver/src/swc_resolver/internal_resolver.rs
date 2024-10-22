use std::path::PathBuf;

use anyhow::{Error, Ok};
use hashbrown::HashSet;
use swc_ecma_loader::resolve::{Resolution, Resolve};

use super::util::package_name;
use packagejson::PackageJson;

/// A resolver which will only resolve internal modules, using the inner resolver.
/// Other modules are marked as External.
#[derive(Debug)]
pub struct InternalOnlyResolver<R> {
    // set of packages to consider "internal"
    internal_packages: HashSet<String>,
    inner_resolver: R,
}

impl<R> InternalOnlyResolver<R> {
    /// Create a new InternalOnlyResolver, with no initial package list
    pub fn new_empty(inner_resolver: R) -> Self {
        InternalOnlyResolver {
            internal_packages: HashSet::new(),
            inner_resolver,
        }
    }

    pub fn add_package(&mut self, package: &PackageJson) {
        if let Some(name) = &package.name {
            self.internal_packages.insert(name.clone());
        }
    }
}

/// Implements Resolve for the InternalOnlyResolver
impl<R: Resolve> Resolve for InternalOnlyResolver<R> {
    fn resolve(
        &self,
        base: &swc_common::FileName,
        module_specifier: &str,
    ) -> Result<Resolution, Error> {
        // split the package name off the module_specifier, if any
        match package_name(module_specifier) {
            Some(packagename) if !self.internal_packages.contains(packagename) => {
                // the package is not internal, leave it unresolved.
                // (this will persist the import/require in the output)
                Ok(Resolution {
                    filename: swc_common::FileName::Real(PathBuf::from(module_specifier)),
                    slug: None,
                })
            }
            _ => {
                // if the package is internal, or it is not a package import
                // (e.g. a relative or absolute path), resolve with the inner resolver
                self.inner_resolver.resolve(base, module_specifier)
            }
        }
    }
}
