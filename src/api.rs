//! Transport-independent API used by the CLI `--json` output, the stdio
//! JSON-lines interface (`lawgit rpc`) and the local HTTP server (`lawgit serve`).

use crate::autocommit;
use crate::check;
use crate::git;
use crate::json::Json;
use crate::law::{self, LawInfo, NewLaw};
use crate::repo::Repo;
use crate::rule::{self, RuleDoc};
use crate::search;
use crate::uuid;
use crate::VERSION;

pub const METHODS: &[&str] = &[
    "ping",
    "methods",
    "law.list",
    "law.get",
    "law.create",
    "law.validate",
    "law.history",
    "law.revision",
    "rule.list",
    "rule.get",
    "rule.create",
    "rule.update",
    "rule.delete",
    "rule.validate",
    "rule.history",
    "rule.revision",
    "search",
    "changes",
    "repo.check",
    "git.status",
    "git.log",
    "git.commit",
];

pub fn law_json(l: &LawInfo) -> Json {
    Json::obj(vec![
        ("uuid", Json::str(&l.uuid)),
        ("name", Json::str(&l.name)),
        ("type", Json::str(&l.law_type)),
        ("law_number", Json::opt_str(&l.law_number)),
        ("e_gov_id", Json::opt_str(&l.e_gov_id)),
        ("path", Json::str(&l.path.display().to_string())),
        ("version", Json::str(&l.version)),
    ])
}

pub fn rule_json(r: &RuleDoc, with_body: bool) -> Json {
    let mut v: Vec<(&str, Json)> = vec![
        ("uuid", Json::str(&r.uuid)),
        ("name", Json::str(&r.name)),
        ("category", Json::str(&r.category)),
        ("created", Json::str(&r.created)),
        ("path", Json::str(&r.path.display().to_string())),
        ("version", Json::str(&r.version)),
    ];
    if with_body {
        v.push(("body", Json::str(&r.body)));
    }
    Json::obj(v)
}

pub fn report_json(count_key: &str, count: usize, errors: &[String]) -> Json {
    Json::obj(vec![
        ("valid", Json::Bool(errors.is_empty())),
        (count_key, Json::num(count)),
        ("errors", Json::strings(errors)),
    ])
}

fn p_str<'a>(params: &'a Json, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|v| v.as_str())
}

fn p_req<'a>(params: &'a Json, key: &str) -> Result<&'a str, String> {
    p_str(params, key).ok_or_else(|| format!("invalid params: '{}' (string) is required", key))
}

fn limit_param(params: &Json, default: usize) -> usize {
    params
        .get("limit")
        .and_then(|v| v.as_f64())
        .map(|n| {
            if n < 1.0 {
                1usize
            } else if n > 500.0 {
                500usize
            } else {
                n as usize
            }
        })
        .unwrap_or(default)
}

fn rel_path(kind: &str, uuid_s: &str) -> Result<String, String> {
    if !uuid::is_valid_v4(uuid_s) {
        return Err(format!(
            "invalid params: '{}' is not a valid lowercase UUID v4",
            uuid_s
        ));
    }
    Ok(format!("{}/{}.xml", kind, uuid_s))
}

fn commit_message(params: &Json, action: &str, name: &str, uuid_s: &str) -> String {
    match p_str(params, "commit_message") {
        Some(m) => m.to_string(),
        None => autocommit::default_message(action, name, uuid_s),
    }
}

/// Wrap a result as `{"ok":true,"result":...}` or `{"ok":false,"error":"..."}`.
pub fn envelope(r: Result<Json, String>) -> Json {
    match r {
        Ok(v) => Json::obj(vec![("ok", Json::Bool(true)), ("result", v)]),
        Err(e) => Json::obj(vec![("ok", Json::Bool(false)), ("error", Json::Str(e))]),
    }
}

/// HTTP status for an error string produced by `dispatch`.
pub fn status_for_error(e: &str) -> u16 {
    if e.starts_with("not found") {
        404
    } else if e.starts_with("conflict") {
        409
    } else if e.starts_with("internal") {
        500
    } else {
        400
    }
}

pub fn dispatch(repo: &Repo, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        "ping" => Ok(Json::obj(vec![
            ("name", Json::str("lawgit")),
            ("version", Json::str(VERSION)),
            ("root", Json::str(&repo.root.display().to_string())),
            ("auto_commit", Json::Bool(autocommit::enabled(repo))),
        ])),
        "methods" => Ok(Json::Arr(METHODS.iter().map(|m| Json::str(m)).collect())),

        "law.list" => {
            let r = law::validate_all(repo)?;
            Ok(Json::obj(vec![
                ("laws", Json::Arr(r.items.iter().map(law_json).collect())),
                ("errors", Json::strings(&r.errors)),
            ]))
        }
        "law.get" => {
            let (info, xml_text) = law::get(repo, p_req(params, "uuid")?)?;
            Ok(Json::obj(vec![("law", law_json(&info)), ("xml", Json::Str(xml_text))]))
        }
        "law.create" => {
            let new = NewLaw {
                name: p_req(params, "name")?.to_string(),
                law_type: p_req(params, "type")?.to_string(),
                law_number: p_str(params, "law_number").map(|s| s.to_string()),
                e_gov_id: p_str(params, "e_gov_id").map(|s| s.to_string()),
            };
            let info = law::create(repo, &new)?;
            let msg = commit_message(params, "法令を追加", &info.name, &info.uuid);
            let c = autocommit::commit_change(repo, &[format!("laws/{}.xml", info.uuid)], &msg);
            Ok(autocommit::attach(law_json(&info), autocommit::commit_json(&c, &msg)))
        }
        "law.validate" => {
            let r = law::validate_all(repo)?;
            Ok(report_json("laws", r.items.len(), &r.errors))
        }
        "law.history" => {
            let rel = rel_path("laws", p_req(params, "uuid")?)?;
            let commits = git::log_paths(repo, limit_param(params, 50), &[rel.as_str()])?;
            Ok(Json::obj(vec![("commits", commits)]))
        }
        "law.revision" => {
            let rev = p_req(params, "rev")?;
            let rel = rel_path("laws", p_req(params, "uuid")?)?;
            let xml_text = git::show_at(repo, rev, &rel)?;
            Ok(Json::obj(vec![("rev", Json::str(rev)), ("xml", Json::Str(xml_text))]))
        }

        "rule.list" => {
            let r = rule::validate_all(repo)?;
            let items = rule::filter(r.items, p_str(params, "category"), p_str(params, "query"));
            Ok(Json::obj(vec![
                ("rules", Json::Arr(items.iter().map(|x| rule_json(x, false)).collect())),
                ("errors", Json::strings(&r.errors)),
            ]))
        }
        "rule.get" => {
            let doc = rule::get(repo, p_req(params, "uuid")?)?;
            Ok(rule_json(&doc, true))
        }
        "rule.create" => {
            let doc = rule::create(
                repo,
                p_req(params, "name")?,
                p_str(params, "category").unwrap_or(""),
                p_str(params, "body").unwrap_or(""),
            )?;
            let msg = commit_message(params, "ルールを追加", &doc.name, &doc.uuid);
            let c = autocommit::commit_change(repo, &[format!("rules/{}.xml", doc.uuid)], &msg);
            Ok(autocommit::attach(rule_json(&doc, true), autocommit::commit_json(&c, &msg)))
        }
        "rule.update" => {
            let doc = rule::update_checked(
                repo,
                p_req(params, "uuid")?,
                p_str(params, "name"),
                p_str(params, "category"),
                p_str(params, "body"),
                p_str(params, "if_version"),
            )?;
            let msg = commit_message(params, "ルールを更新", &doc.name, &doc.uuid);
            let c = autocommit::commit_change(repo, &[format!("rules/{}.xml", doc.uuid)], &msg);
            Ok(autocommit::attach(rule_json(&doc, true), autocommit::commit_json(&c, &msg)))
        }
        "rule.delete" => {
            let u = p_req(params, "uuid")?;
            let name = rule::delete_checked(repo, u, p_str(params, "if_version"))?;
            let msg = commit_message(params, "ルールを削除", &name, u);
            let c = autocommit::commit_change(repo, &[format!("rules/{}.xml", u)], &msg);
            Ok(autocommit::attach(
                Json::obj(vec![("deleted", Json::str(u))]),
                autocommit::commit_json(&c, &msg),
            ))
        }
        "rule.validate" => {
            let r = rule::validate_all(repo)?;
            Ok(report_json("rules", r.items.len(), &r.errors))
        }
        "rule.history" => {
            let rel = rel_path("rules", p_req(params, "uuid")?)?;
            let commits = git::log_paths(repo, limit_param(params, 50), &[rel.as_str()])?;
            Ok(Json::obj(vec![("commits", commits)]))
        }
        "rule.revision" => {
            let rev = p_req(params, "rev")?;
            let rel = rel_path("rules", p_req(params, "uuid")?)?;
            let xml_text = git::show_at(repo, rev, &rel)?;
            Ok(Json::obj(vec![("rev", Json::str(rev)), ("xml", Json::Str(xml_text))]))
        }

        "search" => search::search(repo, p_req(params, "q")?, limit_param(params, 50)),
        "changes" => git::changes(repo, p_str(params, "since"), limit_param(params, 50)),

        "repo.check" => {
            let c = check::check_all(repo)?;
            Ok(Json::obj(vec![
                ("valid", Json::Bool(c.errors.is_empty())),
                ("laws", Json::num(c.laws)),
                ("rules", Json::num(c.rules)),
                ("errors", Json::strings(&c.errors)),
            ]))
        }

        "git.status" => {
            let out = git::status(repo)?;
            Ok(Json::obj(vec![
                ("clean", Json::Bool(out.trim().is_empty())),
                ("output", Json::Str(out)),
            ]))
        }
        "git.log" => Ok(Json::obj(vec![("commits", git::log(repo, limit_param(params, 20))?)])),
        "git.commit" => {
            let out = git::commit(repo, p_req(params, "message")?)?;
            Ok(Json::obj(vec![("output", Json::Str(out))]))
        }

        other => Err(format!("not found: unknown method '{}'", other)),
    }
}
