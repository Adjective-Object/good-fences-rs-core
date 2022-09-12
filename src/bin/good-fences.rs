extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use std::env::set_current_dir;
use std::path::Path;
use std::time::Instant;

fn main() {
    // set working dir
    let start = Instant::now();
    let root = Path::new("../client-web");
    // let mut file = File::create("./good-fences-errors.log").unwrap();
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

    // file.write(format!("{:?}", violations).as_bytes());
    // Print results and statistics
    println!("Violations: {:#?}", violations);
    println!("Total violations: {}", violations.len());
    println!("Elapsed time since start: {:?}", elapsed);
}
