use clap::Parser;
use good_fences_rs_core::cli::Cli;
use good_fences_rs_core::good_fences;

fn main() {
    // set working dir
    let args = Cli::parse();
    good_fences(args.paths, args.project, args.base_url, Some(args.output));
}
