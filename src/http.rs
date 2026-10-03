use flate2::read::GzDecoder;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

const ENDPOINT: &str = "/play/magcarp/player/move";

pub fn post_desert(base_url: &str, token: &str, payload: &Value) -> Result<(u16, String), String> {
    let origin = parse_server_origin(base_url)?;
    let address = if origin.host.contains(':') {
        format!("[{}]:{}", origin.host, origin.port)
    } else {
        format!("{}:{}", origin.host, origin.port)
    };
    let stream = TcpStream::connect(&address).map_err(|e| format!("connect {address}: {e}"))?;
    let timeout = Some(Duration::from_secs(3));
    stream
        .set_read_timeout(timeout)
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(timeout)
        .map_err(|e| e.to_string())?;

    let mut stream: Box<dyn ReadWrite> = if origin.tls {
        let server_name = ServerName::try_from(origin.host.clone())
            .map_err(|error| format!("invalid HTTPS server name {}: {error}", origin.host))?;
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connection = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|error| format!("start TLS with {}: {error}", origin.host))?;
        Box::new(StreamOwned::new(connection, stream))
    } else {
        Box::new(stream)
    };

    let body = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let request = format!(
        "POST {ENDPOINT} HTTP/1.1\r\nHost: {}\r\nX-Auth-Token: {token}\r\nContent-Type: application/json\r\nAccept: application/json\r\nAccept-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        origin.authority,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("send request to {base_url}: {e}"))?;
    stream
        .write_all(&body)
        .map_err(|e| format!("send request body to {base_url}: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read response from {base_url}: {e}"))?;
    decode_response(&response)
}

#[derive(Debug, PartialEq, Eq)]
struct ServerOrigin {
    authority: String,
    host: String,
    port: u16,
    tls: bool,
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn parse_server_origin(url: &str) -> Result<ServerOrigin, String> {
    let (tls, raw_authority, default_port) = if let Some(authority) = url.strip_prefix("http://") {
        (false, authority, 80)
    } else if let Some(authority) = url.strip_prefix("https://") {
        (true, authority, 443)
    } else {
        return Err("DATS_SERVER_URL must start with http:// or https://".into());
    };
    let authority = raw_authority.trim_end_matches('/');
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('@')
        || authority.contains('?')
        || authority.contains('#')
        || authority.chars().any(char::is_whitespace)
    {
        return Err("DATS_SERVER_URL must be an origin without path, query, or user info".into());
    }

    if authority.starts_with('[') {
        let close = authority.find(']').ok_or("invalid IPv6 server origin")?;
        let host = authority[1..close].to_string();
        let port_suffix = &authority[close + 1..];
        let port = if port_suffix.is_empty() {
            default_port
        } else {
            port_suffix
                .strip_prefix(':')
                .ok_or("invalid IPv6 server origin")?
                .parse::<u16>()
                .map_err(|_| "invalid server port")?
        };
        return Ok(ServerOrigin {
            authority: authority.to_string(),
            host,
            port,
            tls,
        });
    }

    if let Some((host, raw_port)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return Err("IPv6 server origins must use brackets".into());
        }
        let port = raw_port.parse::<u16>().map_err(|_| "invalid server port")?;
        if host.is_empty() {
            return Err("DATS_SERVER_URL host is empty".into());
        }
        return Ok(ServerOrigin {
            authority: authority.to_string(),
            host: host.to_string(),
            port,
            tls,
        });
    }

    Ok(ServerOrigin {
        authority: authority.to_string(),
        host: authority.to_string(),
        port: default_port,
        tls,
    })
}

fn decode_response(response: &[u8]) -> Result<(u16, String), String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("HTTP response has no header terminator")?;
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|e| e.to_string())?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("invalid HTTP status line")?;
    let mut body = response[header_end + 4..].to_vec();
    let gzip_encoded = headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("content-encoding")
                && value.trim().eq_ignore_ascii_case("gzip")
        })
    });
    if headers.lines().any(|line| {
        line.to_ascii_lowercase()
            .starts_with("transfer-encoding: chunked")
    }) {
        body = decode_chunked(&body)?;
    } else if let Some(length) = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    }) {
        body.truncate(length.min(body.len()));
    }
    if gzip_encoded {
        let mut decoded = Vec::new();
        GzDecoder::new(body.as_slice())
            .read_to_end(&mut decoded)
            .map_err(|error| format!("invalid gzip response: {error}"))?;
        body = decoded;
    }
    let text = String::from_utf8(body).map_err(|e| e.to_string())?;
    if (200..300).contains(&status) {
        let _: Value =
            serde_json::from_str(&text).map_err(|e| format!("invalid JSON response: {e}"))?;
    }
    Ok((status, text))
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("invalid chunked body: missing chunk header")?;
        let raw_size = std::str::from_utf8(&input[..line_end]).map_err(|e| e.to_string())?;
        let size = usize::from_str_radix(raw_size.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "invalid chunk size")?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if input.len() < size + 2 || &input[size..size + 2] != b"\r\n" {
            return Err("truncated chunked body".into());
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_chunked, decode_response, parse_server_origin};

    #[test]
    fn parses_localhost_origin() {
        let origin = parse_server_origin("http://127.0.0.1:8080").unwrap();
        assert_eq!(origin.port, 8080);
        assert!(!origin.tls);
    }

    #[test]
    fn parses_https_origin_with_default_tls_port() {
        let origin = parse_server_origin("https://stadmagic.strangled.net/").unwrap();
        assert_eq!(origin.authority, "stadmagic.strangled.net");
        assert_eq!(origin.host, "stadmagic.strangled.net");
        assert_eq!(origin.port, 443);
        assert!(origin.tls);
    }

    #[test]
    fn parses_https_origin_with_explicit_port() {
        let origin = parse_server_origin("https://arena.example:8443").unwrap();
        assert_eq!(origin.port, 8443);
        assert!(origin.tls);
    }

    #[test]
    fn rejects_server_urls_that_are_not_origins() {
        assert!(parse_server_origin("https://arena.example/play").is_err());
        assert!(parse_server_origin("ftp://arena.example").is_err());
        assert!(parse_server_origin("https://user@arena.example").is_err());
        assert!(parse_server_origin("https://arena.example:nope").is_err());
    }

    #[test]
    fn decodes_chunked_http_body() {
        assert_eq!(decode_chunked(b"4\r\ntest\r\n0\r\n\r\n").unwrap(), b"test");
    }

    #[test]
    fn decodes_gzip_http_json_response() {
        let payload = br#"{"points":42}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            compressed.len()
        );
        let mut response = headers.into_bytes();
        response.extend(compressed);

        let (status, body) = decode_response(&response).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, std::str::from_utf8(payload).unwrap());
    }
}
