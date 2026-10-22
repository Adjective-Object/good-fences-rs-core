//! Source-local work only; the coordinator retains resolver and CSS ownership.
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_PREPARE: std::cell::RefCell<Option<std::sync::Arc<dyn Fn(&Path) + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) enum Prepared {
    Script(Loaded),
    Resource {
        input: Vec<u8>,
        format: ModuleFormat,
    },
}

pub(super) struct Loaded {
    pub source: Option<Source>,
    pub parsed: ast_segmenter::ParsedModule,
    pub status: Option<CacheStatus>,
    pub warning: Option<String>,
    pub transpiled: Option<Transpiled>,
}

pub(super) fn enqueue(
    paths: impl IntoIterator<Item = PathBuf>,
    scheduled: &mut BTreeSet<PathBuf>,
    resources: &mut BTreeSet<PathBuf>,
    options: &BuildOptions,
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| scheduled.insert(path.clone()))
        .filter(|path| {
            if crate::resource::kind(path, &options.css)
                == crate::resource::ResourceKind::Script
            {
                true
            } else {
                resources.insert(path.clone());
                false
            }
        })
        .collect()
}

pub(super) fn read(
    root: &Path,
    path: &Path,
    inventory: &Inventory,
    previous: Option<&Source>,
    options: &BuildOptions,
    declarations: &PreparedDeclarations,
    base: &crate::output_url::PublicBase,
) -> eyre::Result<Prepared> {
    let _span = tracing::info_span!("prepare.read").entered();
    ensure!(
        inventory.contains(path),
        "source outside reachable build inventory (or reserved/excluded): {}",
        path.display()
    );
    let format =
        tracing::info_span!("prepare.package_format").in_scope(|| package_format(root, path))?;
    let input = tracing::info_span!("prepare.read_bytes")
        .in_scope(|| fs::read(path))
        .wrap_err_with(|| format!("source {}", path.display()))?;
    if crate::resource::kind(path, &options.css)
        != crate::resource::ResourceKind::Script
    {
        return Ok(Prepared::Resource { input, format });
    }
    if crate::transpile::required(path) {
        let config = ParserConfig::for_path(path, format)?;
        if let Some(previous) = previous.filter(|previous| {
            previous.input == input
                && previous.transpiled.as_ref().is_some_and(|transpiled| {
                    transpiled.config == config
                        && (!options.output.source_maps || transpiled.source_map.is_some())
                })
        }) {
            let parser_path = path.with_extension(if previous.record.config().commonjs() {
                "cjs"
            } else {
                "mjs"
            });
            let mut loaded = load(
                path,
                input,
                previous.text.clone(),
                &parser_path,
                format,
                Some(previous),
                options.cache,
                declarations,
            )?;
            if loaded.source.is_some() {
                loaded.transpiled = previous.transpiled.clone();
            }
            return Ok(Prepared::Script(loaded));
        }
        let source = std::str::from_utf8(&input)
            .wrap_err_with(|| format!("{}: source is not UTF-8", path.display()))?;
        let compiled = tracing::info_span!("prepare.transpile", bytes = input.len())
            .in_scope(|| {
                crate::transpile::compile(path, source, format, options.output.source_maps)
            })?;
        let mut loaded = load(
            path,
            input,
            compiled.source,
            &compiled.parser_path,
            format,
            previous,
            options.cache,
            declarations,
        )?;
        loaded.transpiled = Some(Transpiled {
            config,
            source_map: compiled.source_map,
            source_map_hint: compiled.source_map_hint,
        });
        return Ok(Prepared::Script(loaded));
    }
    let wrapper_format = match options.resolver.target_env {
        TargetEnv::Browser => ModuleFormat::Esm,
        TargetEnv::Node => ModuleFormat::CommonJs,
    };
    let text = crate::resource::prepare(path, &input, base, wrapper_format)?;
    Ok(Prepared::Script(load(
        path,
        input,
        text,
        path,
        format,
        previous,
        options.cache,
        declarations,
    )?))
}

pub(super) fn load(
    path: &Path,
    input: Vec<u8>,
    text: String,
    parser_path: &Path,
    package: ModuleFormat,
    previous: Option<&Source>,
    cache: CacheMode,
    declarations: &PreparedDeclarations,
) -> eyre::Result<Loaded> {
    let _span = tracing::info_span!("prepare.load", bytes = input.len()).entered();
    let config = ParserConfig::for_path(parser_path, package)?;
    let previous = tracing::info_span!("prepare.memory_check").in_scope(|| {
        previous.filter(|s| {
            s.input == input
                && s.text == text
                && config.accepts(s.record.config())
                && s.record.matches_prepared_declarations(declarations)
        })
    });
    if let Some(source) = previous {
        return Ok(Loaded {
            source: None,
            parsed: source.record.to_parsed(&source.text)?,
            status: None,
            warning: None,
            transpiled: None,
        });
    }
    let loaded = segment_cache::load_prepared_with_declarations(
        &logger::StdioLogger::new(),
        path,
        segment_cache::PreparedSource {
            source: &text,
            original: &input,
            parser_path,
            package,
        },
        cache,
        declarations,
    )?;
    let warning = if let CacheWrite::Failed(message) = loaded.write {
        Some(format!("{}: cache write: {message}", path.display()))
    } else {
        None
    };
    Ok(Loaded {
        parsed: loaded.record.to_parsed(&loaded.source)?,
        source: Some(Source {
            css_identity: None,
            input,
            text: loaded.source,
            record: loaded.record,
            transpiled: None,
        }),
        status: Some(loaded.status),
        warning,
        transpiled: None,
    })
}
