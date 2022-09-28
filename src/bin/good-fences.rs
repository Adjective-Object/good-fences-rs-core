use clap::Parser;
use good_fences_rs_core::cli::Cli;
use good_fences_rs_core::run_evaluations;

fn main() {
    // set working dir
    let args = Cli::parse();
    run_evaluations(args);
}
