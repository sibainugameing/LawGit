//! JSON-lines interface on stdin/stdout: one request object per line.
//!
//! Request:  {"id": 1, "method": "rule.list", "params": {"category": "general"}}
//! Response: {"id": 1, "ok": true, "result": {...}}  or  {"id": 1, "ok": false, "error": "..."}

use std::io::{self, BufRead, Write};

use crate::api;
use crate::json::{self, Json};
use crate::repo::Repo;

pub fn handle_line(repo: &Repo, line: &str) -> Json {
    let req = match json::parse(line) {
        Ok(v) => v,
        Err(e) => {
            return Json::obj(vec![
                ("id", Json::Null),
                ("ok", Json::Bool(false)),
                ("error", Json::Str(format!("invalid JSON: {}", e))),
            ]);
        }
    };
    let id = req.get("id").cloned().unwrap_or(Json::Null);
    let method = match req.get("method").and_then(|m| m.as_str()) {
        Some(m) => m.to_string(),
        None => {
            return Json::obj(vec![
                ("id", id),
                ("ok", Json::Bool(false)),
                ("error", Json::str("invalid params: 'method' (string) is required")),
            ]);
        }
    };
    let params = req.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
    let result = api::dispatch(repo, &method, &params);
    let mut pairs: Vec<(String, Json)> = vec![("id".to_string(), id)];
    match api::envelope(result) {
        Json::Obj(rest) => pairs.extend(rest),
        other => pairs.push(("result".to_string(), other)),
    }
    Json::Obj(pairs)
}

pub fn run(repo: &Repo) -> Result<(), String> {
    repo.require_initialized()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("cannot read stdin: {}", e))?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = handle_line(repo, &line);
        writeln!(out, "{}", resp.dump()).map_err(|e| format!("cannot write stdout: {}", e))?;
        out.flush().map_err(|e| format!("cannot write stdout: {}", e))?;
    }
    Ok(())
}
