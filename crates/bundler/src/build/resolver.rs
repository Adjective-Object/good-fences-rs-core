use std::cell::RefCell;

use super::*;

/// Outside the input root, Node ancestry can probe unenumerated node_modules
/// trees. Replay actual requests rather than scanning those unrelated trees.
pub(super) struct BuildResolver {
    inner: StructuralResolver,
    replay: bool,
    requests: RefCell<Vec<(PathBuf, String, ResolutionMode, ResolvedDependency)>>,
}

impl BuildResolver {
    pub(super) fn options(options: &BuildOptions) -> NodeModulesResolverOptions {
        let mut resolver = options.resolver.clone();
        // A target-only switch follows host defaults; nondefault condition sets
        // are explicit caller policy and remain untouched.
        if [TargetEnv::Node, TargetEnv::Browser]
            .into_iter()
            .any(|target| {
                resolver.export_conditions
                    == NodeModulesResolverOptions::default_for_env(target).export_conditions
            })
        {
            resolver.export_conditions =
                NodeModulesResolverOptions::default_for_env(resolver.target_env).export_conditions;
        }
        resolver
    }

    pub(super) fn new(root: &Path, options: &BuildOptions, replay: bool) -> Self {
        Self {
            inner: StructuralResolver::new(
                root.to_owned(),
                Self::options(options),
                options.externals.clone(),
            ),
            replay,
            requests: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn recheck(&self, root: &Path, options: &BuildOptions) -> eyre::Result<()> {
        let fresh = StructuralResolver::new(
            root.to_owned(),
            Self::options(options),
            options.externals.clone(),
        );
        for (base, request, mode, before) in self.requests.borrow().iter() {
            let after = fresh
                .resolve_dependency(base, request, *mode)
                .wrap_err_with(|| {
                    format!(
                        "resolution inputs changed during build: {}: {request:?}",
                        base.display()
                    )
                })?;
            ensure!(
                &after == before,
                "resolution inputs changed during build: {}: {request:?}",
                base.display()
            );
        }
        Ok(())
    }
}

impl DependencyResolver for BuildResolver {
    fn resolve_dependency(
        &self,
        base: &Path,
        specifier: &str,
        mode: ResolutionMode,
    ) -> eyre::Result<ResolvedDependency> {
        let resolved = self.inner.resolve_dependency(base, specifier, mode)?;
        if self.replay {
            self.requests.borrow_mut().push((
                base.to_owned(),
                specifier.to_owned(),
                mode,
                resolved.clone(),
            ));
        }
        Ok(resolved)
    }
}
