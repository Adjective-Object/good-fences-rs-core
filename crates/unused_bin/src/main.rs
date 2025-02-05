extern crate unused_finder;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use logger::{Logger, StdioLogger};
use std::{convert::TryInto, env, fs, io::Write, path::Path};
use unused_finder::UnusedFinderConfig;

#[derive(Parser, Debug)]
struct CliArgs {
    #[arg(short, long, default_value = None, alias = "config")]
    config_path: Option<String>,
    #[arg(short, long, default_value_t = false)]
    // If this flag is set, ignore all other options and
    // try to stack dump the parent process. Used internally
    // for dumping stacks
    rstack: std::primitive::bool,
    // If this flag is set, run a timer to dump stacks every 5s
    #[cfg(feature = "rstack")]
    #[arg(short = 'd', long, default_value_t = false)]
    dump_stacks: std::primitive::bool,
    // If this flag is set, run the parking_lot deadlock detector
    #[arg(short = 'D', long, default_value_t = true)]
    deadlock_detector: std::primitive::bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generates a dot graph of the dependency graph
    Graph {
        #[arg(short = 'f', alias = "filter")]
        filter: Option<String>,
    },
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

#[cfg(feature = "rstack")]
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
    let logger = &StdioLogger::new();

    let args = CliArgs::parse();
    #[cfg(feature = "rstack")]
    if args.rstack {
        let _ = rstack_self::child();
        return Ok(());
    }

    if args.deadlock_detector {
        start_deadlock_detector();
    }
    #[cfg(feature = "rstack")]
    if args.dump_stacks {
        start_stackdump_timer();
    }

    let config_path = args.config_path.unwrap_or_else(|| {
        logger.log("No config file path provided, using default config file path");
        DEFAULT_CONFIG_PATH.to_string()
    });
    logger.log(format!("reading config from path {config_path}"));
    // read and parse the config file
    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("reading config file {}", &config_path))?;
    let config: unused_finder::UnusedFinderJSONConfig = serde_hjson::from_str(&config_str)
        .with_context(|| format!("Parsing unused-finder config {config_path}"))?;
    let mut parsed_config: UnusedFinderConfig = config.try_into()?;
    // HACK: if the repo_root is not an absolute path, make it relative to the config file
    if !Path::new(&parsed_config.repo_root).is_absolute() {
        parsed_config.repo_root = Path::new(&config_path)
            .parent()
            .expect("Failed to get parent directory of config file")
            .join(&parsed_config.repo_root)
            .to_string_lossy()
            .to_string();
    }

    // move the the working directory of the config path
    let config_dir = Path::new(&config_path)
        .parent()
        .expect("Failed to get parent directory of config file")
        .to_path_buf();

    logger.log(format!("working in {}..", config_dir.display()));
    env::set_current_dir(&config_dir)
        .expect("Failed to change working directory to config file directory");

    let mut unused_finder = unused_finder::UnusedFinder::new_from_cfg(logger, parsed_config)?;
    let result: unused_finder::UnusedFinderResult = unused_finder.find_unused(logger)?;
    let report = result.get_report();
    logger.log(format!("result:\n{report}"));

    match &args.command {
        Some(Commands::Graph { filter }) => {
            println!("Generating graph.dot file...");
            let file = std::fs::File::create("graph.dot").expect("Failed to create graph.dot");
            let mut stream = std::io::BufWriter::new(file);
            result.write_dot_graph(logger, filter.as_ref().map(|x| x.as_str()), &mut stream)?;
            stream.flush().expect("Failed to flush graph.dot");
            println!("Done!");
        }
        None => {}
    }

    logger.log("done!");
    Ok(())
}
