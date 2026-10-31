//! Shadow Realm command-line entry point.

use std::env;
use std::process::ExitCode;

#[cfg(windows)]
mod app;
#[cfg(not(windows))]
mod app {
    pub(crate) fn run(_options: &crate::cli::Options) -> anyhow::Result<()> {
        anyhow::bail!("This experiment requires Windows. No operation was performed.")
    }
}
mod cli;
mod error;
mod paths;
mod snapshot;
#[cfg(windows)]
mod windows;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let options = match cli::parse_args(&args) {
        Ok(options) => options,
        Err(error) => error.exit(),
    };
    match app::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error:?}");
            ExitCode::FAILURE
        }
    }
}
