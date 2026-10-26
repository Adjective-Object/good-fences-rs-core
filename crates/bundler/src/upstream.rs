//! Output-only upstream map inputs; JS extraction caches only directive spans.
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use base64::Engine;
use cache_flatbuffers::{wire, MapData, MapSource, WireSourceMap, MAX_RECORD_BYTES};
use eyre::{ensure, WrapErr};
use segment_cache::CacheMode;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Upstream {
    map: WireSourceMap,
    linked: Option<PathBuf>,
    input: crate::prepared_text::InputIdentity,
}

impl Upstream {
    pub(crate) fn map(&self) -> &WireSourceMap {
        &self.map
    }

    pub(crate) fn recheck(&self) -> eyre::Result<()> {
        if let Some(path) = &self.linked {
            ensure!(
                path.canonicalize()? == *path && self.input.matches(&read(path)?),
                "upstream source map changed during build: {}",
                path.display()
            );
        }
        Ok(())
    }
}

fn read(path: &Path) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .wrap_err_with(|| format!("opening upstream source map {}", path.display()))?
        .take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .wrap_err_with(|| format!("reading upstream source map {}", path.display()))?;
    ensure!(
        bytes.len() <= MAX_RECORD_BYTES,
        "upstream source map exceeds byte limit"
    );
    Ok(bytes)
}

pub(crate) fn is_cache_artifact(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.ends_with(".upstream.seg.fb") || name.contains(".upstream.seg.fb.stage-")
    })
}

pub(crate) fn load(
    inventory: &crate::build::Inventory,
    path: &Path,
    source: &str,
    directive: Option<oxc_span::Span>,
    mode: CacheMode,
    previous: Option<&Upstream>,
) -> eyre::Result<(Option<Upstream>, Option<String>)> {
    let Some(span) = directive else {
        return Ok((None, None));
    };
    let request = ast_segmenter::emission::source_mapping_url(
        &source[span.start as usize..span.end as usize],
    )
    .ok_or_else(|| eyre::eyre!("invalid cached source map directive"))?;
    let result = load_inner(inventory, path, source, request, mode, previous);
    result.wrap_err_with(|| format!("{}: upstream source map", path.display()))
}

fn load_inner(
    inventory: &crate::build::Inventory,
    path: &Path,
    source: &str,
    request: &str,
    mode: CacheMode,
    previous: Option<&Upstream>,
) -> eyre::Result<(Option<Upstream>, Option<String>)> {
    ensure!(!request.is_empty(), "empty sourceMappingURL");
    let source_url = Url::from_file_path(path).map_err(|()| eyre::eyre!("invalid source path"))?;
    let (input, linked, location) = if let Some(data) = request.strip_prefix("data:") {
        let (metadata, payload) = data
            .split_once(',')
            .ok_or_else(|| eyre::eyre!("malformed inline source map data URL"))?;
        let mut parts = metadata.split(';');
        ensure!(
            parts.next() == Some("application/json"),
            "unsupported inline source map media type"
        );
        let mut base64 = false;
        for part in parts {
            match part {
                "base64" if !base64 => base64 = true,
                "charset=utf-8" | "charset=UTF-8" => {}
                _ => eyre::bail!("unsupported inline source map encoding"),
            }
        }
        ensure!(
            payload.len() <= MAX_RECORD_BYTES * 2,
            "inline source map exceeds byte limit"
        );
        let decoded = percent_encoding::percent_decode_str(payload).collect::<Vec<_>>();
        let input = if base64 {
            base64::engine::general_purpose::STANDARD.decode(decoded)?
        } else {
            decoded
        };
        (input, None, source_url)
    } else {
        let url = source_url.join(request)?;
        ensure!(
            url.scheme() == "file" && url.host_str().is_none_or(|host| host == "localhost"),
            "unsupported remote source map link: {request}"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "source map links cannot contain query or fragment"
        );
        let path = url
            .to_file_path()
            .map_err(|()| eyre::eyre!("unsupported source map file URL"))?;
        let canonical = inventory.admit(&path).wrap_err_with(|| {
            format!(
                "source map outside build root or checked linked inventory: {}",
                path.display()
            )
        })?;
        ensure!(
            inventory.contains(&canonical),
            "source map outside build root or checked linked inventory: {}",
            path.display()
        );
        let location =
            Url::from_file_path(&canonical).map_err(|()| eyre::eyre!("invalid source map path"))?;
        (read(&canonical)?, Some(canonical), location)
    };
    ensure!(
        input.len() <= MAX_RECORD_BYTES,
        "upstream source map exceeds byte limit"
    );
    if let Some(previous) = previous {
        if previous.input.matches(&input)
            && previous.linked == linked
            && previous
                .map
                .matches_inputs(source, &input, location.as_str())
        {
            return Ok((Some(previous.clone()), None));
        }
    }
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(".upstream.seg.fb");
    let sidecar = PathBuf::from(sidecar);
    let cached = if mode == CacheMode::Disabled {
        None
    } else {
        File::open(&sidecar).ok().and_then(|file| {
            cache_flatbuffers::SharedBytes::snapshot(file.take(MAX_RECORD_BYTES as u64 + 1))
                .ok()
                .and_then(|bytes| {
                    WireSourceMap::from_backing(bytes, source, &input, location.as_str()).ok()
                })
        })
    };
    let mut warning = None;
    let map = if let Some(map) = cached {
        map
    } else {
        let text = std::str::from_utf8(&input)?;
        let map =
            oxc_sourcemap::SourceMap::from_json_string(text).wrap_err("malformed source map")?;
        let contents = map.get_source_contents().count();
        ensure!(
            contents == 0 || contents == map.get_sources().count(),
            "malformed source map: sourcesContent length mismatch"
        );
        let urls = map
            .get_sources()
            .map(|source| {
                // sourceRoot is a directory prefix, even without its trailing slash.
                // Absolute source URLs override it; original contents are never read.
                if let Ok(absolute) = Url::parse(source) {
                    return Ok(absolute.to_string());
                }
                let request =
                    if let Some(root) = map.get_source_root().filter(|root| !root.is_empty()) {
                        format!(
                            "{root}{}{}",
                            if root.ends_with('/') { "" } else { "/" },
                            source.trim_start_matches('/')
                        )
                    } else {
                        source.to_string()
                    };
                Ok(location.join(&request)?.to_string())
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        let sources = map
            .get_sources()
            .enumerate()
            .map(|(index, original)| MapSource {
                original,
                url: &urls[index],
                content: map.get_source_content(index as u32).map(AsRef::as_ref),
            })
            .collect::<Vec<_>>();
        let names = map.get_names().map(AsRef::as_ref).collect::<Vec<_>>();
        let tokens = map
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
        let encoded = WireSourceMap::encode(
            source,
            &input,
            MapData {
                location: location.as_str(),
                file: map.get_file().map(AsRef::as_ref),
                source_root: map.get_source_root(),
                sources: &sources,
                names: &names,
                tokens: &tokens,
            },
        )?;
        if mode == CacheMode::ReadWrite {
            if let Some(path) = &linked {
                ensure!(
                    path.canonicalize()? == *path && read(path)? == input,
                    "upstream source map changed during cache operation"
                );
            }
            ensure!(
                fs::read(path)? == source.as_bytes(),
                "source changed while loading upstream map"
            );
            if let Err(error) = publish(&sidecar, encoded.bytes()) {
                warning = Some(format!(
                    "{}: source map cache write: {error:#}",
                    path.display()
                ));
            }
        }
        encoded
    };
    let upstream = Upstream {
        map: map
            .snapshot()
            .wrap_err_with(|| format!("snapshotting upstream map for {}", path.display()))?,
        linked,
        input: crate::prepared_text::InputIdentity::new(&input),
    };
    upstream.recheck()?;
    ensure!(
        fs::read(path)? == source.as_bytes(),
        "source changed while loading upstream map"
    );
    Ok((Some(upstream), warning))
}

fn publish(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| eyre::eyre!("missing source map cache parent"))?;
    let mut prefix = path.file_name().unwrap().to_owned();
    prefix.push(".stage-");
    let mut staging = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)?;
    staging.write_all(bytes)?;
    staging.flush()?;
    staging.as_file().sync_all()?;
    staging
        .persist(path)
        .map_err(|error| eyre::eyre!("source map cache replacement failed: {error}"))?;
    Ok(())
}
