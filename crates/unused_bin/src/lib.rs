use clap::Parser
use anyhow::{Context, Result};

#[Parser, Debug]
struct CliArgs {
    #[arg(short, long, default_value = None)]
    config_path: Option<String>,
}

const DEFAULT_CONFIG_PATH: &'static str = "unused-finder.json";

fn main() {
    let args = CliArgs::parse();
    let config_path = args.config_path.unwrap_or_else(|| {
        println!("No config file path provided, using default config file path");
        DEFAULT_CONFIG_PATH.to_string()
    });

    // read the config file
    let config = match fs::read_to_string(&config_path).with_context(|| {
        format!("Failed to read config file: {}", config_path)
    }).unwrap()
}