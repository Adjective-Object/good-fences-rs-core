//! Resolver-backed Sass imports stay within the build's checked input inventory.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use eyre::{ensure, WrapErr};
use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};
use sasso::{CanonicalUrl, CanonicalizeContext, Importer, ImporterError, ImporterResult};

pub(crate) fn is_sass(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "scss" || ext == "sass")
}

pub(crate) struct Compiled {
    pub css: String,
    pub source_map: Option<cache_flatbuffers::WireSourceMap>,
    pub inputs: BTreeMap<PathBuf, Vec<u8>>,
    pub warnings: Vec<String>,
}

struct ResolverImporter<'a, R> {
    entry: &'a Path,
    inventory: &'a crate::build::Inventory,
    resolver: &'a R,
    aliases: &'a BTreeSet<String>,
    inputs: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
}

fn syntax(path: &Path) -> sasso::Syntax {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("sass") => sasso::Syntax::Sass,
        Some("css") => sasso::Syntax::Css,
        _ => sasso::Syntax::Scss,
    }
}

/// Each tier is checked for ambiguity before considering the next one.
fn candidates(request: &str, from_import: bool) -> Vec<Vec<String>> {
    let (directory, name) = request
        .rsplit_once('/')
        .map_or(("", request), |(directory, name)| (directory, name));
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    let (stem, extensions): (&str, &[&str]) = if let Some(stem) = name.strip_suffix(".scss") {
        (stem, &["scss"])
    } else if let Some(stem) = name.strip_suffix(".sass") {
        (stem, &["sass"])
    } else if let Some(stem) = name.strip_suffix(".css") {
        return vec![vec![
            format!("{prefix}{stem}.css"),
            format!("{prefix}_{stem}.css"),
        ]];
    } else {
        (name, &["scss", "sass"])
    };
    let tier = |stem: &str, import_only: bool| {
        let infix = if import_only { ".import" } else { "" };
        extensions
            .iter()
            .flat_map(|ext| {
                [
                    format!("{prefix}{stem}{infix}.{ext}"),
                    format!("{prefix}_{stem}{infix}.{ext}"),
                ]
            })
            .collect::<Vec<_>>()
    };
    let mut tiers = Vec::new();
    if from_import {
        tiers.push(tier(stem, true));
    }
    tiers.push(tier(stem, false));
    // Explicit extensions never name directories.
    if stem == name {
        tiers.push(vec![
            format!("{request}.css"),
            format!("{prefix}_{name}.css"),
        ]);
        if from_import {
            tiers.push(vec![
                format!("{request}/index.import.scss"),
                format!("{request}/_index.import.scss"),
                format!("{request}/index.import.sass"),
                format!("{request}/_index.import.sass"),
            ]);
        }
        tiers.push(vec![
            format!("{request}/index.scss"),
            format!("{request}/_index.scss"),
            format!("{request}/index.sass"),
            format!("{request}/_index.sass"),
        ]);
    }
    tiers
}

impl<R: DependencyResolver> ResolverImporter<'_, R> {
    fn checked(&self, path: PathBuf) -> eyre::Result<PathBuf> {
        let path = self.inventory.admit(&path).wrap_err_with(|| {
            format!(
                "Sass dependency outside bounded source inventory: {}",
                path.display()
            )
        })?;
        ensure!(
            self.inventory.contains(&path),
            "Sass dependency outside bounded source inventory: {}",
            path.display()
        );
        ensure!(
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("scss" | "sass" | "css")
            ),
            "Sass import resolved to a non-stylesheet: {}",
            path.display()
        );
        Ok(path)
    }

    fn unique(&self, paths: BTreeSet<PathBuf>, request: &str) -> eyre::Result<Option<PathBuf>> {
        ensure!(
            paths.len() <= 1,
            "ambiguous Sass import {request:?}: {}",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(paths.into_iter().next())
    }

    fn resolve(&self, request: &str, ctx: &CanonicalizeContext<'_>) -> eyre::Result<PathBuf> {
        ensure!(
            !request.is_empty() && !request.contains(['\\', '\0', '?', '#']),
            "invalid Sass import {request:?}"
        );
        let containing = ctx
            .containing_url
            .map_or(self.entry, |url| Path::new(url.as_str()));
        let parent = containing
            .parent()
            .ok_or_else(|| eyre::eyre!("Sass importer has no parent: {}", containing.display()))?;
        let module_request = request.strip_prefix('~');
        let request = module_request.unwrap_or(request);
        if self.aliases.contains(request) {
            let target =
                self.resolver
                    .resolve_dependency(containing, request, ResolutionMode::Import)?;
            return match target {
                ResolvedDependency::File(path) => self.checked(path),
                ResolvedDependency::External(_) => {
                    eyre::bail!("Sass import cannot be external: {request:?}");
                }
            };
        }
        let tiers = candidates(request, ctx.from_import);
        if module_request.is_none() {
            for tier in &tiers {
                let mut paths = BTreeSet::new();
                for candidate in tier {
                    let path = url::Url::from_directory_path(parent)
                        .map_err(|()| eyre::eyre!("invalid Sass parent: {}", parent.display()))?
                        .join(candidate)?
                        .to_file_path()
                        .map_err(|()| {
                            eyre::eyre!("Sass import is not a local file: {candidate:?}")
                        })?;
                    if self.inventory.probe_file(&path)? {
                        paths.insert(self.checked(path)?);
                    }
                }
                if let Some(path) = self.unique(paths, request)? {
                    return Ok(path);
                }
            }
        }
        ensure!(
            !request.starts_with(['.', '/']) && !Path::new(request).is_absolute(),
            "{}: cannot resolve Sass import {request:?}",
            containing.display()
        );
        let mut failures = Vec::new();
        // The original request lets package exports and exact aliases select an
        // entry, including a "sass"/"style" conditional export.
        let mut tiers = tiers;
        let normal = usize::from(ctx.from_import);
        tiers[normal].insert(0, request.to_owned());
        for tier in tiers {
            let mut paths = BTreeSet::new();
            for candidate in tier {
                match self.resolver.resolve_dependency(
                    containing,
                    &candidate,
                    ResolutionMode::Import,
                ) {
                    Ok(ResolvedDependency::File(path)) => {
                        paths.insert(self.checked(path)?);
                    }
                    Ok(ResolvedDependency::External(_)) => {
                        eyre::bail!("Sass import cannot be external: {candidate:?}");
                    }
                    Err(error) => failures.push(format!("{candidate:?}: {error:#}")),
                }
            }
            if let Some(path) = self.unique(paths, request)? {
                return Ok(path);
            }
        }
        eyre::bail!(
            "{}: cannot resolve Sass import {request:?}\n{}",
            containing.display(),
            failures.join("\n")
        )
    }
}

impl<R: DependencyResolver> Importer for ResolverImporter<'_, R> {
    fn canonicalize(
        &self,
        request: &str,
        ctx: &CanonicalizeContext<'_>,
    ) -> Result<Option<CanonicalUrl>, ImporterError> {
        self.resolve(request, ctx)
            .and_then(|path| {
                Ok(Some(CanonicalUrl::new(
                    path.to_str()
                        .ok_or_else(|| eyre::eyre!("Sass path is not UTF-8"))?,
                )))
            })
            .map_err(|error| ImporterError {
                message: format!("{error:#}"),
            })
    }

    fn load(&self, canonical: &CanonicalUrl) -> Result<Option<ImporterResult>, ImporterError> {
        let load = || -> eyre::Result<_> {
            let path = self.checked(PathBuf::from(canonical.as_str()))?;
            let input = fs::read(&path)?;
            let contents = std::str::from_utf8(&input)
                .wrap_err_with(|| format!("{}: Sass is not UTF-8", path.display()))?
                .to_owned();
            let mut inputs = self.inputs.borrow_mut();
            if let Some(previous) = inputs.get(&path) {
                ensure!(
                    previous == &input,
                    "Sass dependency changed during build: {}",
                    path.display()
                );
            } else {
                inputs.insert(path.clone(), input);
            }
            Ok(Some(ImporterResult {
                contents,
                syntax: syntax(&path),
                source_map_url: None,
            }))
        };
        load().map_err(|error| ImporterError {
            message: format!("{error:#}"),
        })
    }
}

pub(crate) fn compile(
    path: &Path,
    input: &[u8],
    inventory: &crate::build::Inventory,
    resolver: &impl DependencyResolver,
    aliases: &BTreeSet<String>,
    source_maps: bool,
) -> eyre::Result<Compiled> {
    let importer = ResolverImporter {
        entry: path,
        inventory,
        resolver,
        aliases,
        inputs: RefCell::new(BTreeMap::from([(path.to_owned(), input.to_vec())])),
    };
    let warnings = Rc::new(RefCell::new(Vec::new()));
    let diagnostics = warnings.clone();
    let options = sasso::Options::default()
        .with_syntax(syntax(path))
        .with_importer(&importer)
        .with_url(
            path.to_str()
                .ok_or_else(|| eyre::eyre!("Sass path is not UTF-8"))?,
        )
        .with_unicode(false)
        .with_warn_handler(Rc::new(move |event| {
            diagnostics.borrow_mut().push(event.formatted.to_owned());
        }));
    let input = std::str::from_utf8(input)
        .wrap_err_with(|| format!("{}: Sass is not UTF-8", path.display()))?;
    let (css, source_map) = if source_maps {
        let result = sasso::compile_with_source_map(
            input,
            &options.with_source_map_include_sources(true),
        )
        .map_err(|error| eyre::eyre!("{}: Sass compilation failed: {error}", path.display()))?;
        let source_map = checked_map(path, &result.source_map, &importer.inputs.borrow())?;
        (result.css, Some(source_map))
    } else {
        let css = sasso::compile(input, &options)
            .map_err(|error| eyre::eyre!("{}: Sass compilation failed: {error}", path.display()))?;
        (css, None)
    };
    Ok(Compiled {
        css,
        source_map,
        inputs: importer.inputs.into_inner(),
        warnings: warnings.take(),
    })
}

fn checked_map(
    path: &Path,
    map: &sasso::SourceMap,
    inputs: &BTreeMap<PathBuf, Vec<u8>>,
) -> eyre::Result<cache_flatbuffers::WireSourceMap> {
    use cache_flatbuffers::{wire, MapData, MapSource, WireSourceMap};

    let json = map.to_json();
    let decoded = oxc_sourcemap::SourceMap::from_json_string(&json)?;
    let location = url::Url::from_file_path(path)
        .map_err(|()| eyre::eyre!("invalid Sass source map path: {}", path.display()))?;
    let urls = decoded
        .get_sources()
        .map(|source| {
            let source = Path::new(source);
            ensure!(
                inputs.contains_key(source),
                "Sass source map references unchecked input: {}",
                source.display()
            );
            url::Url::from_file_path(source)
                .map(String::from)
                .map_err(|()| eyre::eyre!("invalid Sass source map input: {}", source.display()))
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    let sources = decoded
        .get_sources()
        .enumerate()
        .map(|(index, source)| {
            Ok(MapSource {
                original: source,
                url: &urls[index],
                content: Some(std::str::from_utf8(&inputs[Path::new(source)])?),
            })
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    let names = decoded.get_names().collect::<Vec<_>>();
    let tokens = decoded
        .get_tokens()
        .map(|token| {
            wire::MapToken::new(
                token.get_dst_line(),
                token.get_dst_col(),
                token.get_src_line(),
                token.get_src_col(),
                token.get_source_id().map_or(-1, |id| id as i32),
                token.get_name_id().map_or(-1, |id| id as i32),
            )
        })
        .collect::<Vec<_>>();
    WireSourceMap::encode(
        path.to_str()
            .ok_or_else(|| eyre::eyre!("Sass path is not UTF-8"))?,
        json.as_bytes(),
        MapData {
            location: location.as_str(),
            file: decoded.get_file().map(AsRef::as_ref),
            source_root: None,
            sources: &sources,
            names: &names,
            tokens: &tokens,
        },
    )
}
