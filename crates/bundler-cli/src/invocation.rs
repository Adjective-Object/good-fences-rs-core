use std::path::{Path, PathBuf};

use eyre::WrapErr;

use crate::{config::Config, Cli};

pub(crate) struct Invocation {
    pub root: PathBuf,
    pub output: PathBuf,
    pub entries: Vec<PathBuf>,
    pub trace_file: Option<PathBuf>,
    pub css_auto_mount: bool,
    pub locale: Option<String>,
    pub config: ConfigSource,
}

pub(crate) enum ConfigSource {
    Loaded(Box<Config>),
    Fixture(PathBuf),
}

impl ConfigSource {
    pub fn load(&self) -> eyre::Result<Config> {
        match self {
            Self::Loaded(config) => Ok(config.as_ref().clone()),
            Self::Fixture(path) => {
                let config = Config::read(path)?;
                eyre::ensure!(
                    config.root.is_none()
                        && config.outdir.is_none()
                        && config.entries.is_none()
                        && config.trace_file.is_none(),
                    "project paths require --config, not --fixture-config"
                );
                Ok(config)
            }
        }
    }
}

impl Invocation {
    pub fn resolve(cli: Cli, cwd: &Path) -> eyre::Result<Self> {
        eyre::ensure!(cwd.is_absolute(), "working directory must be absolute");
        let Cli {
            config,
            fixture_config,
            trace_file,
            css_auto_mount,
            locale,
            root,
            output,
            entries,
        } = cli;
        let trace_file = trace_file.map(|path| cwd.join(path));
        // Validate here too: callers embedding Cli may construct it without clap.
        if let (None, Some(root)) = (&config, &root) {
            Ok(Self {
                root: cwd.join(root),
                output: cwd.join(output.ok_or_else(|| eyre::eyre!("missing output directory"))?),
                entries,
                trace_file,
                css_auto_mount,
                locale,
                config: fixture_config.map_or_else(
                    || ConfigSource::Loaded(Box::default()),
                    |path| ConfigSource::Fixture(cwd.join(path)),
                ),
            })
        } else {
            eyre::ensure!(
                root.is_none() && output.is_none() && entries.is_empty() && fixture_config.is_none(),
                "project configuration cannot be combined with positional arguments or --fixture-config"
            );
            let filename = cwd.join(config.unwrap_or_else(|| "bundler.config.json".into()));
            let filename = filename
                .canonicalize()
                .wrap_err_with(|| format!("opening configuration {}", filename.display()))?;
            let mut config = Config::read(&filename)?;
            let entries = config
                .entries
                .take()
                .filter(|entries| !entries.is_empty())
                .ok_or_else(|| eyre::eyre!("config requires nonempty entries"))?;
            let root = filename
                .parent()
                .expect("canonical config has a parent")
                .join(config.root.as_deref().unwrap_or(Path::new(".")));
            let root = root.canonicalize().wrap_err_with(|| {
                format!(
                    "resolving configured build root {} from {}",
                    root.display(),
                    filename.display()
                )
            })?;
            Ok(Self {
                output: root.join(
                    config
                        .outdir
                        .as_deref()
                        .unwrap_or(Path::new(".bundler-out")),
                ),
                entries,
                trace_file: trace_file.or_else(|| config.trace_file.as_ref().map(|p| root.join(p))),
                root,
                css_auto_mount,
                locale,
                config: ConfigSource::Loaded(Box::new(config)),
            })
        }
    }
}
