//! Command-line interface shared by `lawgit` and `git-law` (`git law ...`).

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::Command;

use crate::api;
use crate::autocommit;
use crate::check;
use crate::git;
use crate::json::Json;
use crate::law::{self, NewLaw};
use crate::openapi;
use crate::repo::Repo;
use crate::rpc;
use crate::rule;
use crate::server;
use crate::VERSION;

const USAGE: &str = "\
LawGit - 法令・ルールを XML + Git で管理する / Manage laws and rules as XML in Git

使い方 / Usage:
  lawgit [--repo <dir>] <command> [options]
  git law [--repo <dir>] <command> [options]      (git の外部サブコマンド。ヘルプは `git law help`)

法令 / Laws (laws/<UUID>.xml):
  init                          リポジトリを初期化 (laws/, rules/, git init)
  new --name N --type T [--law-number X] [--e-gov-id Y]
                                UUID v4 を採番して法令 XML を作成
  validate                      laws/ を検証 (構文・必須要素・UUID・ファイル名・重複・本文構造)
  list                          検証済みの法令を一覧 (TAB 区切り: uuid, type, name, law-number)
  show <UUID>                   法令 XML を表示
  check                         laws/ と rules/ をまとめて検証

ルール / Rules (rules/<UUID>.xml):
  rule make                     対話的に作成 (本文は Markdown。単独の . で終了、.. でリテラルの .)
  rule new --name N [--category C] [--body TEXT | --body-file FILE]
                                非対話で作成
  rule list [--category C] [--query Q]
  rule show <UUID>
  rule set <UUID> [--name N] [--category C] [--body TEXT | --body-file FILE]
  rule edit <UUID>              $VISUAL / $EDITOR で編集し、保存後に検証
  rule path <UUID>              XML ファイルのパスを表示
  rule delete <UUID> --yes
  rule validate

Git / 自動コミット:
  commit -m <message>           検証後、laws/ と rules/ をまとめて commit
  config autocommit [on|off]    自動コミットの設定を表示/変更 (リポジトリ内の git config に保存)
  --commit / --no-commit        その実行だけ自動コミットを強制 ON / OFF (環境変数 LAWGIT_AUTOCOMMIT=1 でも可)
                                ON の間、new / rule new・make・set・edit・delete と API の書き込みは
                                「変更したファイルだけ」を 1 変更 = 1 commit で記録する。
                                -m <message> で commit メッセージを指定できる (既定: 「ルールを更新: 名前 [UUID]」など)

検索・履歴:
  search <語句...>              法令・ルールを全文検索 (名前・番号・カテゴリ・本文)
  history <UUID>                その法令/ルールを変更した commit の一覧 (削除済みも可)

外部アプリ連携 / Integration:
  serve [--host H] [--port P] [--token T] [--cors-origin ORIGIN[,ORIGIN...]] [--commit]
                                ローカル HTTP JSON API (既定 127.0.0.1:8787, /api/v1/...)
                                --cors-origin: ブラウザアプリ用 (完全一致のオリジンのみ。--token 必須)
  openapi                       OpenAPI 3 仕様 (JSON) を出力 (サーバでは /api/v1/openapi.json)
  rpc                           stdin/stdout の JSON-lines インターフェース
  --json                        list / show / validate / check / rule ... を JSON で出力

共通オプション:
  --repo <dir>                  対象リポジトリ (既定: カレントディレクトリ)
  -h, --help / help             ヘルプ
  -V, --version                 バージョン

終了コード: 0 成功 / 1 検証エラーあり / 2 使用方法・実行エラー
";

struct Args {
    pos: Vec<String>,
    opts: HashMap<String, String>,
    flags: HashSet<String>,
}

impl Args {
    fn opt(&self, key: &str) -> Option<&str> {
        self.opts.get(key).map(|s| s.as_str())
    }

    fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
}

const VALUE_OPTS: &[&str] = &[
    "repo", "name", "type", "law-number", "e-gov-id", "category", "body", "body-file", "query",
    "host", "port", "token", "message", "cors-origin",
];
const FLAG_OPTS: &[&str] = &["json", "help", "version", "yes", "commit", "no-commit"];

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut args = Args {
        pos: Vec::new(),
        opts: HashMap::new(),
        flags: HashSet::new(),
    };
    let mut i = 0;
    let mut only_positional = false;
    while i < raw.len() {
        let a = raw[i].as_str();
        i += 1;
        if only_positional {
            args.pos.push(a.to_string());
            continue;
        }
        if a == "--" {
            only_positional = true;
            continue;
        }
        let key: String;
        let mut inline_value: Option<String> = None;
        if let Some(long) = a.strip_prefix("--") {
            match long.find('=') {
                Some(p) => {
                    key = long[..p].to_string();
                    inline_value = Some(long[p + 1..].to_string());
                }
                None => key = long.to_string(),
            }
        } else if a == "-h" {
            key = "help".to_string();
        } else if a == "-V" {
            key = "version".to_string();
        } else if a == "-m" {
            key = "message".to_string();
        } else if a.starts_with('-') && a.len() > 1 {
            return Err(format!("unknown option '{}'", a));
        } else {
            args.pos.push(a.to_string());
            continue;
        }

        if VALUE_OPTS.contains(&key.as_str()) {
            let value = match inline_value {
                Some(v) => v,
                None => {
                    if i < raw.len() {
                        let v = raw[i].clone();
                        i += 1;
                        v
                    } else {
                        return Err(format!("option '--{}' requires a value", key));
                    }
                }
            };
            args.opts.insert(key, value);
        } else if FLAG_OPTS.contains(&key.as_str()) {
            if inline_value.is_some() {
                return Err(format!("option '--{}' does not take a value", key));
            }
            args.flags.insert(key);
        } else {
            return Err(format!("unknown option '--{}'", key));
        }
    }
    Ok(args)
}

pub fn run(raw: Vec<String>) -> i32 {
    match run_inner(&raw) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    }
}

fn run_inner(raw: &[String]) -> Result<i32, String> {
    let args = parse_args(raw)?;
    if args.flag("version") {
        println!("lawgit {}", VERSION);
        return Ok(0);
    }
    if args.flag("help") || args.pos.is_empty() || args.pos[0] == "help" {
        print!("{}", USAGE);
        return Ok(if args.flag("help") || (!args.pos.is_empty() && args.pos[0] == "help") { 0 } else { 2 });
    }
    let auto = match (args.flag("commit"), args.flag("no-commit")) {
        (true, true) => return Err("--commit and --no-commit cannot be used together".to_string()),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    let repo = Repo::new(args.opt("repo").unwrap_or(".")).with_auto_commit(auto);
    let json = args.flag("json");
    match args.pos[0].as_str() {
        "init" => cmd_init(&repo),
        "new" => cmd_new(&repo, &args),
        "validate" => cmd_validate(&repo, json),
        "check" => cmd_check(&repo, json),
        "list" => cmd_list(&repo, json),
        "show" => cmd_show(&repo, &args, json),
        "rule" => cmd_rule(&repo, &args, json),
        "commit" => cmd_commit(&repo, &args),
        "config" => cmd_config(&repo, &args),
        "search" => cmd_search(&repo, &args, json),
        "history" => cmd_history(&repo, &args, json),
        "openapi" => {
            println!("{}", openapi::SPEC);
            Ok(0)
        }
        "serve" => cmd_serve(repo, &args),
        "rpc" => {
            rpc::run(&repo)?;
            Ok(0)
        }
        other => Err(format!("unknown command '{}' (try `help`)", other)),
    }
}

fn empty_params() -> Json {
    Json::Obj(Vec::new())
}

fn print_errors(errors: &[String]) {
    for e in errors {
        eprintln!("  - {}", e);
    }
}

/// After a successful change: record it as one commit when auto-commit is on.
/// A failed commit is reported (exit code 1) but never undoes the saved change.
fn finish_commit(repo: &Repo, args: &Args, json: bool, paths: Vec<String>, default_msg: String) -> Result<i32, String> {
    let msg = match args.opt("message") {
        Some(m) => m.to_string(),
        None => default_msg,
    };
    match autocommit::commit_change(repo, &paths, &msg) {
        Ok(Some(h)) => {
            let line = format!("Committed {}: {}", h, msg.lines().next().unwrap_or(""));
            if json {
                eprintln!("{}", line);
            } else {
                println!("{}", line);
            }
            Ok(0)
        }
        Ok(None) => Ok(0),
        Err(e) => {
            eprintln!("warning: the change was saved, but auto-commit failed: {}", e);
            Ok(1)
        }
    }
}

fn need_uuid(args: &Args, index: usize, what: &str) -> Result<String, String> {
    args.pos
        .get(index)
        .cloned()
        .ok_or_else(|| format!("missing <UUID> argument for `{}`", what))
}

fn cmd_init(repo: &Repo) -> Result<i32, String> {
    for m in repo.init()? {
        println!("{}", m);
    }
    Ok(0)
}

fn cmd_new(repo: &Repo, args: &Args) -> Result<i32, String> {
    let name = args.opt("name").ok_or("`new` requires --name")?;
    let law_type = args.opt("type").ok_or("`new` requires --type")?;
    let new = NewLaw {
        name: name.to_string(),
        law_type: law_type.to_string(),
        law_number: args.opt("law-number").map(|s| s.to_string()),
        e_gov_id: args.opt("e-gov-id").map(|s| s.to_string()),
    };
    let info = law::create(repo, &new)?;
    println!("Created {}", info.path.display());
    println!("UUID: {}", info.uuid);
    println!("Edit the <body> of the file to add the legal text, then run `validate`.");
    finish_commit(
        repo,
        args,
        false,
        vec![format!("laws/{}.xml", info.uuid)],
        autocommit::default_message("法令を追加", &info.name, &info.uuid),
    )
}

fn cmd_validate(repo: &Repo, json: bool) -> Result<i32, String> {
    let r = law::validate_all(repo)?;
    if json {
        println!("{}", api::report_json("laws", r.items.len(), &r.errors).dump());
    } else if r.errors.is_empty() {
        println!("OK: {} law document(s) valid", r.items.len());
    } else {
        eprintln!("{} problem(s) found in laws/:", r.errors.len());
        print_errors(&r.errors);
    }
    Ok(if r.errors.is_empty() { 0 } else { 1 })
}

fn cmd_check(repo: &Repo, json: bool) -> Result<i32, String> {
    let c = check::check_all(repo)?;
    if json {
        let v = Json::obj(vec![
            ("valid", Json::Bool(c.errors.is_empty())),
            ("laws", Json::num(c.laws)),
            ("rules", Json::num(c.rules)),
            ("errors", Json::strings(&c.errors)),
        ]);
        println!("{}", v.dump());
    } else if c.errors.is_empty() {
        println!("OK: {} law document(s), {} rule document(s) valid", c.laws, c.rules);
    } else {
        eprintln!("{} problem(s) found:", c.errors.len());
        print_errors(&c.errors);
    }
    Ok(if c.errors.is_empty() { 0 } else { 1 })
}

fn cmd_list(repo: &Repo, json: bool) -> Result<i32, String> {
    if json {
        println!("{}", api::dispatch(repo, "law.list", &empty_params())?.dump());
        return Ok(0);
    }
    let r = law::validate_all(repo)?;
    for l in &r.items {
        println!(
            "{}\t{}\t{}\t{}",
            l.uuid,
            l.law_type,
            l.name,
            l.law_number.as_deref().unwrap_or("")
        );
    }
    if !r.errors.is_empty() {
        eprintln!("warning: {} invalid file(s) skipped (run `validate`):", r.errors.len());
        print_errors(&r.errors);
    }
    Ok(0)
}

fn cmd_show(repo: &Repo, args: &Args, json: bool) -> Result<i32, String> {
    let u = need_uuid(args, 1, "show")?;
    if json {
        let p = Json::obj(vec![("uuid", Json::str(&u))]);
        println!("{}", api::dispatch(repo, "law.get", &p)?.dump());
    } else {
        print!("{}", law::read_xml(repo, &u)?);
    }
    Ok(0)
}

fn read_line() -> Result<Option<String>, String> {
    let mut s = String::new();
    let n = io::stdin()
        .lock()
        .read_line(&mut s)
        .map_err(|e| format!("cannot read stdin: {}", e))?;
    if n == 0 {
        return Ok(None);
    }
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    Ok(Some(s))
}

fn prompt(label: &str) -> Result<String, String> {
    eprint!("{}", label);
    let _ = io::stderr().flush();
    Ok(read_line()?.unwrap_or_default())
}

fn read_body_stdin() -> Result<String, String> {
    eprintln!("Body (Markdown). End with a single '.' on its own line; use '..' for a literal '.' line:");
    let mut lines: Vec<String> = Vec::new();
    loop {
        match read_line()? {
            None => break,
            Some(l) => {
                if l == "." {
                    break;
                } else if l == ".." {
                    lines.push(".".to_string());
                } else {
                    lines.push(l);
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

/// Body from --body / --body-file, if given.
fn body_from_opts(args: &Args) -> Result<Option<String>, String> {
    if args.opt("body").is_some() && args.opt("body-file").is_some() {
        return Err("give either --body or --body-file, not both".to_string());
    }
    if let Some(b) = args.opt("body") {
        return Ok(Some(b.to_string()));
    }
    if let Some(f) = args.opt("body-file") {
        let t = fs::read_to_string(f).map_err(|e| format!("cannot read {}: {}", f, e))?;
        return Ok(Some(t));
    }
    Ok(None)
}

fn print_rule_created(doc: &rule::RuleDoc) {
    println!("Created {}", doc.path.display());
    println!("UUID: {}", doc.uuid);
}

fn cmd_rule(repo: &Repo, args: &Args, json: bool) -> Result<i32, String> {
    let sub = args
        .pos
        .get(1)
        .map(|s| s.as_str())
        .ok_or("`rule` requires a subcommand (make, new, list, show, set, edit, path, delete, validate)")?;
    match sub {
        "make" => {
            repo.require_initialized()?;
            let name = match args.opt("name") {
                Some(n) => n.to_string(),
                None => prompt("Rule name: ")?,
            };
            let category = match args.opt("category") {
                Some(c) => c.to_string(),
                None => prompt(&format!("Category [{}]: ", rule::DEFAULT_CATEGORY))?,
            };
            let body = match body_from_opts(args)? {
                Some(b) => b,
                None => read_body_stdin()?,
            };
            let doc = rule::create(repo, &name, &category, &body)?;
            print_rule_created(&doc);
            finish_commit(
                repo,
                args,
                false,
                vec![format!("rules/{}.xml", doc.uuid)],
                autocommit::default_message("ルールを追加", &doc.name, &doc.uuid),
            )
        }
        "new" => {
            let name = args.opt("name").ok_or("`rule new` requires --name")?;
            let body = body_from_opts(args)?.unwrap_or_default();
            let doc = rule::create(repo, name, args.opt("category").unwrap_or(""), &body)?;
            if json {
                println!("{}", api::rule_json(&doc, true).dump());
            } else {
                print_rule_created(&doc);
            }
            finish_commit(
                repo,
                args,
                json,
                vec![format!("rules/{}.xml", doc.uuid)],
                autocommit::default_message("ルールを追加", &doc.name, &doc.uuid),
            )
        }
        "list" => {
            if json {
                let mut v: Vec<(&str, Json)> = Vec::new();
                if let Some(c) = args.opt("category") {
                    v.push(("category", Json::str(c)));
                }
                if let Some(q) = args.opt("query") {
                    v.push(("query", Json::str(q)));
                }
                println!("{}", api::dispatch(repo, "rule.list", &Json::obj(v))?.dump());
                return Ok(0);
            }
            let r = rule::validate_all(repo)?;
            let items = rule::filter(r.items, args.opt("category"), args.opt("query"));
            for d in &items {
                println!("{}\t{}\t{}\t{}", d.uuid, d.category, d.name, d.path.display());
            }
            if !r.errors.is_empty() {
                eprintln!("warning: {} invalid file(s) skipped (run `rule validate`):", r.errors.len());
                print_errors(&r.errors);
            }
            Ok(0)
        }
        "show" => {
            let u = need_uuid(args, 2, "rule show")?;
            let doc = rule::get(repo, &u)?;
            if json {
                println!("{}", api::rule_json(&doc, true).dump());
            } else {
                println!("UUID:     {}", doc.uuid);
                println!("Name:     {}", doc.name);
                println!("Category: {}", doc.category);
                println!("Created:  {}", doc.created);
                println!("Path:     {}", doc.path.display());
                println!();
                println!("{}", doc.body);
            }
            Ok(0)
        }
        "set" => {
            let u = need_uuid(args, 2, "rule set")?;
            let body = body_from_opts(args)?;
            let doc = rule::update(repo, &u, args.opt("name"), args.opt("category"), body.as_deref())?;
            if json {
                println!("{}", api::rule_json(&doc, true).dump());
            } else {
                println!("Updated {}", doc.path.display());
            }
            finish_commit(
                repo,
                args,
                json,
                vec![format!("rules/{}.xml", doc.uuid)],
                autocommit::default_message("ルールを更新", &doc.name, &doc.uuid),
            )
        }
        "path" => {
            let u = need_uuid(args, 2, "rule path")?;
            println!("{}", rule::path_for(repo, &u)?.display());
            Ok(0)
        }
        "edit" => {
            let u = need_uuid(args, 2, "rule edit")?;
            let path = rule::path_for(repo, &u)?;
            let before = fs::read_to_string(&path).unwrap_or_default();
            let editor = env::var("VISUAL")
                .or_else(|_| env::var("EDITOR"))
                .unwrap_or_else(|_| "vi".to_string());
            let status = Command::new(&editor)
                .arg(&path)
                .status()
                .map_err(|e| format!("cannot run editor '{}': {}", editor, e))?;
            if !status.success() {
                return Err(format!("editor '{}' exited with an error", editor));
            }
            match rule::load_file(&path) {
                Ok(doc) => {
                    println!("OK: {} is valid", path.display());
                    let after = fs::read_to_string(&path).unwrap_or_default();
                    if after != before {
                        finish_commit(
                            repo,
                            args,
                            false,
                            vec![format!("rules/{}.xml", doc.uuid)],
                            autocommit::default_message("ルールを編集", &doc.name, &doc.uuid),
                        )
                    } else {
                        Ok(0)
                    }
                }
                Err(errs) => {
                    eprintln!("{} is not valid:", path.display());
                    print_errors(&errs);
                    Ok(1)
                }
            }
        }
        "delete" => {
            let u = need_uuid(args, 2, "rule delete")?;
            if !args.flag("yes") {
                return Err("refusing to delete without --yes".to_string());
            }
            let name = rule::delete_checked(repo, &u, None)?;
            println!("Deleted rule {} (its UUID is never reused)", u);
            finish_commit(
                repo,
                args,
                false,
                vec![format!("rules/{}.xml", u)],
                autocommit::default_message("ルールを削除", &name, &u),
            )
        }
        "validate" => {
            let r = rule::validate_all(repo)?;
            if json {
                println!("{}", api::report_json("rules", r.items.len(), &r.errors).dump());
            } else if r.errors.is_empty() {
                println!("OK: {} rule document(s) valid", r.items.len());
            } else {
                eprintln!("{} problem(s) found in rules/:", r.errors.len());
                print_errors(&r.errors);
            }
            Ok(if r.errors.is_empty() { 0 } else { 1 })
        }
        other => Err(format!("unknown rule subcommand '{}'", other)),
    }
}

fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

fn cmd_config(repo: &Repo, args: &Args) -> Result<i32, String> {
    let key = args.pos.get(1).map(|s| s.as_str());
    let value = args.pos.get(2).map(|s| s.as_str());
    match (key, value) {
        (None, _) | (Some("autocommit"), None) => {
            println!("autocommit = {}", on_off(autocommit::enabled(repo)));
            Ok(0)
        }
        (Some("autocommit"), Some(v)) => {
            let on = match v {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => return Err(format!("invalid value '{}' (use on or off)", v)),
            };
            repo.require_initialized()?;
            autocommit::set_config(repo, on)?;
            println!("autocommit = {} (saved in this repository's git config)", on_off(on));
            Ok(0)
        }
        (Some(other), _) => Err(format!("unknown setting '{}' (available: autocommit)", other)),
    }
}

fn cmd_search(repo: &Repo, args: &Args, json: bool) -> Result<i32, String> {
    let q = args.pos[1..].join(" ");
    if q.trim().is_empty() {
        return Err("`search` requires a query".to_string());
    }
    let res = api::dispatch(repo, "search", &Json::obj(vec![("q", Json::Str(q))]))?;
    if json {
        println!("{}", res.dump());
        return Ok(0);
    }
    if let Some(Json::Arr(hits)) = res.get("hits") {
        for h in hits {
            let g = |k: &str| h.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let matched = match h.get("matched") {
                Some(Json::Arr(m)) => m.iter().filter_map(|x| x.as_str()).collect::<Vec<&str>>().join(","),
                _ => String::new(),
            };
            println!("{}\t{}\t{}\t{}\t{}", g("kind"), g("uuid"), g("name"), matched, g("snippet"));
        }
    }
    if res.get("truncated") == Some(&Json::Bool(true)) {
        eprintln!("(more results exist; refine the query)");
    }
    Ok(0)
}

fn cmd_history(repo: &Repo, args: &Args, json: bool) -> Result<i32, String> {
    let u = need_uuid(args, 1, "history")?;
    let p = Json::obj(vec![("uuid", Json::str(&u))]);
    let mut res = api::dispatch(repo, "law.history", &p)?;
    if matches!(res.get("commits"), Some(Json::Arr(a)) if a.is_empty()) {
        res = api::dispatch(repo, "rule.history", &p)?;
    }
    if json {
        println!("{}", res.dump());
        return Ok(0);
    }
    if let Some(Json::Arr(commits)) = res.get("commits") {
        for c in commits {
            let g = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let hash = g("hash");
            let short: String = hash.chars().take(8).collect();
            println!("{}\t{}\t{}\t{}", short, g("date"), g("author"), g("subject"));
        }
    }
    Ok(0)
}

fn cmd_commit(repo: &Repo, args: &Args) -> Result<i32, String> {
    let msg = args.opt("message").ok_or("`commit` requires -m <message>")?;
    let out = git::commit(repo, msg)?;
    print!("{}", out);
    Ok(0)
}

fn cmd_serve(repo: Repo, args: &Args) -> Result<i32, String> {
    let host = args.opt("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = match args.opt("port") {
        Some(p) => p.parse::<u16>().map_err(|_| format!("invalid --port '{}'", p))?,
        None => 8787,
    };
    let token = match args.opt("token") {
        Some(t) => Some(t.to_string()),
        None => env::var("LAWGIT_TOKEN").ok().filter(|t| !t.is_empty()),
    };
    let cors: Vec<String> = args
        .opt("cors-origin")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    server::serve(
        repo,
        &host,
        port,
        server::ServerConfig {
            token,
            cors_origins: cors,
        },
    )?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_options() {
        let a = parse_args(&v(&["--repo", "x", "new", "--name=道路", "--json", "-m", "msg"])).unwrap();
        assert_eq!(a.pos, vec!["new".to_string()]);
        assert_eq!(a.opt("repo"), Some("x"));
        assert_eq!(a.opt("name"), Some("道路"));
        assert_eq!(a.opt("message"), Some("msg"));
        assert!(a.flag("json"));
    }

    #[test]
    fn rejects_bad_options() {
        assert!(parse_args(&v(&["--bogus"])).is_err());
        assert!(parse_args(&v(&["--name"])).is_err());
        assert!(parse_args(&v(&["--json=1"])).is_err());
        assert!(parse_args(&v(&["-x"])).is_err());
    }

    #[test]
    fn end_to_end_rule_and_law() {
        let dir = std::env::temp_dir().join(format!("lawgit-test-e2e-{}", crate::uuid::new_v4()));
        let repo = Repo::new(&dir);
        repo.init().unwrap();

        let law = law::create(
            &repo,
            &NewLaw {
                name: "道路交通法".to_string(),
                law_type: "法律".to_string(),
                law_number: Some("昭和三十五年法律第百五号".to_string()),
                e_gov_id: Some("335AC0000000105".to_string()),
            },
        )
        .unwrap();
        let rule_doc = rule::create(&repo, "規則", "", "# t\n\nbody").unwrap();
        assert_eq!(rule_doc.category, "general");

        let rep = check::check_all(&repo).unwrap();
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!((rep.laws, rep.rules), (1, 1));

        let updated = rule::update(&repo, &rule_doc.uuid, Some("新規則"), None, Some("changed")).unwrap();
        assert_eq!(updated.name, "新規則");
        assert_eq!(updated.body, "changed");
        assert_eq!(updated.created, rule_doc.created);

        let p = Json::obj(vec![("uuid", Json::str(&law.uuid))]);
        let r = api::dispatch(&repo, "law.get", &p).unwrap();
        assert!(r.get("xml").is_some());
        assert!(api::dispatch(&repo, "law.get", &Json::obj(vec![("uuid", Json::str("../x"))])).is_err());
        let line = rpc::handle_line(&repo, "{\"id\":7,\"method\":\"ping\"}");
        assert_eq!(line.get("ok"), Some(&Json::Bool(true)));
        assert_eq!(line.get("id"), Some(&Json::Num(7.0)));

        rule::delete(&repo, &rule_doc.uuid).unwrap();
        assert!(rule::get(&repo, &rule_doc.uuid).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
