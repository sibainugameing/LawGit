//! Local HTTP/1.1 JSON API (`lawgit serve`). Standard library only.
//!
//! Safety defaults: binds to 127.0.0.1, serves one request at a time, limits
//! request size, validates the Host header on loopback binds (DNS rebinding),
//! requires `Content-Type: application/json` for writes (so browsers need a
//! CORS preflight), and supports an optional token. CORS is opt-in per origin
//! (`--cors-origin`) and then requires a token.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::api;
use crate::json::{self, Json};
use crate::openapi;
use crate::repo::Repo;
use crate::util::percent_decode;

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;

pub struct ServerConfig {
    pub token: Option<String>,
    /// Exact origins (e.g. `https://app.example.com`) allowed to call the API from a browser.
    pub cors_origins: Vec<String>,
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    code: u16,
    body: String,
    headers: String,
}

pub fn serve(repo: Repo, host: &str, port: u16, cfg: ServerConfig) -> Result<(), String> {
    repo.require_initialized()?;
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    if !loopback && cfg.token.is_none() {
        return Err("binding to a non-loopback address requires --token".to_string());
    }
    if !cfg.cors_origins.is_empty() && cfg.token.is_none() {
        return Err("--cors-origin requires --token".to_string());
    }
    for o in &cfg.cors_origins {
        let ok = (o.starts_with("http://") || o.starts_with("https://"))
            && !o.contains('*')
            && !o.contains(char::is_whitespace)
            && !o.ends_with('/');
        if !ok {
            return Err(format!(
                "invalid --cors-origin '{}' (use exact origins like https://app.example.com; wildcards are not allowed)",
                o
            ));
        }
    }
    let listener = TcpListener::bind((host, port))
        .map_err(|e| format!("cannot bind {}:{}: {}", host, port, e))?;
    eprintln!(
        "lawgit: serving {} on http://{}:{}/api/v1/ (auto-commit: {}) (Ctrl+C to stop)",
        repo.root.display(),
        host,
        port,
        if crate::autocommit::enabled(&repo) { "on" } else { "off" }
    );
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle(&repo, s, &cfg, loopback) {
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
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn write_reply(stream: &mut TcpStream, r: &Reply) -> io::Result<()> {
    let head = if r.code == 204 {
        format!(
            "HTTP/1.1 204 No Content\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
            r.headers
        )
    } else {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{}Connection: close\r\n\r\n",
            r.code,
            status_text(r.code),
            r.body.len(),
            r.headers
        )
    };
    stream.write_all(head.as_bytes())?;
    if r.code != 204 {
        stream.write_all(r.body.as_bytes())?;
    }
    stream.flush()
}

fn json_reply(code: u16, body: &Json) -> Reply {
    Reply {
        code,
        body: body.dump(),
        headers: String::new(),
    }
}

fn err_reply(code: u16, msg: &str) -> Reply {
    json_reply(code, &api::envelope(Err(msg.to_string())))
}

fn handle(repo: &Repo, mut stream: TcpStream, cfg: &ServerConfig, loopback: bool) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(msg) => return write_reply(&mut stream, &err_reply(400, &msg)),
    };
    let origin = req.headers.get("origin").cloned().unwrap_or_default();
    let origin_ok = !origin.is_empty() && cfg.cors_origins.iter().any(|a| *a == origin);
    let mut reply = if req.method == "OPTIONS" {
        if origin_ok {
            Reply {
                code: 204,
                body: String::new(),
                headers: "Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Lawgit-Token\r\nAccess-Control-Max-Age: 600\r\n".to_string(),
            }
        } else {
            err_reply(403, "forbidden: origin not allowed")
        }
    } else {
        route(repo, &req, cfg, loopback)
    };
    if origin_ok {
        reply
            .headers
            .push_str(&format!("Access-Control-Allow-Origin: {}\r\nVary: Origin\r\n", origin));
    }
    write_reply(&mut stream, &reply)
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

/// Build call parameters from the query string (plus fixed `extra` pairs).
fn query_params(req: &Request, extra: Vec<(&str, Json)>) -> Json {
    let mut v = extra;
    for key in ["q", "category", "query", "since", "if_version", "commit_message"] {
        if let Some(val) = req.query.get(key) {
            v.push((key, Json::str(val)));
        }
    }
    if let Some(n) = req.query.get("limit").and_then(|s| s.parse::<f64>().ok()) {
        v.push(("limit", Json::Num(n)));
    }
    Json::obj(v)
}

fn run_method(repo: &Repo, method: &str, params: Json) -> Reply {
    match api::dispatch(repo, method, &params) {
        Ok(v) => json_reply(200, &api::envelope(Ok(v))),
        Err(e) => json_reply(api::status_for_error(&e), &api::envelope(Err(e))),
    }
}

fn run_with_body(repo: &Repo, req: &Request, method: &str, id: Option<&str>) -> Reply {
    match body_json(req) {
        Ok(b) => {
            let params = match id {
                Some(i) => with_uuid(b, i),
                None => b,
            };
            run_method(repo, method, params)
        }
        Err(e) => err_reply(400, &e),
    }
}

fn route(repo: &Repo, req: &Request, cfg: &ServerConfig, loopback: bool) -> Reply {
    if let Some(t) = cfg.token.as_deref() {
        let want = format!("Bearer {}", t);
        let by_bearer = req.headers.get("authorization").map(|v| *v == want).unwrap_or(false);
        let by_header = req.headers.get("x-lawgit-token").map(|v| v.as_str() == t).unwrap_or(false);
        if !by_bearer && !by_header {
            return err_reply(401, "unauthorized: missing or wrong token");
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
            return err_reply(403, "forbidden: unexpected Host header");
        }
    }

    let writes = req.method == "POST" || req.method == "PUT" || req.method == "PATCH";
    if writes {
        let ct = req.headers.get("content-type").map(|v| v.to_lowercase()).unwrap_or_default();
        if !ct.contains("application/json") {
            return err_reply(415, "invalid: Content-Type must be application/json");
        }
    }

    let rest = match req.path.strip_prefix("/api/v1") {
        Some(r) => r,
        None => return err_reply(404, "not found: use /api/v1/..."),
    };
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();

    match (req.method.as_str(), segs.as_slice()) {
        ("GET", ["ping"]) => run_method(repo, "ping", Json::Obj(Vec::new())),
        ("GET", ["methods"]) => run_method(repo, "methods", Json::Obj(Vec::new())),
        ("GET", ["validate"]) => run_method(repo, "repo.check", Json::Obj(Vec::new())),
        ("GET", ["openapi.json"]) => Reply {
            code: 200,
            body: openapi::SPEC.to_string(),
            headers: String::new(),
        },
        ("GET", ["search"]) => run_method(repo, "search", query_params(req, vec![])),
        ("GET", ["changes"]) => run_method(repo, "changes", query_params(req, vec![])),

        ("GET", ["laws"]) => run_method(repo, "law.list", Json::Obj(Vec::new())),
        ("GET", ["laws", id]) => run_method(repo, "law.get", Json::obj(vec![("uuid", Json::str(id))])),
        ("GET", ["laws", id, "history"]) => {
            run_method(repo, "law.history", query_params(req, vec![("uuid", Json::str(id))]))
        }
        ("GET", ["laws", id, "revisions", rev]) => run_method(
            repo,
            "law.revision",
            Json::obj(vec![("uuid", Json::str(id)), ("rev", Json::str(rev))]),
        ),
        ("POST", ["laws"]) => run_with_body(repo, req, "law.create", None),

        ("GET", ["rules"]) => run_method(repo, "rule.list", query_params(req, vec![])),
        ("GET", ["rules", id]) => run_method(repo, "rule.get", Json::obj(vec![("uuid", Json::str(id))])),
        ("GET", ["rules", id, "history"]) => {
            run_method(repo, "rule.history", query_params(req, vec![("uuid", Json::str(id))]))
        }
        ("GET", ["rules", id, "revisions", rev]) => run_method(
            repo,
            "rule.revision",
            Json::obj(vec![("uuid", Json::str(id)), ("rev", Json::str(rev))]),
        ),
        ("POST", ["rules"]) => run_with_body(repo, req, "rule.create", None),
        ("PUT", ["rules", id]) | ("PATCH", ["rules", id]) => {
            run_with_body(repo, req, "rule.update", Some(*id))
        }
        ("DELETE", ["rules", id]) => {
            run_method(repo, "rule.delete", query_params(req, vec![("uuid", Json::str(id))]))
        }

        ("GET", ["git", "status"]) => run_method(repo, "git.status", Json::Obj(Vec::new())),
        ("GET", ["git", "log"]) => run_method(repo, "git.log", query_params(req, vec![])),
        ("POST", ["git", "commit"]) => run_with_body(repo, req, "git.commit", None),

        ("POST", ["rpc"]) => match body_json(req) {
            Ok(b) => {
                let method = b.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
                let params = b.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
                if method.is_empty() {
                    err_reply(400, "invalid params: 'method' (string) is required")
                } else {
                    run_method(repo, &method, params)
                }
            }
            Err(e) => err_reply(400, &e),
        },

        _ => err_reply(404, "not found: no such route or method"),
    }
}
