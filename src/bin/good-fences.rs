extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use std::env::set_current_dir;
use std::path::Path;

fn main() {
    // set working dir
    let root = Path::new("/home/adjective/Projects/client-web");
    assert!(set_current_dir(&root).is_ok());
    println!(
        "Successfully changed working directory to {}!",
        root.display()
    );

    println!("beginning file walks");
    let good_fences_runner =
        GoodFencesRunner::new("tsconfig.paths.json", &vec!["packages", "shared"]);

    println!("beginning fence evaluations");
    let violations = good_fences_runner.evaluate_fences();

    // print some junk
    println!("Violations: {:#?}", violations)
}
