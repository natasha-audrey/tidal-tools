use crate::playlist::{Artist, Track, format_track};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    env,
    time::{Duration, SystemTime},
};

const API: &str = "https://openapi.tidal.com/v2/";
const AUTH: &str = "https://auth.tidal.com/v1/oauth2/token";

#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
}

fn credentials(name: &str) -> Result<String, String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty()).ok_or_else(|| {
        format!("Set {name} using your app credentials from https://developer.tidal.com (see README.md).")
    })
}

fn rate_limit_error(response: &reqwest::blocking::Response) -> String {
    match response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => format!(
            "Tidal rate limit reached (HTTP 429). Retry-After: {value}. Please retry after the indicated delay (seconds) or date."
        ),
        None => "Tidal rate limit reached (HTTP 429). No Retry-After header provided. Please try again later.".into(),
    }
}

const MAX_RATE_LIMIT_RETRIES: usize = 5;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

fn retry_delay(value: Option<&str>, retry: usize, now: SystemTime) -> Duration {
    let server_delay = value
        .and_then(|value| {
            let value = value.trim();
            value
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or_else(|| {
                    httpdate::parse_http_date(value)
                        .ok()
                        .map(|date| date.duration_since(now).unwrap_or(Duration::ZERO))
                })
        })
        .unwrap_or(Duration::ZERO);
    server_delay.max(Duration::from_secs(2 << retry.min(4)))
}

fn retry_rate_limits<T>(
    mut send: impl FnMut() -> Result<T, String>,
    mut limited: impl FnMut(&T) -> Option<(Duration, String)>,
    mut wait: impl FnMut(Duration),
) -> Result<T, String> {
    for retry in 0..=MAX_RATE_LIMIT_RETRIES {
        let response = send()?;
        let Some((server_delay, error)) = limited(&response) else {
            return Ok(response);
        };
        if retry == MAX_RATE_LIMIT_RETRIES {
            return Err(format!(
                "{error} Stopped after {MAX_RATE_LIMIT_RETRIES} automatic retries."
            ));
        }
        let delay = server_delay.max(retry_delay(None, retry, SystemTime::now()));
        if delay > MAX_RETRY_WAIT {
            return Err(format!(
                "{error} Requested wait exceeds the 60-second automatic retry limit."
            ));
        }
        eprintln!(
            "Tidal rate limit reached (HTTP 429). Waiting {:.1} seconds before retry {}/{}.",
            delay.as_secs_f64(),
            retry + 1,
            MAX_RATE_LIMIT_RETRIES
        );
        drop(response);
        wait(delay);
    }
    unreachable!()
}

fn send_with_backoff(
    send: impl FnMut() -> Result<reqwest::blocking::Response, String>,
) -> Result<reqwest::blocking::Response, String> {
    retry_rate_limits(
        send,
        |response| {
            (response.status().as_u16() == 429).then(|| {
                let delay = retry_delay(
                    response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()),
                    0,
                    SystemTime::now(),
                );
                (delay, rate_limit_error(response))
            })
        },
        std::thread::sleep,
    )
}

fn issue_token(client: &Client, endpoint: &str, id: &str, secret: &str) -> Result<String, String> {
    let response = send_with_backoff(|| {
        client
            .post(endpoint)
            .basic_auth(id, Some(secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .map_err(|_| "Could not reach Tidal's token service.".to_string())
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Tidal token request failed (HTTP {}). Check TIDAL_CLIENT_ID and TIDAL_CLIENT_SECRET.",
            response.status().as_u16()
        ));
    }
    let token: Token = response
        .json()
        .map_err(|_| "Could not read Tidal's token response.".to_string())?;
    if token.access_token.trim().is_empty() || !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err("Tidal returned an invalid access token.".into());
    }
    Ok(token.access_token)
}

struct Api<'a> {
    client: &'a Client,
    id: String,
    secret: String,
    token: String,
}

impl Api<'_> {
    fn get(&mut self, url: Url) -> Result<Value, String> {
        for attempt in 0..2 {
            let response = send_with_backoff(|| {
                self.client
                    .get(url.clone())
                    .bearer_auth(&self.token)
                    .header("Accept", "application/vnd.api+json")
                    .send()
                    .map_err(|_| "Could not reach Tidal.".to_string())
            })?;
            match response.status().as_u16() {
                200 => return response.json().map_err(|_| "Could not read Tidal's API response.".into()),
                401 if attempt == 0 => self.token = issue_token(self.client, AUTH, &self.id, &self.secret)?,
                401 | 403 => return Err("Tidal denied access. Check your developer app credentials and ensure the playlist is public.".into()),
                404 => return Err("Playlist or track not found or unavailable in this country.".into()),
                status => return Err(format!("Tidal returned HTTP {status}. Please try again later.")),
            }
        }
        unreachable!()
    }
}

fn malformed() -> String {
    "Tidal returned incomplete playlist metadata; export aborted.".into()
}

// Resolve only a cursor from next links. Never send credentials to a supplied URL.
fn next_cursor(doc: &Value) -> Result<Option<String>, String> {
    let links = doc
        .get("links")
        .and_then(Value::as_object)
        .ok_or_else(malformed)?;
    match links.get("next") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(next)) if !next.is_empty() => {
            let url = Url::parse(API)
                .unwrap()
                .join(next)
                .map_err(|_| malformed())?;
            url.query_pairs()
                .find(|(key, _)| key == "page[cursor]")
                .map(|(_, value)| Some(value.into_owned()))
                .ok_or_else(malformed)
        }
        _ => Err(malformed()),
    }
}

fn track_line(doc: &Value) -> Result<String, String> {
    let data = &doc["data"];
    let mut track: Track =
        serde_json::from_value(data["attributes"].clone()).map_err(|_| malformed())?;
    let artists = &data["relationships"]["artists"];
    // A truncated artist relationship must not silently omit contributors.
    if artists["links"]["next"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(
            "Tidal returned a paginated artist list; export aborted to avoid missing artists."
                .into(),
        );
    }
    let included = doc["included"].as_array().ok_or_else(malformed)?;
    for artist in artists["data"].as_array().ok_or_else(malformed)? {
        let id = artist["id"].as_str().ok_or_else(malformed)?;
        let resource = included
            .iter()
            .find(|r| r["type"] == "artists" && r["id"].as_str() == Some(id))
            .ok_or_else(malformed)?;
        track.artists.push(Artist {
            name: resource["attributes"]["name"]
                .as_str()
                .ok_or_else(malformed)?
                .into(),
        });
    }
    format_track(track)
}

fn collect_playlist(
    mut page: impl FnMut(Option<&str>) -> Result<Value, String>,
    mut track: impl FnMut(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut cache = HashMap::new();
    let mut cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let doc = page(cursor.as_deref())?;
        for item in doc["data"].as_array().ok_or_else(malformed)? {
            match item["type"].as_str() {
                Some("videos") => continue,
                Some("tracks") => {}
                _ => return Err(malformed()),
            }
            let id = item["id"].as_str().ok_or_else(malformed)?;
            if !cache.contains_key(id) {
                cache.insert(id.to_string(), track(id)?);
            }
            lines.push(cache[id].clone());
        }
        cursor = next_cursor(&doc)?;
        match &cursor {
            None => return Ok(lines),
            Some(next) if !cursors.insert(next.clone()) => {
                return Err("Tidal repeated a pagination cursor; export aborted.".into());
            }
            _ => {}
        }
    }
}

pub(crate) fn playlist(client: &Client, id: &str, country: &str) -> Result<Vec<String>, String> {
    let client_id = credentials("TIDAL_CLIENT_ID")?;
    let secret = credentials("TIDAL_CLIENT_SECRET")?;
    let token = issue_token(client, AUTH, &client_id, &secret)?;
    let api = std::cell::RefCell::new(Api {
        client,
        id: client_id,
        secret,
        token,
    });
    collect_playlist(
        |cursor| {
            let mut url = Url::parse(&format!("{API}playlists/{id}/relationships/items")).unwrap();
            url.query_pairs_mut()
                .append_pair("countryCode", country)
                .append_pair("sort", "itemIndex");
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("page[cursor]", cursor);
            }
            api.borrow_mut().get(url)
        },
        |track_id| {
            let mut url = Url::parse(API).unwrap();
            url.path_segments_mut()
                .unwrap()
                .pop_if_empty()
                .push("tracks")
                .push(track_id);
            url.query_pairs_mut()
                .append_pair("countryCode", country)
                .append_pair("include", "artists");
            track_line(&api.borrow_mut().get(url)?)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn honors_retry_after_seconds_and_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let date = httpdate::fmt_http_date(now + Duration::from_secs(12));
        assert_eq!(retry_delay(Some("4"), 0, now).as_secs(), 4);
        assert_eq!(retry_delay(Some(&date), 0, now).as_secs(), 12);
        for value in [None, Some("invalid"), Some("0")] {
            assert_eq!(retry_delay(value, 2, now).as_secs(), 8);
        }
        assert_eq!(retry_delay(Some("120"), 0, now).as_secs(), 120);
    }

    #[test]
    fn retries_same_operation_then_returns_success() {
        let mut attempts = 0;
        let mut waits = Vec::new();
        let result = retry_rate_limits(
            || {
                attempts += 1;
                Ok(if attempts < 4 { 429 } else { 200 })
            },
            |status| (*status == 429).then(|| (Duration::from_secs(4), "limited".into())),
            |delay| waits.push(delay.as_secs()),
        )
        .unwrap();
        assert_eq!(result, 200);
        assert_eq!(attempts, 4);
        assert_eq!(waits, [4, 4, 8]);
    }

    #[test]
    fn bounds_retries_and_does_not_shorten_long_server_delays() {
        for (server_delay, expected_waits) in [(4, vec![4, 4, 8, 16, 32]), (120, vec![])] {
            let mut waits = Vec::new();
            let result = retry_rate_limits(
                || Ok(429),
                |_| {
                    Some((
                        Duration::from_secs(server_delay),
                        "Retry-After supplied".into(),
                    ))
                },
                |delay| waits.push(delay.as_secs()),
            );
            assert!(result.unwrap_err().contains("Retry-After supplied"));
            assert_eq!(waits, expected_waits);
        }
        assert_eq!(
            retry_rate_limits(|| Ok(404), |_| None, |_| panic!("unexpected wait")).unwrap(),
            404
        );
        assert!(
            retry_rate_limits::<()>(
                || Err("network".into()),
                |_| panic!("unexpected response"),
                |_| panic!("unexpected wait")
            )
            .is_err()
        );
    }

    #[test]
    fn preserves_order_duplicates_and_paginates_past_videos() {
        let mut requests = Vec::new();
        let lines = collect_playlist(|cursor| Ok(match cursor {
            None => json!({"data": [{"type":"videos","id":"v"}], "links":{"next":"/playlists/p/relationships/items?page%5Bcursor%5D=a%2Bb"}}),
            Some("a+b") => json!({"data": [{"type":"tracks","id":"2"},{"type":"tracks","id":"1"},{"type":"tracks","id":"2"}], "links":{}}),
            _ => panic!("wrong cursor"),
        }), |id| { requests.push(id.to_string()); Ok(format!("Artist - {id}")) }).unwrap();
        assert_eq!(lines, ["Artist - 2", "Artist - 1", "Artist - 2"]);
        assert_eq!(requests, ["2", "1"]);
    }

    #[test]
    fn rejects_failed_missing_and_looping_pages() {
        for doc in [
            json!({"links":{}}),
            json!({"data":[]}),
            json!({"data":[],"links":{"next":"?page[cursor]=again"}}),
        ] {
            assert!(collect_playlist(|_| Ok(doc.clone()), |_| unreachable!()).is_err());
        }
        assert!(collect_playlist(|_| Err("network failure".into()), |_| unreachable!()).is_err());
        assert!(
            collect_playlist(|_| Ok(json!({"data":[],"links":{}})), |_| unreachable!())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn resolves_artists_by_relationship_order_and_rejects_missing_metadata() {
        let mut doc = json!({"data":{"attributes":{"title":"Song","version":"Live"}, "relationships":{"artists":{"data":[{"id":"2"},{"id":"1"}]}}}, "included":[{"type":"artists","id":"1","attributes":{"name":"One"}},{"type":"artists","id":"2","attributes":{"name":"Two"}}]});
        assert_eq!(track_line(&doc).unwrap(), "Two, One - Song (Live)");
        doc["included"].as_array_mut().unwrap().pop();
        assert!(track_line(&doc).is_err());
    }

    #[test]
    fn exchanges_app_credentials_without_exposing_error_body() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for (status, body, success) in [
            (
                "200 OK",
                r#"{"access_token":"issued-token","token_type":"Bearer"}"#,
                true,
            ),
            ("401 Unauthorized", "sensitive server detail", false),
            (
                "200 OK",
                r#"{"access_token":"","token_type":"Bearer"}"#,
                false,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let headers = String::from_utf8(request).unwrap().to_lowercase();
                assert!(headers.starts_with("post /token "));
                assert!(headers.contains("authorization: basic awq6c2vjcmv0"));
                let len: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body_bytes = vec![0; len];
                stream.read_exact(&mut body_bytes).unwrap();
                assert_eq!(body_bytes, b"grant_type=client_credentials");
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let result = issue_token(
                &Client::builder().no_proxy().build().unwrap(),
                &endpoint,
                "id",
                "secret",
            );
            server.join().unwrap();
            if success {
                assert_eq!(result.unwrap(), "issued-token");
            } else {
                assert!(!result.unwrap_err().contains("sensitive server detail"));
            }
        }
    }
}
