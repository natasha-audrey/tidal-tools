mod playlist;
mod tidal;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print each track as Artist(s) - Song
    #[command(long_about = "
Print each track as Artist(s) - Song, in playlist order.
Set TIDAL_CLIENT_ID and TIDAL_CLIENT_SECRET from your Tidal developer app.
Public playlists only. Multiple artists are separated by commas.")]
    Playlist(playlist::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Playlist(args) => playlist::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn playlist_country_defaults_and_normalization() {
        for (args, expected) in [
            (vec!["tidal-tools", "playlist", "URL"], "US"),
            (
                vec!["tidal-tools", "playlist", "--country", "gb", "URL"],
                "GB",
            ),
            (vec!["tidal-tools", "playlist", "URL", "--country=ca"], "CA"),
        ] {
            let Command::Playlist(args) = Cli::try_parse_from(args).unwrap().command;
            assert_eq!(args.playlist_url, "URL");
            assert_eq!(args.country, expected);
        }
    }

    #[test]
    fn rejects_invalid_arguments() {
        for args in [
            vec!["tidal-tools"],
            vec!["tidal-tools", "unknown"],
            vec!["tidal-tools", "playlist"],
            vec!["tidal-tools", "playlist", "URL", "extra"],
            vec!["tidal-tools", "playlist", "URL", "--unknown"],
            vec!["tidal-tools", "playlist", "URL", "--country"],
            vec!["tidal-tools", "playlist", "URL", "--country", "USA"],
            vec!["tidal-tools", "playlist", "URL", "--country", "12"],
            vec!["tidal-tools", "playlist", "URL", "--country", "é"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn help_does_not_require_a_playlist() {
        for args in [
            vec!["tidal-tools", "--help"],
            vec!["tidal-tools", "playlist", "--help"],
        ] {
            assert_eq!(
                Cli::try_parse_from(args).err().unwrap().kind(),
                ErrorKind::DisplayHelp
            );
        }
    }
}
