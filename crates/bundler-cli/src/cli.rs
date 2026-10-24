use eyre::WrapErr;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Build and publish native bundles, reporting the result as JSON.
///
/// With no positional arguments, reads bundler.config.json in the current directory.
/// This definition can also be embedded as a clap subcommand.
#[derive(Debug, clap::Parser)]
#[command(name = "bundler-cli", version, about)]
pub struct Cli {
    /// Project configuration file (defaults to cwd's bundler.config.json).
    #[arg(long, value_name = "FILE", conflicts_with_all = ["fixture_config", "root", "output", "entries"])]
    pub config: Option<PathBuf>,

    /// Build settings for positional/fixture mode, without project paths.
    #[arg(long, value_name = "FILE", requires = "root")]
    pub fixture_config: Option<PathBuf>,

    /// Write a new Chrome/Perfetto trace file; relative to the current directory.
    #[arg(long, value_name = "FILE")]
    pub trace_file: Option<PathBuf>,

    /// Override the configured CSS loader with automatic DOM mounting.
    #[arg(long)]
    pub css_auto_mount: bool,

    /// Input project root (positional mode only).
    #[arg(value_name = "ROOT", requires = "output")]
    pub root: Option<PathBuf>,

    /// Output directory, relative to the current directory.
    #[arg(value_name = "OUTPUT", requires = "root")]
    pub output: Option<PathBuf>,

    /// Entry paths, relative to ROOT.
    #[arg(value_name = "ENTRY", requires = "output")]
    pub entries: Vec<PathBuf>,
}

impl Cli {
    /// Run using the process working directory and write one JSON result on success.
    pub fn run(self, stdout: &mut impl Write) -> eyre::Result<()> {
        self.run_from(
            &std::env::current_dir().wrap_err("reading CLI working directory")?,
            stdout,
        )
    }

    /// Run relative to an explicit working directory, without changing process state.
    pub fn run_from(self, cwd: &Path, stdout: &mut impl Write) -> eyre::Result<()> {
        let _ = crate::trace::uptime();
        let invocation = crate::invocation::Invocation::resolve(self, cwd)
            .wrap_err_with(|| format!("resolving bundler invocation from {}", cwd.display()))?;
        crate::trace::record(
            &invocation.root,
            &invocation.output,
            invocation.trace_file.as_deref(),
            || crate::execute::build(&invocation, stdout),
        )
    }
}
