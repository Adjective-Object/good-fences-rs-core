use clap::Parser;

fn main() -> eyre::Result<()> {
    bundler_cli::Cli::parse().run(&mut std::io::stdout().lock())
}
