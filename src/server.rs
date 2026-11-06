//! Local HTTP/1.1 JSON API (`lawgit serve`). Standard library only.
//!
//! Safety defaults: binds to 127.0.0.1, serves one request at a time, limits
//! request size, validates the Host header on loopback binds (DNS-rebinding),
//! requires `Content-Type: application/json` for writes (so browsers need a
//! CORS preflight, which is never granted), and supports an optional token.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::api;
use crate::json::{self, Json};
use crate::repo::Repo;
use crate::util::percent_decode;

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

pub fn serve(repo: Repo, host: &str, port: u16, token: Option<String>) -> Result<(), String> {
    repo.require_initialized()?;
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    if !loopback && token.is_none() {
        return Err("binding to a non-loopback address requires --token".to_string());
    }
    let listener = TcpListener::bind((host, port))
        .map_err(|e| format!("cannot bind {}:{}: {}", host, port, e))?;
    eprintln!("lawgit: serving {} on http://{}:{}/api/v1/ (Ctrl+C to stop)", repo.root.display(), host, port);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle(&repo, s, token.as_deref(), loopback) {
                    eprintln!("lawgit: connection error: {}", e);
                }
            }
            Err(e) => eprintln!("lawgit: accept error: {}", e),
        }
    }
    Ok(())
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_sub(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEADER {
            return Err("request headers too large".to_string());
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| format!("read error: {}", e))?;
        if n == 0 {
            return Err("connection closed before request was complete".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let target = parts.next().unwrap_or("");
    if method.is_empty() || target.is_empty() {
        return Err("malformed request line".to_string());
    }

    let (raw_path, raw_query) = match target.find('?') {
        Some(i) => (&target[..i], &target[i + 1..]),
        None => (target, ""),
    };
    let mut query: HashMap<String, String> = HashMap::new();
    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        match pair.find('=') {
            Some(i) => {
                query.insert(percent_decode(&pair[..i]), percent_decode(&pair[i + 1..]));
            }
            None => {
                query.insert(percent_decode(pair), String::new());
            }
        }
    }

    let mut headers: HashMap<String, String> = HashMap::new();
    for line in lines {
        if let Some(i) = line.find(':') {
            headers.insert(line[..i].trim().to_lowercase(), line[i + 1..].trim().to_string());
        }
    }

    let content_length: usize = match headers.get("content-length") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| "invalid Content-Length".to_string())?,
        None => 0,
    };
    if content_length > MAX_BODY {
        return Err("request body too large".to_string());
    }
    let mut body: Vec<u8> = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| format!("read error: {}", e))?;
        if n == 0 {
            return Err("connection closed before body was complete".to_string());
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(Request {
        method,
        path: percent_decode(raw_path),
        query,
        headers,
        body,
    })
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn write_response(stream: &mut TcpStream, code: u16, body: &Json) -> io::Result<()> {
    let payload = body.dump();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        code,
        status_text(code),
        payload.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(payload.as_bytes())?;
    stream.flush()
}

fn err_body(msg: &str) -> Json {
    api::envelope(Err(msg.to_string()))
}

fn handle(repo: &Repo, mut stream: TcpStream, token: Option<&str>, loopback: bool) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(msg) => return write_response(&mut stream, 400, &err_body(&msg)),
    };
    let (code, body) = route(repo, &req, token, loopback);
    write_response(&mut stream, code, &body)
}

fn host_name(h: &str) -> &str {
    if h.starts_with('[') {
        match h.find(']') {
            Some(e) => &h[..=e],
            None => h,
        }
    } else {
        match h.rfind(':') {
            Some(p) => &h[..p],
            None => h,
        }
    }
}

fn body_json(req: &Request) -> Result<Json, String> {
    let text = std::str::from_utf8(&req.body).map_err(|_| "invalid: request body is not UTF-8".to_string())?;
    if text.trim().is_empty() {
        return Ok(Json::Obj(Vec::new()));
    }
    let v = json::parse(text).map_err(|e| format!("invalid: bad JSON body: {}", e))?;
    match v {
        Json::Obj(_) => Ok(v),
        _ => Err("invalid: JSON body must be an object".to_string()),
    }
}

fn with_uuid(body: Json, id: &str) -> Json {
    match body {
        Json::Obj(mut pairs) => {
            pairs.retain(|p| p.0 != "uuid");
            pairs.push(("uuid".to_string(), Json::str(id)));
            Json::Obj(pairs)
        }
        other => other,
    }
}

fn run_method(repo: &Repo, method: &str, params: Json) -> (u16, Json) {
    match api::dispatch(repo, method, &params) {
        Ok(v) => (200, api::envelope(Ok(v))),
        Err(e) => (api::status_for_error(&e), api::envelope(Err(e))),
    }
}

fn route(repo: &Repo, req: &Request, token: Option<&str>, loopback: bool) -> (u16, Json) {
    if let Some(t) = token {
        let want = format!("Bearer {}", t);
        let by_bearer = req.headers.get("authorization").map(|v| *v == want).unwrap_or(false);
        let by_header = req.headers.get("x-lawgit-token").map(|v| v.as_str() == t).unwrap_or(false);
        if !by_bearer && !by_header {
            return (401, err_body("unauthorized: missing or wrong token"));
        }
    } else if loopback {
        let host_ok = match req.headers.get("host") {
            Some(h) => {
                let n = host_name(h);
                n == "127.0.0.1" || n == "localhost" || n == "[::1]"
            }
            None => true,
        };
        if !host_ok {
            return (403, err_body("forbidden: unexpected Host header"));
        }
    }

    let writes = req.method == "POST" || req.method == "PUT" || req.method == "PATCH";
    if writes {
        let ct = req.headers.get("content-type").map(|v| v.to_lowercase()).unwrap_or_default();
        if !ct.contains("application/json") {
            return (415, err_body("invalid: Content-Type must be application/json"));
        }
    }

    let rest = match req.path.strip_prefix("/api/v1") {
        Some(r) => r,
        None => return (404, err_body("not found: use /api/v1/...")),
    };
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();

    let list_params = || -> Json {
        let mut v: Vec<(&str, Json)> = Vec::new();
        if let Some(c) = req.query.get("category") {
            v.push(("category", Json::str(c)));
        }
        if let Some(q) = req.query.get("query") {
            v.push(("query", Json::str(q)));
        }
        Json::obj(v)
    };

    match (req.method.as_str(), segs.as_slice()) {
        ("GET", ["ping"]) => run_method(repo, "ping", Json::Obj(Vec::new())),
        ("GET", ["methods"]) => run_method(repo, "methods", Json::Obj(Vec::new())),
        ("GET", ["validate"]) => run_method(repo, "repo.check", Json::Obj(Vec::new())),

        ("GET", ["laws"]) => run_method(repo, "law.list", Json::Obj(Vec::new())),
        ("GET", ["laws", id]) => run_method(repo, "law.get", Json::obj(vec![("uuid", Json::str(id))])),
        ("POST", ["laws"]) => match body_json(req) {
            Ok(b) => run_method(repo, "law.create", b),
            Err(e) => (400, err_body(&e)),
        },

        ("GET", ["rules"]) => run_method(repo, "rule.list", list_params()),
        ("GET", ["rules", id]) => run_method(repo, "rule.get", Json::obj(vec![("uuid", Json::str(id))])),
        ("POST", ["rules"]) => match body_json(req) {
            Ok(b) => run_method(repo, "rule.create", b),
            Err(e) => (400, err_body(&e)),
        },
        ("PUT", ["rules", id]) | ("PATCH", ["rules", id]) => match body_json(req) {
            Ok(b) => run_method(repo, "rule.update", with_uuid(b, id)),
            Err(e) => (400, err_body(&e)),
        },
        ("DELETE", ["rules", id]) => {
            run_method(repo, "rule.delete", Json::obj(vec![("uuid", Json::str(id))]))
        }

        ("GET", ["git", "status"]) => run_method(repo, "git.status", Json::Obj(Vec::new())),
        ("GET", ["git", "log"]) => {
            let mut v: Vec<(&str, Json)> = Vec::new();
            if let Some(l) = req.query.get("limit").and_then(|s| s.parse::<f64>().ok()) {
                v.push(("limit", Json::Num(l)));
            }
            run_method(repo, "git.log", Json::obj(v))
        }
        ("POST", ["git", "commit"]) => match body_json(req) {
            Ok(b) => run_method(repo, "git.commit", b),
            Err(e) => (400, err_body(&e)),
        },

        ("POST", ["rpc"]) => match body_json(req) {
            Ok(b) => {
                let method = b.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
                let params = b.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
                if method.is_empty() {
                    (400, err_body("invalid params: 'method' (string) is required"))
                } else {
                    run_method(repo, &method, params)
                }
            }
            Err(e) => (400, err_body(&e)),
        },

        _ => (404, err_body("not found: no such route or method")),
    }
}
