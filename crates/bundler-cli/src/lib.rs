//! Embeddable clap command and execution entrypoint for the bundler.

mod cli;
mod config;
mod execute;
mod invocation;
mod progress;
mod report;
mod trace;

pub use cli::Cli;

#[cfg(test)]
mod tests;
