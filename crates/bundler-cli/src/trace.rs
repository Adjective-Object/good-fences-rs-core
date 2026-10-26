use eyre::WrapErr;
use std::{
    io::IsTerminal,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};
use tracing_subscriber::fmt::{format, time::Uptime};
use tracing_subscriber::prelude::*;

pub(crate) fn uptime() -> Uptime {
    static START: OnceLock<Instant> = OnceLock::new();
    Uptime::from(*START.get_or_init(Instant::now))
}

fn log_format(ansi: bool) -> format::Format<format::Full, Uptime> {
    format()
        .with_timer(uptime())
        .with_target(false)
        .with_ansi(ansi)
}

pub(crate) fn record<T>(
    root: &Path,
    output: &Path,
    trace_file: Option<&Path>,
    execute: impl FnOnce() -> eyre::Result<T>,
) -> eyre::Result<T> {
    let interactive = std::io::stderr().is_terminal();
    let progress = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(log_format(interactive))
        .with_ansi(interactive)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.is_event()
                && matches!(
                    metadata.target(),
                    "bundler::progress" | "bundler::progress::discovery" | "bundler::diagnostic"
                )
        }));
    let (chrome, guard) = if let Some(path) = trace_file {
        let file = open_trace(root, output, path)
            .wrap_err_with(|| format!("opening local build trace {}", path.display()))?;
        let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
            .writer(file)
            .include_args(true)
            .include_locations(false)
            .trace_style(tracing_chrome::TraceStyle::Threaded)
            .build();
        // Only these audited span targets are recorded; caller data and unrelated
        // library events must not leak into a profiling artifact.
        let layer = layer.with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.is_span()
                && (metadata.target() == "bundler"
                    || metadata.target().starts_with("bundler::")
                    || metadata.target() == "segment_cache"
                    || metadata.target().starts_with("segment_cache::"))
        }));
        (Some(layer), Some(guard))
    } else {
        (None, None)
    };
    let dispatch = tracing::Dispatch::new(
        tracing_subscriber::registry()
            .with(progress)
            .with(crate::progress::LockfileProgress::stderr().with_filter(
                tracing_subscriber::filter::filter_fn(|metadata| {
                    metadata.is_event() && metadata.target() == "bundler::progress::lockfile"
                }),
            ))
            .with(chrome),
    );
    let result = tracing::dispatcher::with_default(&dispatch, execute);
    // Drop after all worker joins and span exits, on errors as well as success.
    drop(dispatch);
    drop(guard);
    result
}

fn absolute_destination(path: &Path) -> eyre::Result<PathBuf> {
    eyre::ensure!(
        !path.components().any(|c| c == Component::ParentDir),
        "parent traversal is not supported"
    );
    let absolute = std::path::absolute(path)
        .wrap_err_with(|| format!("resolving destination {}", path.display()))?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing
        .try_exists()
        .wrap_err_with(|| format!("checking destination ancestor {}", existing.display()))?
    {
        missing.push(
            existing
                .file_name()
                .ok_or_else(|| eyre::eyre!("invalid destination"))?,
        );
        existing = existing
            .parent()
            .ok_or_else(|| eyre::eyre!("invalid destination"))?;
    }
    let mut canonical = existing
        .canonicalize()
        .wrap_err_with(|| format!("canonicalizing destination ancestor {}", existing.display()))?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

pub(crate) fn open_trace(root: &Path, output: &Path, path: &Path) -> eyre::Result<std::fs::File> {
    let root = root
        .canonicalize()
        .wrap_err_with(|| format!("resolving trace input boundary {}", root.display()))?;
    let output = absolute_destination(output).wrap_err("resolving trace output boundary")?;
    eyre::ensure!(
        !path.components().any(|c| c == Component::ParentDir),
        "trace path must not contain parent traversal"
    );
    let name = path
        .file_name()
        .ok_or_else(|| eyre::eyre!("missing trace filename"))?;
    eyre::ensure!(
        !name.to_string_lossy().contains(':'),
        "trace filename must not name an alternate data stream"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .wrap_err_with(|| format!("resolving trace parent {}", parent.display()))?;
    let destination = parent.join(name);
    eyre::ensure!(
        !destination.starts_with(&output),
        "trace must be outside the output directory"
    );
    eyre::ensure!(
        !destination.starts_with(&root)
            || (parent == root && name.to_string_lossy().starts_with(".bundler-trace-")),
        "trace must be outside the build root, or a root-level .bundler-trace-* file"
    );
    eyre::ensure!(
        !parent
            .ancestors()
            .any(|p| p.join(".bundler-current.json").exists()
                || p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".bundler-generation-"))),
        "trace must not modify a published output tree"
    );
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .wrap_err_with(|| format!("creating trace {}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn messages(ansi: bool) -> String {
        let output = Output::default();
        let writer = output.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(log_format(ansi))
                .with_ansi(ansi)
                .with_writer(move || writer.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("info message");
            tracing::warn!("warning message");
            tracing::error!("error message");
        });
        let bytes = output.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    fn elapsed(line: &str) -> f64 {
        line.split_whitespace()
            .next()
            .unwrap()
            .strip_suffix('s')
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn scoped_loggers_share_uptime_and_redirected_prefixes_are_plain() {
        let first = messages(false);
        for (line, level) in first.lines().zip(["INFO", "WARN", "ERROR"]) {
            assert!(elapsed(line).is_finite(), "{line}");
            assert_eq!(line.split_whitespace().nth(1), Some(level), "{line}");
        }
        assert_eq!(first.lines().count(), 3);
        assert!(!first.contains('\u{1b}'), "{first}");
        std::thread::sleep(std::time::Duration::from_millis(10));
        let later = messages(false);
        assert!(
            elapsed(later.lines().next().unwrap()) - elapsed(first.lines().next().unwrap())
                >= 0.009
        );
    }

    #[test]
    fn interactive_prefixes_use_level_specific_colours() {
        let text = messages(true);
        assert_eq!(text.lines().count(), 3);
        for (line, (level, colour)) in text.lines().zip([
            ("INFO", "\u{1b}[32m"),
            ("WARN", "\u{1b}[33m"),
            ("ERROR", "\u{1b}[31m"),
        ]) {
            assert!(line.contains(level) && line.contains(colour), "{line:?}");
            assert!(line.contains("\u{1b}[0m"), "{line:?}");
        }
    }
}
