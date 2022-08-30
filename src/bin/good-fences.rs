extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use std::env::set_current_dir;
use std::path::Path;
use std::time::Instant;

fn main() {
    // set working dir
    let start = Instant::now();
    let root = Path::new("C:\\Users\\Usuario\\client-web");
    assert!(set_current_dir(&root).is_ok());
    println!(
        "Successfully changed working directory to {}!",
        root.display()
    );

    println!("beginning file walks");
    let good_fences_runner =
        GoodFencesRunner::new("tsconfig.paths.json", &vec!["packages", "shared"]);
    // println!("{:#?}", good_fences_runner);
    println!("beginning fence evaluations");
    let violations = good_fences_runner.find_import_violations();
    let elapsed = start.elapsed();

    // Print results and statistics
    println!("Violations: {:#?}", violations);
    println!("Elapsed time since start: {:?}", elapsed);
}
