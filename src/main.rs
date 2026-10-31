//! Shadow Realm command-line entry point.

use std::env;
use std::process::ExitCode;

mod app;
mod cli;
mod error;
mod paths;
mod snapshot;
#[cfg(windows)]
mod windows;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let debug = args.first().is_some_and(|arg| arg == "debug");
    let options = match cli::parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{}", if debug { cli::DEBUG_HELP } else { cli::HELP });
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            let help = if debug { "debug --help" } else { "--help" };
            eprintln!("{error:?}\nUse {help} for usage.");
            return ExitCode::from(2);
        }
    };
    match app::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error:?}");
            ExitCode::FAILURE
        }
    }
}
