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
            if crate::resource::kind(path, &options.css.css_modules)
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
    ensure!(
        inventory.contains(path),
        "source outside reachable build inventory (or reserved/excluded): {}",
        path.display()
    );
    let format = package_format(root, path)?;
    let input = fs::read(path).wrap_err_with(|| format!("source {}", path.display()))?;
    if crate::resource::kind(path, &options.css.css_modules)
        != crate::resource::ResourceKind::Script
    {
        return Ok(Prepared::Resource { input, format });
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
    let config = ParserConfig::for_path(parser_path, package)?;
    if let Some(source) = previous.filter(|s| {
        s.input == input
            && s.text == text
            && s.record.config == config
            && s.record.matches_prepared_declarations(declarations)
    }) {
        return Ok(Loaded {
            source: None,
            parsed: source.record.to_parsed(&source.text)?,
            status: None,
            warning: None,
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
        }),
        status: Some(loaded.status),
        warning,
    })
}
