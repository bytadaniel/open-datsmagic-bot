use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const ENDPOINT: &str = "/play/magcarp/player/move";

pub fn post_desert(base_url: &str, token: &str, payload: &Value) -> Result<(u16, String), String> {
    let (authority, host, port) = parse_http_origin(base_url)?;
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut stream = TcpStream::connect(&address).map_err(|e| format!("connect {address}: {e}"))?;
    let timeout = Some(Duration::from_secs(3));
    stream
        .set_read_timeout(timeout)
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(timeout)
        .map_err(|e| e.to_string())?;

    let body = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let request = format!(
        "POST {ENDPOINT} HTTP/1.1\r\nHost: {authority}\r\nX-Auth-Token: {token}\r\nContent-Type: application/json\r\nAccept: application/json\r\nAccept-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&body).map_err(|e| e.to_string())?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| e.to_string())?;
    decode_response(&response)
}

fn parse_http_origin(url: &str) -> Result<(String, String, u16), String> {
    let authority = url
        .strip_prefix("http://")
        .ok_or_else(|| {
            "only http:// server origins are supported by this minimal client".to_string()
        })?
        .trim_end_matches('/');
    if authority.is_empty() || authority.contains('/') || authority.contains('@') {
        return Err("DATS_SERVER_URL must be an http origin without a path or user info".into());
    }

    if authority.starts_with('[') {
        let close = authority.find(']').ok_or("invalid IPv6 server origin")?;
        let host = authority[1..close].to_string();
        let port = authority[close + 1..]
            .strip_prefix(':')
            .map(str::parse::<u16>)
            .transpose()
            .map_err(|_| "invalid server port")?
            .unwrap_or(80);
        return Ok((authority.to_string(), host, port));
    }

    if let Some((host, raw_port)) = authority.rsplit_once(':') {
        if let Ok(port) = raw_port.parse::<u16>() {
            return Ok((authority.to_string(), host.to_string(), port));
        }
    }
    Ok((authority.to_string(), authority.to_string(), 80))
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
    use super::{decode_chunked, decode_response, parse_http_origin};

    #[test]
    fn parses_localhost_origin() {
        assert_eq!(parse_http_origin("http://127.0.0.1:8080").unwrap().2, 8080);
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
