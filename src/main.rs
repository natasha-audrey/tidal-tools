mod playlist;
mod tidal;

use std::{env, process::ExitCode};

const HELP: &str = "Usage: tidal-tools <COMMAND>\n\nCommands:\n  playlist <PLAYLIST_URL>  Print each track as Artist(s) - Song\n\n  -h, --help  Show help\n\nRun tidal-tools playlist --help for playlist options.";

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next().as_deref() {
        Some("playlist") => playlist::run(args),
        Some("-h" | "--help") => {
            println!("{HELP}");
            Ok(())
        }
        Some(command) => Err(format!("Unknown command: {command}\n\n{HELP}")),
        None => Err(HELP.into()),
    }
}

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
