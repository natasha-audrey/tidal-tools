use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use std::{
    io::{self, Write},
    time::Duration,
};

const HELP: &str = "Usage: tidal-tools playlist [--country CODE] <PLAYLIST_URL>\n\nPrint each track as Artist(s) - Song, in playlist order.\nSet TIDAL_CLIENT_ID and TIDAL_CLIENT_SECRET from your Tidal developer app.\nPublic playlists only. Multiple artists are separated by commas.\n\n  --country CODE  Two-letter country code (default: US)\n  -h, --help      Show help\n\nRedirect output with: tidal-tools playlist 'PLAYLIST_URL' > songs.txt";

#[derive(Deserialize)]
pub(crate) struct Artist {
    pub(crate) name: String,
}
#[derive(Deserialize)]
pub(crate) struct Track {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) artists: Vec<Artist>,
    pub(crate) artist: Option<Artist>,
    pub(crate) version: Option<String>,
}
fn playlist_id(input: &str) -> Result<String, String> {
    let url = Url::parse(input).map_err(|_| "Provide a full Tidal playlist URL.".to_string())?;
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(
            url.host_str(),
            Some("tidal.com" | "www.tidal.com" | "listen.tidal.com" | "embed.tidal.com")
        )
    {
        return Err(
            "Expected a playlist URL on tidal.com, listen.tidal.com, or embed.tidal.com.".into(),
        );
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let id = match parts.as_slice() {
        ["playlist" | "playlists", id] | ["browse", "playlist", id] => *id,
        _ => return Err("The URL must point to a playlist, not an album or track.".into()),
    };
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err("The playlist URL contains an invalid playlist ID.".into());
    }
    Ok(id.into())
}
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub(crate) fn format_track(track: Track) -> Result<String, String> {
    let artists = if track.artists.is_empty() {
        track.artist.into_iter().collect()
    } else {
        track.artists
    };
    let names: Vec<_> = artists
        .iter()
        .map(|a| one_line(&a.name))
        .filter(|s| !s.is_empty())
        .collect();
    let mut title = one_line(&track.title);
    if names.is_empty() || title.is_empty() {
        return Err("Tidal returned a track without a title or artist; export aborted.".into());
    }
    if let Some(version) = track.version {
        let version = one_line(&version);
        if !version.is_empty() && !title.ends_with(&format!("({version})")) {
            title.push_str(&format!(" ({version})"));
        }
    }
    Ok(format!("{} - {title}", names.join(", ")))
}
pub fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut input = None;
    let mut country = "US".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "--country" => {
                country = args
                    .next()
                    .ok_or("--country requires a two-letter code.")?
                    .to_ascii_uppercase()
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option: {arg}\n\n{HELP}")),
            _ => {
                if input.replace(arg).is_some() {
                    return Err(format!("Provide one playlist URL.\n\n{HELP}"));
                }
            }
        }
    }
    let id = playlist_id(&input.ok_or(HELP)?)?;
    if country.len() != 2 || !country.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err("--country requires a two-letter code, such as US or GB.".into());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let lines = crate::tidal::playlist(&client, &id, &country)?;
    // Fetch every page before writing, so failures cannot produce partial exports.
    let mut stdout = io::stdout().lock();
    for line in lines {
        if let Err(error) = writeln!(stdout, "{line}") {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(format!("Could not write output: {error}"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "cca16fec-c56e-4c2e-be05-3dfb21eaede6";
    fn track() -> Track {
        Track {
            title: "Song\nTitle".into(),
            artists: vec![
                Artist {
                    name: "Björk".into(),
                },
                Artist {
                    name: "A & B".into(),
                },
            ],
            artist: None,
            version: Some("Live".into()),
        }
    }
    #[test]
    fn accepts_shared_url_forms() {
        for base in [
            "https://tidal.com/playlist",
            "https://listen.tidal.com/playlist",
            "https://tidal.com/browse/playlist",
            "https://embed.tidal.com/playlists",
        ] {
            assert_eq!(playlist_id(&format!("{base}/{ID}?u=123")).unwrap(), ID);
        }
        for url in [
            format!("https://tidal.com.evil.test/playlist/{ID}"),
            format!("https://tidal.com/track/{ID}"),
            "https://tidal.com/playlist/invalid".into(),
        ] {
            assert!(playlist_id(&url).is_err());
        }
    }
    #[test]
    fn formats_all_artists_and_version() {
        assert_eq!(
            format_track(track()).unwrap(),
            "Björk, A & B - Song Title (Live)"
        );
        let mut t = track();
        t.artists.clear();
        t.artist = Some(Artist {
            name: "Solo".into(),
        });
        t.title = "Song (Live)".into();
        assert_eq!(format_track(t).unwrap(), "Solo - Song (Live)");
    }
}
