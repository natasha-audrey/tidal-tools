# tidal-tools


## Authentication

Add the following to your environment:

```sh
export TIDAL_CLIENT_ID='your-client-id'
export TIDAL_CLIENT_SECRET='your-client-secret'
```

See [Tidal's documentation](https://developer.tidal.com/documentation/api-sdk/api-sdk-manage-apps)
for information on creating a client_id and secret. Ensure these stay secret.

## Building

```sh
cargo build --release
cp target/release/tidal-tools ./tidal-tools
```

or install
```sh
cargo install --path .
```

## Usage

```
Usage: tidal-tools <COMMAND>

Commands:
  playlist  Print each track as Artist(s) - Song
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### Playlist

```
Print each track as Artist(s) - Song, in playlist order.
Set TIDAL_CLIENT_ID and TIDAL_CLIENT_SECRET from your Tidal developer app.
Public playlists only. Multiple artists are separated by commas.

Usage: tidal-tools playlist [OPTIONS] <PLAYLIST_URL>

Arguments:
  <PLAYLIST_URL>
          Full Tidal playlist URL

Options:
      --country <CODE>
          Two-letter country code

          [default: US]

  -h, --help
          Print help (see a summary with '-h')

Redirect output with: tidal-tools playlist 'PLAYLIST_URL' > songs.txt
```


`tidal.com/playlist/...`, `tidal.com/browse/playlist/...`,
`listen.tidal.com/playlist/...`, and `embed.tidal.com/playlists/...` links
are accepted, including sharing query parameters.

This uses Tidal's [official API](https://tidal-music.github.io/tidal-api-reference/)
and [client credentials flow](https://developer.tidal.com/documentation/api-sdk/api-sdk-authorization).
Public playlists only; private playlists require user authorization and are not
supported.

Run checks with `cargo test` and `cargo clippy -- -D warnings`.
