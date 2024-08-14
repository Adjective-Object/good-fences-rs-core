use swc_core::ecma::loader::resolvers::{
    lru::CachingResolver, node::NodeModulesResolver, tsc::TsConfigResolver,
};
use tsconfig_paths::TsconfigPathsJson;

pub type TsconfigPathsResolver = CachingResolver<TsConfigResolver<NodeModulesResolver>>;

pub fn create_tsconfig_paths_resolver(tsconfig: &TsconfigPathsJson) -> TsconfigPathsResolver {
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
