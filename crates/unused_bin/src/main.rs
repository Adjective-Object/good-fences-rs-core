extern crate serde_json;
extern crate unused_finder;

use std::{env, fs, path::Path};

use clap::Parser;

#[derive(Parser, Debug)]
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

    println!("reading config from path {config_path}");

    // read and parse the config file
    let config_str = fs::read_to_string(&config_path).expect("Failed to read config file");
    let config: unused_finder::FindUnusedItemsConfig =
        serde_json::from_str(&config_str).expect("Failed to parse unused-finder config");

    // move the the working directory of the config path
    let config_dir = Path::new(&config_path)
        .parent()
        .expect("Failed to get parent directory of config file")
        .to_path_buf();
    println!("working in {}..", config_dir.display());
    env::set_current_dir(&config_dir)
        .expect("Failed to change working directory to config file directory");

    let start_time = std::time::Instant::now();
    let result = unused_finder::find_unused_items(config).unwrap();
    let delta = start_time.elapsed();
    println!("result ({}ms):\n{}", delta.as_millis(), result);
    

    return ();
}
