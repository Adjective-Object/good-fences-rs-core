//! Loading stubs consume prepared resource identities, never CSS syntax.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ast_segmenter::ModuleFormat;
use eyre::ensure;

pub(crate) struct CssModuleInput<'a> {
    pub stylesheet: &'a Path,
    pub public_url: &'a str,
    pub name_mappings: &'a BTreeMap<String, String>,
    pub is_module: bool,
}

pub(crate) struct PreparedCssModule {
    pub source: String,
    pub parser_path: PathBuf,
    pub package: ModuleFormat,
}

pub(crate) trait CssLoadingStrategy {
    fn generate_module(&self, input: &CssModuleInput<'_>) -> eyre::Result<PreparedCssModule>;
}

pub(crate) struct CallerLoaded;

impl CssLoadingStrategy for CallerLoaded {
    fn generate_module(&self, input: &CssModuleInput<'_>) -> eyre::Result<PreparedCssModule> {
        ensure!(
            !input.is_module && input.name_mappings.is_empty(),
            "{}: CSS Modules mappings are not supported yet",
            input.stylesheet.display()
        );
        Ok(PreparedCssModule {
            source: crate::resource::public_url_module(input.public_url)?,
            parser_path: input.stylesheet.with_extension("css.cjs"),
            package: ModuleFormat::CommonJs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_loaded_preserves_url_bytes_and_rejects_unimplemented_mappings() {
        let mappings = BTreeMap::new();
        let mut input = CssModuleInput {
            stylesheet: Path::new("style.css"),
            public_url: "./assets/a\"b.css",
            name_mappings: &mappings,
            is_module: false,
        };
        let prepared = CallerLoaded.generate_module(&input).unwrap();
        assert_eq!(
            prepared.source,
            "module.exports = \"./assets/a\\\"b.css\";\n"
        );
        assert_eq!(
            segment_cache::ParserConfig::for_path(&prepared.parser_path, prepared.package).unwrap(),
            segment_cache::ParserConfig::for_path(Path::new("old.asset.cjs"), ModuleFormat::Esm)
                .unwrap()
        );
        input.is_module = true;
        assert!(CallerLoaded.generate_module(&input).is_err());
        input.is_module = false;
        let mappings = BTreeMap::from([("button".into(), "_scoped".into())]);
        input.name_mappings = &mappings;
        assert!(CallerLoaded.generate_module(&input).is_err());
    }
}
