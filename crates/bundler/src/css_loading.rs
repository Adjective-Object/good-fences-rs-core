//! Loading stubs consume prepared resource identities, never CSS syntax.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ast_segmenter::ModuleFormat;
use eyre::ensure;
use oxc_span::Span;

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
    pub relocations: Vec<UrlRelocation>,
}

#[derive(Debug, Clone)]
pub(crate) struct UrlRelocation {
    pub operation: Span,
    pub public_url: String,
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
            relocations: Vec::new(),
        })
    }
}

pub(crate) struct AutoMount;

impl CssLoadingStrategy for AutoMount {
    fn generate_module(&self, input: &CssModuleInput<'_>) -> eyre::Result<PreparedCssModule> {
        ensure!(
            !input.is_module && input.name_mappings.is_empty(),
            "{}: CSS Modules mappings are not supported yet",
            input.stylesheet.display()
        );
        let mut source = String::from(
            "if (typeof document === \"undefined\") throw new Error(\"CSS auto-mount requires document\");\nconst href = ",
        );
        let start = source.len() as u32;
        // Final assembly supplies the owner anchor to this exact construction.
        source.push_str(&format!(
            "new URL({})",
            serde_json::to_string(input.public_url)?
        ));
        let operation = Span::new(start, source.len() as u32);
        source.push_str(
            ".href;\nconst link = document.createElement(\"link\");\nlink.rel = \"stylesheet\";\nlink.href = href;\ndocument.head.appendChild(link);\nexport {href as default};\n",
        );
        Ok(PreparedCssModule {
            source,
            parser_path: input.stylesheet.with_extension("css.mjs"),
            package: ModuleFormat::Esm,
            relocations: vec![UrlRelocation {
                operation,
                public_url: input.public_url.to_owned(),
            }],
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
