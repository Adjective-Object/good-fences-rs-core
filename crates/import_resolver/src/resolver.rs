use swc_core::ecma::loader::resolvers::{
    lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver,
};
use tsconfig_paths::TsconfigPathsJson;

pub fn create_caching_resolver(
    tsconfig: &TsconfigPathsJson,
) -> CachingResolver<TsConfigResolver<NodeModulesResolver>> {
    let resolver: CachingResolver<TsConfigResolver<NodeModulesResolver>> = CachingResolver::new(
        60_000,
        TsConfigResolver::new(
            NodeModulesResolver::default(),
            ".".into(),
            tsconfig
                .compiler_options
                .paths
                .clone()
                .into_iter()
                .collect(),
        ),
    );
    resolver
}
