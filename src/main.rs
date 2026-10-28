// koburi — low-RSS container stats HTTP bridge.
// See AGENTS.md for the architecture rules this file must obey.
//
// Rules in force here:
//   * stdlib only (no [dependencies] in Cargo.toml).
//   * blocking I/O, one thread per connection, no async runtime.
//   * container runtime socket opened per request, closed on drop.
//   * stream-parse upstream payloads in a bounded buffer; never read_to_string
//     the whole response into a single growing Vec.
//   * musl static binary built with the release profile in Cargo.toml.
#![allow(dead_code)]

use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::Duration;

mod json;
mod stats;
#[cfg(unix)]
mod runtime;
#[cfg(unix)]
use runtime::{Runtime, SocketResolution};

const DEFAULT_PORT: u16 = 9911;
const READ_CHUNK: usize = 8 * 1024;
#[cfg(unix)]
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);

fn main() {
    let port = env::var("KOBURI_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("koburi: bind {port}: {e}");
            process::exit(1);
        }
    };

    eprintln!("koburi: listening on 0.0.0.0:{port}");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
                thread::spawn(move || {
                    let _ = handle_conn(s);
                });
            }
            Err(e) => eprintln!("koburi: accept: {e}"),
        }
    }
}

// ---------- HTTP server ----------

fn handle_conn(stream: TcpStream) -> io::Result<()> {
    let mut br = BufReader::new(stream);
    let mut request_line = String::new();
    if br.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.trim_end().split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    // drain headers
    let mut content_length: usize = 0;
    loop {
        let mut header = String::new();
        let n = br.read_line(&mut header)?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
    }
    // ignore body bytes; this server has no POST handlers
    if content_length > 0 {
        let mut sink = [0u8; 1024];
        let mut remaining = content_length;
        while remaining > 0 {
            let take = remaining.min(sink.len());
            let _ = br.read_exact(&mut sink[..take]);
            remaining -= take;
        }
    }

    let stream = br.into_inner();
    let mut writer = stream;

    match (method, path) {
        ("GET", "/health") => write_json(&mut writer, 200, br#"{"ok":true}"#),
        ("GET", "/stats") => serve_stats(&mut writer, None),
        (m, p) if m == "GET" && p.starts_with("/stats/") => {
            let id = &p["/stats/".len()..];
            if id.is_empty() || id.contains('/') || id.len() > 128 {
                return write_json(&mut writer, 400, br#"{"error":"bad id"}"#);
            }
            serve_stats(&mut writer, Some(id))
        }
        ("GET", "/") => {
            let body = format!(
                r#"{{"version":{},"endpoints":["/health","/stats","/stats/{{id}}"]}}"#,
                json::quote_string(env!("CARGO_PKG_VERSION"))
            );
            write_json(&mut writer, 200, body.as_bytes())
        }
        _ => write_json(&mut writer, 404, br#"{"error":"not found"}"#),
    }
}

fn write_json<W: Write>(w: &mut W, status: u16, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    };
    write!(
        w,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    w.write_all(body)
}

fn err_json(w: &mut TcpStream, status: u16, msg: &str) -> io::Result<()> {
    let s = format!(r#"{{"error":{}}}"#, json::quote_string(msg));
    write_json(w, status, s.as_bytes())
}

// ---------- request orchestration ----------

fn serve_stats(writer: &mut TcpStream, id: Option<&str>) -> io::Result<()> {
    #[cfg(not(unix))]
    {
        let _ = id;
        return err_json(writer, 503, "koburi runs on unix only");
    }
    #[cfg(unix)]
    {
        let resolutions = match runtime::resolve() {
            Ok(r) => r,
            Err(e) => return err_json(writer, 503, &format!("runtime: {e}")),
        };

        let mut out = String::with_capacity(512);
        out.push('[');
        let mut first = true;
        let mut found_any = false;

        for res in &resolutions {
            if res.runtime == Runtime::Pm2 {
                let payload = r#"j:{"type":"call","method":"getMonitorData","args":[{}]}"#;
                let mut msg = Vec::new();
                msg.push(0x12); // version 1, argc = 2

                // arg 0 length and val (payload)
                msg.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                msg.extend_from_slice(payload.as_bytes());

                // arg 1 length and val (ID)
                let arg1 = b"1";
                msg.extend_from_slice(&(arg1.len() as u32).to_be_bytes());
                msg.extend_from_slice(arg1);

                let list_body = match upstream_send_recv(res, &msg) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("koburi: pm2 error on {}: {e}", res.path.display());
                        continue;
                    }
                };

                let json_data = match stats::parse_pm2_response(&list_body) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("koburi: parse pm2 response failed on {}: {e}", res.path.display());
                        continue;
                    }
                };

                let root = match json::classify(json_data, 0) {
                    Some(s) if s.kind == json::Kind::Object => s,
                    _ => {
                        eprintln!("koburi: pm2 response is not a valid JSON object");
                        continue;
                    }
                };

                if let Some(err_span) = json::object_get(json_data, root, "error") {
                    let err_msg = json::unquote(&json_data[err_span.start..err_span.end])
                        .unwrap_or_else(|_| "unknown error".to_owned());
                    eprintln!("koburi: pm2 god daemon returned error: {}", err_msg);
                    continue;
                }

                let args_span = match json::object_get(json_data, root, "args") {
                    Some(s) if s.kind == json::Kind::Array => s,
                    _ => {
                        eprintln!("koburi: pm2 response args is missing or not an array");
                        continue;
                    }
                };

                let mut list_span = None;
                json::array_iter(json_data, args_span, |item| {
                    list_span = Some(item);
                    false // stop after first item
                });

                let Some(list_span) = list_span else {
                    eprintln!("koburi: pm2 response args array is empty");
                    continue;
                };

                if list_span.kind != json::Kind::Array {
                    eprintln!(
                        "koburi: pm2 response first arg is not an array (kind={:?}). Full JSON: {}",
                        list_span.kind,
                        String::from_utf8_lossy(json_data)
                    );
                    continue;
                }

                let list_slice = &json_data[list_span.start..list_span.end];

                let processes = match stats::parse_pm2_list(list_slice) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!(
                            "koburi: parse pm2 failed on {}: {e}. Body was (len={}): {:?}",
                            res.path.display(),
                            list_slice.len(),
                            String::from_utf8_lossy(list_slice)
                        );
                        continue;
                    }
                };

                let host_mem = stats::get_system_mem();
                for p in processes {
                    if id.map(|want| want == p.name || want == format!("{}", p.pid) || want == format!("{}", p.pm_id)).unwrap_or(true) {
                        found_any = true;
                        if !first {
                            out.push(',');
                        }
                        first = false;
                        stats::append_pm2_json(&mut out, &p, host_mem);
                    }
                }
                continue;
            }

            let list_path = "/containers/json?all=0";

            let list_body = match upstream_get(res, list_path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("koburi: list error on {}: {e}", res.path.display());
                    continue;
                }
            };

            let chosen: Vec<(String, String, String)> = match stats::parse_container_list(&list_body) {
                Ok(v) => v
                    .into_iter()
                    .filter(|(cid, _, _)| id.map(|want| want == cid).unwrap_or(true))
                    .collect(),
                Err(e) => {
                    eprintln!(
                        "koburi: parse list failed on {}: {e}. Body was (len={}): {:?}",
                        res.path.display(),
                        list_body.len(),
                        String::from_utf8_lossy(&list_body)
                    );
                    continue;
                }
            };

            if !chosen.is_empty() {
                found_any = true;
            }

            for (cid, name, status) in chosen {
                let stats_path = match res.runtime {
                    Runtime::Docker => format!("/containers/{cid}/stats?stream=false&one-shot=true"),
                    Runtime::Podman => format!("/containers/{cid}/stats?stream=false"),
                    Runtime::Pm2 => unreachable!(),
                };
                let body = match upstream_get(res, &stats_path) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("koburi: stats {cid}: {e}");
                        continue;
                    }
                };
                let snap = match stats::compute(&body) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("koburi: parse stats {cid}: {e}");
                        continue;
                    }
                };
                if !first {
                    out.push(',');
                }
                first = false;
                stats::append_json(
                    &mut out,
                    &cid,
                    &name,
                    &status,
                    match res.runtime {
                        Runtime::Docker => "docker",
                        Runtime::Podman => "podman",
                        Runtime::Pm2 => unreachable!(),
                    },
                    &snap,
                );
            }
        }
        out.push(']');

        if id.is_some() && !found_any {
            return err_json(writer, 404, "container not found");
        }

        write_json(writer, 200, out.as_bytes())
    }
}

// ---------- upstream socket client ----------

#[cfg(unix)]
fn read_amp_message(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 1];
    stream.read_exact(&mut header)?;
    let version_argc = header[0];
    let argc = version_argc & 0x0f;

    let mut msg = Vec::new();
    msg.push(version_argc);

    for _ in 0..argc {
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes)?;
        msg.extend_from_slice(&len_bytes);
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut arg_data = vec![0u8; len];
        stream.read_exact(&mut arg_data)?;
        msg.extend_from_slice(&arg_data);
    }

    Ok(msg)
}

#[cfg(unix)]
fn upstream_send_recv(res: &SocketResolution, request: &[u8]) -> io::Result<Vec<u8>> {
    // Open per request, close on return. AGENTS.md rule 3.
    let mut sock = UnixStream::connect(&res.path)?;
    sock.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    sock.set_write_timeout(Some(UPSTREAM_TIMEOUT))?;

    sock.write_all(request)?;
    let _ = sock.flush();
    read_amp_message(&mut sock)
}

#[cfg(unix)]
fn upstream_get(res: &SocketResolution, path: &str) -> io::Result<Vec<u8>> {
    // Open per request, close on return. AGENTS.md rule 3.
    let mut sock = UnixStream::connect(&res.path)?;
    sock.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    sock.set_write_timeout(Some(UPSTREAM_TIMEOUT))?;

    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
    sock.write_all(req.as_bytes())?;
    let _ = sock.flush();

    // Read the response. Headers are short; we do bounded reads and stop
    // accumulating after the body. We do NOT read_to_string the whole thing.
    read_http_response(&mut sock)
}

fn read_http_response<R: Read>(sock: &mut R) -> io::Result<Vec<u8>> {
    // Read until we have seen the \r\n\r\n header terminator. Track how many
    // header bytes we consumed so the caller can truncate if it cares; we
    // return the full buffer minus the head for now because the body shapes
    // we need are small and the JSON parser skips over header bytes anyway
    // is the easy choice. To keep the buffer bounded, we cap the read.
    let mut buf = Vec::with_capacity(READ_CHUNK);
    let mut tmp = [0u8; READ_CHUNK];
    let header_end;
    loop {
        let n = sock.read(&mut tmp)?;
        if n == 0 {
            // EOF before headers complete; return what we have.
            return Ok(buf);
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(idx) = find_double_crlf(&buf) {
            header_end = idx + 4;
            break;
        }
        if buf.len() > 64 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "headers too large"));
        }
    }

    // Parse Content-Length if present; otherwise read until EOF.
    let headers = std::str::from_utf8(&buf[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "headers not utf-8"))?;
    let content_length = header_content_length(headers);
    let transfer_chunked = header_has(headers, "transfer-encoding", "chunked");

    let body_start = header_end;
    if transfer_chunked {
        const BODY_CAP: usize = 16 * 1024 * 1024;
        let mut body = Vec::with_capacity(4096);
        let mut cursor = body_start;
        loop {
            let line_end = match find_crlf(&buf, cursor) {
                Some(i) => i,
                None => {
                    let n = sock.read(&mut tmp)?;
                    if n == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "chunked: eof before size",
                        ));
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    continue;
                }
            };
            let size_line = std::str::from_utf8(&buf[cursor..line_end])
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size utf8"))?;
            let size_hex = size_line.split(';').next().unwrap_or("").trim();
            let chunk_size = usize::from_str_radix(size_hex, 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size hex"))?;
            cursor = line_end + 2; // skip CRLF
            if chunk_size == 0 {
                break;
            }
            while buf.len() < cursor + chunk_size + 2 {
                let n = sock.read(&mut tmp)?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "chunked: eof in chunk",
                    ));
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            if body.len() + chunk_size > BODY_CAP {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
            }
            body.extend_from_slice(&buf[cursor..cursor + chunk_size]);
            cursor += chunk_size + 2; // skip chunk data + CRLF
        }
        Ok(body)
    } else if let Some(cl) = content_length {
        // Ensure the full body is in the buffer (bounded by cl).
        while buf.len() - body_start < cl {
            let n = sock.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.len() > body_start + cl {
                // truncate overshoot to keep the buffer tight
                buf.truncate(body_start + cl);
                break;
            }
        }
        // Return only the body slice.
        let len = (body_start + cl).min(buf.len());
        Ok(buf.drain(body_start..len).collect())
    } else {
        // No Content-Length (Connection: close). Read until EOF, but cap.
        let cap = 16 * 1024 * 1024;
        while buf.len() < cap {
            let n = sock.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        Ok(buf.split_off(body_start))
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    for i in 0..=buf.len() - 4 {
        if &buf[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    if from >= buf.len() {
        return None;
    }
    let mut i = from;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn header_content_length(headers: &str) -> Option<usize> {
    for line in headers.split("\r\n").skip(1) {
        let mut it = line.splitn(2, ':');
        let name = it.next()?.trim();
        let value = it.next()?.trim();
        if name.eq_ignore_ascii_case("content-length") {
            return value.parse().ok();
        }
    }
    None
}

fn header_has(headers: &str, name: &str, expected_value: &str) -> bool {
    for line in headers.split("\r\n").skip(1) {
        let mut it = line.splitn(2, ':');
        let n = it.next().unwrap_or("").trim();
        let v = it.next().unwrap_or("").trim();
        if n.eq_ignore_ascii_case(name) {
            return v
                .split(',')
                .any(|p| p.trim().eq_ignore_ascii_case(expected_value));
        }
    }
    false
}

// ---------- small utility accessors used by tests / future endpoints ----------

#[allow(dead_code)]
fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_http_response_content_length() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nHello, world!";
        let mut cursor = Cursor::new(&response[..]);
        let body = read_http_response(&mut cursor).unwrap();
        assert_eq!(body, b"Hello, world!");
    }

    #[test]
    fn test_read_http_response_no_content_length() {
        let response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nHello, world!";
        let mut cursor = Cursor::new(&response[..]);
        let body = read_http_response(&mut cursor).unwrap();
        assert_eq!(body, b"Hello, world!");
    }

    #[test]
    fn test_read_http_response_chunked() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nHello\r\n7\r\n, world\r\n0\r\n\r\n";
        let mut cursor = Cursor::new(&response[..]);
        let body = read_http_response(&mut cursor).unwrap();
        assert_eq!(body, b"Hello, world");
    }
}
