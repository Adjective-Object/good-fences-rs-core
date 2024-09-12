extern crate serde_json;
extern crate unused_finder;

use anyhow::{Context, Result};
use clap::Parser;
use std::{env, fs, path::Path};

#[derive(Parser, Debug)]
struct CliArgs {
    #[arg(short, long, default_value = None)]
    config_path: Option<String>,
    #[arg(short, long, default_value_t = false)]
    // If this flag is set, ignore all other options and
    // try to stack dump the parent process. Used internally
    // for dumping stacks
    rstack: std::primitive::bool,
    // If this flag is set, run a timer to dump stacks every 5s
    #[arg(short = 'd', long, default_value_t = false)]
    dump_stacks: std::primitive::bool,
    // If this flag is set, run the parking_lot deadlock detector
    #[arg(short = 'D', long, default_value_t = true)]
    deadlock_detector: std::primitive::bool,
}

const DEFAULT_CONFIG_PATH: &str = "unused-finder.json";

fn start_deadlock_detector() {
    // only for #[cfg]
    use parking_lot::deadlock;
    use std::thread;
    use std::time::Duration;

    // Create a background thread which checks for deadlocks every 10s
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(10));
        let deadlocks = deadlock::check_deadlock();
        if deadlocks.is_empty() {
            continue;
        }

        println!("{} deadlocks detected", deadlocks.len());
        for (i, threads) in deadlocks.iter().enumerate() {
            println!("Deadlock #{}", i);
            for t in threads {
                println!("Thread Id {:#?}", t.thread_id());
                println!("{:#?}", t.backtrace());
            }
        }
    });
}

fn start_stackdump_timer() {
    // only for #[cfg]
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    // Create a background thread which checks for deadlocks every 10s
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));

        let exe = env::current_exe().unwrap();
        let trace = match rstack_self::trace(Command::new(exe).arg("--rstack")) {
            Ok(x) => x,
            Err(e) => {
                println!("failed to spawn rtrace sub-process: {e}");
                return;
            }
        };

        println!(
            "\n\n\n\n\n\n\n\n\n\n\n\nThread dump contains {} threads:",
            trace.threads().len()
        );
        for thread in trace.threads() {
            println!("Thread {}", thread.id());
            for (i, frame) in thread.frames().iter().enumerate() {
                print!("  {:>4}:", i);
                for symbol in frame.symbols() {
                    print!(
                        "      {} {}:{}",
                        symbol.name().unwrap_or("<unknown_symbol>"),
                        symbol
                            .file()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or("<unknown_file>".to_string()),
                        symbol
                            .line()
                            .map(|l| l.to_string())
                            .unwrap_or("<??>".to_string()),
                    );
                    println!();
                }
            }
        }
    });
}

fn main() -> Result<()> {
    let args = CliArgs::parse();
    if args.rstack {
        let _ = rstack_self::child();
        return Ok(());
    }

    if args.deadlock_detector {
        start_deadlock_detector();
    }
    if args.dump_stacks {
        start_stackdump_timer();
    }

    let config_path = args.config_path.unwrap_or_else(|| {
        println!("No config file path provided, using default config file path");
        DEFAULT_CONFIG_PATH.to_string()
    });

    println!("reading config from path {config_path}");

    // read and parse the config file
    let config_str = fs::read_to_string(&config_path).expect("Failed to read config file");
    let mut config: unused_finder::FindUnusedItemsConfig = serde_json::from_str(&config_str)
        .with_context(|| format!("Parsing unused-finder config {config_path}"))?;

    // move the the working directory of the config path
    let config_dir = Path::new(&config_path)
        .parent()
        .expect("Failed to get parent directory of config file")
        .to_path_buf();
    config.ts_config_path = config_dir
        .join("tsconfig.json")
        .to_str()
        .with_context(|| "Failed to coerce unprintable directory to a string!".to_string())?
        .to_string();

    println!("working in {}..", config_dir.display());
    env::set_current_dir(&config_dir)
        .expect("Failed to change working directory to config file directory");

    let start_time = std::time::Instant::now();
    let result = unused_finder::find_unused_items(config)?;
    let delta = start_time.elapsed();
    println!("result ({}ms):\n{result}", delta.as_millis());

    Ok(())
}
