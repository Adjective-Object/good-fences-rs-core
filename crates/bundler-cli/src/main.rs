use clap::Parser;

#[global_allocator]
static ALLOC: sasso::ScopedAlloc = sasso::ScopedAlloc;

fn main() -> eyre::Result<()> {
    bundler_cli::Cli::parse().run(&mut std::io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use sasso::{
        compile, compile_with_source_map, CanonicalUrl, CanonicalizeContext, Importer,
        ImporterError, ImporterResult, Options, Syntax,
    };

    const HELPERS: &str = r#"
@function double($value) { @return $value * 2; }
@mixin emit($value) { width: double($value); }
"#;

    #[derive(Default)]
    struct RecordingImporter {
        loaded: RefCell<Vec<(String, String)>>,
    }

    impl Importer for RecordingImporter {
        fn canonicalize(
            &self,
            url: &str,
            _context: &CanonicalizeContext<'_>,
        ) -> Result<Option<CanonicalUrl>, ImporterError> {
            Ok((url == "helpers").then(|| CanonicalUrl::new("memory:helpers.scss")))
        }

        fn load(&self, canonical: &CanonicalUrl) -> Result<Option<ImporterResult>, ImporterError> {
            self.loaded
                .borrow_mut()
                .push((canonical.as_str().to_owned(), HELPERS.to_owned()));
            Ok(Some(ImporterResult {
                contents: HELPERS.to_owned(),
                syntax: Syntax::Scss,
                source_map_url: Some("memory:helpers.scss".to_owned()),
            }))
        }
    }

    // Keep this sequential: native Sasso arenas are pooled per concurrent compile.
    #[test]
    fn sass_outputs_and_callbacks_survive_arena_reuse_after_errors() {
        let importer = RecordingImporter::default();
        let warnings = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&warnings);
        let mut options = Options::default().with_warn_handler(Rc::new(move |event| {
            captured
                .borrow_mut()
                .push((event.message.to_owned(), event.formatted.to_owned()));
        }));
        options.importer = Some(&importer);
        options.url = Some("memory:entry.scss");
        options.source_map_include_sources = true;
        let source = r#"@use "helpers"; @warn "retained warning";
.retained { @include helpers.emit(7px); }"#;
        let mapped = compile_with_source_map(source, &options).unwrap();
        let css = compile(".plain { value: retained; }", &Options::default()).unwrap();
        let error = compile("@error \"retained failure\";", &options).unwrap_err();
        let mapped_error =
            compile_with_source_map("@error \"retained mapped failure\";", &options).unwrap_err();
        assert!(mapped.css.contains("width: 14px;"));
        assert!(!mapped.source_map.mappings.is_empty());
        assert!(mapped
            .source_map
            .sources_content
            .as_ref()
            .unwrap()
            .iter()
            .any(|contents| contents == source));
        assert!(error.message.contains("retained failure"));
        assert!(mapped_error.message.contains("retained mapped failure"));
        assert_eq!(warnings.borrow()[0].0, "retained warning");
        assert!(warnings.borrow()[0].1.contains("retained warning"));
        let expected_css = mapped.css.clone();
        let expected_map = mapped.source_map.to_json();
        let expected_error = format!("{error:#}");
        let expected_mapped_error = format!("{mapped_error:#}");
        let expected_warnings = warnings.borrow().clone();

        for value in 0..256 {
            let source = format!("{HELPERS}\n.churn-{value} {{ @include emit({value}px); }}");
            let options = Options::default();
            let plain = compile(&source, &options).unwrap();
            let mapped = compile_with_source_map(&source, &options).unwrap();
            assert!(plain.contains(&format!("width: {}px;", value * 2)));
            assert_eq!(plain, mapped.css);
        }

        assert_eq!(mapped.css, expected_css);
        assert_eq!(mapped.source_map.to_json(), expected_map);
        assert_eq!(css, ".plain {\n  value: retained;\n}");
        assert_eq!(format!("{error:#}"), expected_error);
        assert_eq!(format!("{mapped_error:#}"), expected_mapped_error);
        assert_eq!(*warnings.borrow(), expected_warnings);
        assert_eq!(
            *importer.loaded.borrow(),
            vec![("memory:helpers.scss".to_owned(), HELPERS.to_owned())]
        );
    }
}
