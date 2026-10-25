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
    pub evaluation_dependencies: &'a [&'a str],
    pub wrapper_format: ModuleFormat,
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
        if input.is_module {
            return Ok(PreparedCssModule {
                source: evaluation_imports(input.evaluation_dependencies)?
                    + &mapping_exports(input.name_mappings)?,
                parser_path: input.stylesheet.with_extension("css.mjs"),
                package: ModuleFormat::Esm,
                relocations: Vec::new(),
            });
        }
        ensure!(
            input.name_mappings.is_empty(),
            "{}: global CSS cannot have name mappings",
            input.stylesheet.display()
        );
        Ok(PreparedCssModule {
            source: crate::resource::public_url_module(input.public_url, input.wrapper_format)?,
            parser_path: input.stylesheet.with_extension(
                if input.wrapper_format == ModuleFormat::Esm {
                    "css.mjs"
                } else {
                    "css.cjs"
                },
            ),
            package: input.wrapper_format,
            relocations: Vec::new(),
        })
    }
}

pub(crate) struct AutoMount;

impl CssLoadingStrategy for AutoMount {
    fn generate_module(&self, input: &CssModuleInput<'_>) -> eyre::Result<PreparedCssModule> {
        ensure!(
            input.is_module || input.name_mappings.is_empty(),
            "{}: global CSS cannot have name mappings",
            input.stylesheet.display()
        );
        let mut source = evaluation_imports(input.evaluation_dependencies)?;
        source.push_str(
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
            ".href;\nconst link = document.createElement(\"link\");\nlink.rel = \"stylesheet\";\nlink.href = href;\ndocument.head.appendChild(link);\n",
        );
        if input.is_module {
            source.push_str(&mapping_exports(input.name_mappings)?);
        } else {
            source.push_str("export {href as default};\n");
        }
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

fn evaluation_imports(dependencies: &[&str]) -> eyre::Result<String> {
    let mut source = String::new();
    for request in dependencies {
        source.push_str(&format!("import {};\n", serde_json::to_string(request)?));
    }
    Ok(source)
}

fn mapping_exports(mappings: &BTreeMap<String, String>) -> eyre::Result<String> {
    let mut source = String::new();
    for (index, (name, value)) in mappings.iter().enumerate() {
        if name == "default" {
            continue;
        }
        // Private bindings avoid keywords and loader locals; string aliases also
        // represent decoded CSS names outside JavaScript IdentifierName syntax.
        source.push_str(&format!(
            "const __css_{index} = {};\nexport {{__css_{index} as {}}};\n",
            serde_json::to_string(value)?,
            serde_json::to_string(name)?,
        ));
    }
    // Pure construction keeps literal-key maps prunable without binding-read barriers.
    source.push_str("const __css_default = /*#__PURE__*/ (() => ({\n");
    for (name, value) in mappings {
        source.push_str(&format!(
            "[{}]: {},\n",
            serde_json::to_string(name)?,
            serde_json::to_string(value)?,
        ));
    }
    source.push_str("}))();\nexport {__css_default as default};\n");
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_loaded_preserves_global_url_bytes_and_emits_separate_module_bindings() {
        let mappings = BTreeMap::new();
        let mut input = CssModuleInput {
            stylesheet: Path::new("style.css"),
            public_url: "./assets/a\"b.css",
            name_mappings: &mappings,
            is_module: false,
            evaluation_dependencies: &[],
            wrapper_format: ModuleFormat::CommonJs,
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
        assert_eq!(
            CallerLoaded.generate_module(&input).unwrap().source,
            "const __css_default = /*#__PURE__*/ (() => ({\n}))();\nexport {__css_default as default};\n"
        );
        let mappings = BTreeMap::from([("button".into(), "_scoped".into())]);
        input.name_mappings = &mappings;
        let prepared = CallerLoaded.generate_module(&input).unwrap();
        assert_eq!(prepared.package, ModuleFormat::Esm);
        assert_eq!(
            prepared.source,
            "const __css_0 = \"_scoped\";\nexport {__css_0 as \"button\"};\n\
             const __css_default = /*#__PURE__*/ (() => ({\n[\"button\"]: \"_scoped\",\n}))();\nexport {__css_default as default};\n"
        );
        input.is_module = false;
        assert!(CallerLoaded.generate_module(&input).is_err());
    }
}
