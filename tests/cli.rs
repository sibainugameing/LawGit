//! End-to-end tests that drive the real `lawgit` binary (CLI, rpc, HTTP, commit).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_lawgit");

fn temp_repo(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("lawgit-it-{}-{}-{}", tag, std::process::id(), nanos))
}

fn run(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(repo)
        .args(args)
        .env_remove("LAWGIT_AUTOCOMMIT")
        .env_remove("LAWGIT_TOKEN")
        .output()
        .expect("failed to run lawgit");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn init_repo(tag: &str) -> PathBuf {
    let repo = temp_repo(tag);
    let (rc, out, err) = run(&repo, &["init"]);
    assert_eq!(rc, 0, "init failed: {}{}", out, err);
    repo
}

fn uuid_from(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("UUID: "))
        .expect("no UUID line in output")
        .trim()
        .to_string()
}

fn new_law(repo: &Path) -> String {
    let (rc, out, err) = run(
        repo,
        &[
            "new",
            "--name",
            "道路交通法",
            "--type",
            "法律",
            "--law-number",
            "昭和三十五年法律第百五号",
            "--e-gov-id",
            "335AC0000000105",
        ],
    );
    assert_eq!(rc, 0, "new failed: {}{}", out, err);
    uuid_from(&out)
}

#[test]
fn version_and_usage_errors() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("lawgit"));

    let out = Command::new(BIN).arg("bogus-command").output().unwrap();
    assert_eq!(out.status.code(), Some(2));

    let out = Command::new(BIN).arg("--bogus-option").output().unwrap();
    assert_eq!(out.status.code(), Some(2));

    let out = Command::new(BIN).arg("help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn requires_initialized_repo() {
    let repo = temp_repo("uninit");
    fs::create_dir_all(&repo).unwrap();
    let (rc, _, err) = run(&repo, &["list"]);
    assert_eq!(rc, 2);
    assert!(err.contains("init"), "{}", err);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn law_and_rule_flow() {
    let repo = init_repo("flow");
    let law = new_law(&repo);

    assert_eq!(run(&repo, &["validate"]).0, 0);
    let (_, list, _) = run(&repo, &["list"]);
    assert!(list.contains(law.as_str()) && list.contains("道路交通法"), "{}", list);
    let (rc, shown, _) = run(&repo, &["show", law.as_str()]);
    assert_eq!(rc, 0);
    assert!(shown.contains("<e-gov-id>335AC0000000105</e-gov-id>"));
    assert_eq!(run(&repo, &["show", "../etc/passwd"]).0, 2);

    let (rc, out, err) = run(
        &repo,
        &["rule", "new", "--name", "休暇規則", "--category", "人事", "--body", "# 休暇\n\n- 5日 <b> & more"],
    );
    assert_eq!(rc, 0, "{}{}", out, err);
    let rule = uuid_from(&out);

    let (rc, out, _) = run(&repo, &["rule", "list", "--json", "--category", "人事"]);
    assert_eq!(rc, 0);
    assert!(out.contains("\"rules\"") && out.contains("休暇規則"), "{}", out);
    let (_, out, _) = run(&repo, &["rule", "list", "--query", "存在しない語"]);
    assert!(out.trim().is_empty(), "{}", out);

    let (rc, _, err) = run(&repo, &["rule", "set", rule.as_str(), "--name", "新休暇規則", "--body", "更新後"]);
    assert_eq!(rc, 0, "{}", err);
    let (rc, shown, _) = run(&repo, &["rule", "show", rule.as_str()]);
    assert_eq!(rc, 0);
    assert!(shown.contains("新休暇規則") && shown.contains("更新後"), "{}", shown);

    assert_eq!(run(&repo, &["check"]).0, 0);
    assert_eq!(run(&repo, &["rule", "delete", rule.as_str()]).0, 2, "delete needs --yes");
    assert_eq!(run(&repo, &["rule", "delete", rule.as_str(), "--yes"]).0, 0);
    assert_eq!(run(&repo, &["rule", "show", rule.as_str()]).0, 2);

    // corrupt the law file: validation must fail with exit code 1
    fs::write(repo.join("laws").join(format!("{}.xml", law)), "<law>").unwrap();
    assert_eq!(run(&repo, &["validate"]).0, 1);
    assert_eq!(run(&repo, &["check"]).0, 1);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn rule_make_reads_stdin() {
    let repo = init_repo("make");
    let mut child = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["rule", "make"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("運用規則\n運用\n# 見出し\n..\nend\n.\n".as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let uuid = uuid_from(&String::from_utf8_lossy(&out.stdout));
    let (_, shown, _) = run(&repo, &["rule", "show", uuid.as_str()]);
    assert!(shown.contains("運用規則") && shown.contains("# 見出し\n.\nend"), "{}", shown);
    let _ = fs::remove_dir_all(&repo);
}

fn git_available() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn commit_validates_then_commits() {
    if !git_available() {
        return;
    }
    let repo = init_repo("commit");
    new_law(&repo);
    let envs = [
        ("GIT_AUTHOR_NAME", "t"),
        ("GIT_AUTHOR_EMAIL", "t@example.com"),
        ("GIT_COMMITTER_NAME", "t"),
        ("GIT_COMMITTER_EMAIL", "t@example.com"),
    ];
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["commit", "-m", "初回"])
        .envs(envs)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let log = Command::new("git").arg("-C").arg(&repo).args(["log", "--format=%s"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "初回");

    // an invalid document blocks the commit
    let laws = repo.join("laws");
    let file = fs::read_dir(&laws)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().map(|x| x == "xml").unwrap_or(false))
        .unwrap();
    fs::write(&file, "<law>").unwrap();
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["commit", "-m", "壊れた"])
        .envs(envs)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let log = Command::new("git").arg("-C").arg(&repo).args(["log", "--format=%s"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "初回");
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn rpc_jsonlines() {
    let repo = init_repo("rpc");
    let law = new_law(&repo);
    let mut child = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .arg("rpc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = format!(
        "{}\n\nnot json\n{}\n{}\n",
        r#"{"id":1,"method":"ping"}"#,
        r#"{"id":"two","method":"rule.create","params":{"name":"RPC規則","body":"b"}}"#,
        format!(r#"{{"id":3,"method":"law.get","params":{{"uuid":"{}"}}}}"#, law),
    );
    child.stdin.as_mut().unwrap().write_all(input.as_bytes()).unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{}", text);
    assert!(lines[0].contains("\"id\":1") && lines[0].contains("\"ok\":true"), "{}", lines[0]);
    assert!(lines[1].contains("\"ok\":false") && lines[1].contains("invalid JSON"), "{}", lines[1]);
    assert!(lines[2].contains("\"id\":\"two\"") && lines[2].contains("RPC規則"), "{}", lines[2]);
    assert!(lines[3].contains("335AC0000000105"), "{}", lines[3]);
    let _ = fs::remove_dir_all(&repo);
}

// ------------------------------------------------------------------ HTTP

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// `extra` holds raw header lines, each ending in "\r\n".
fn http(port: u16, method: &str, path: &str, host: &str, extra: &str, body: Option<&str>) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{}", method, path, host, extra);
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

fn start_server(repo: &Path, extra: &[&str], envs: &[(&str, &str)]) -> (Kill, u16) {
    let port = free_port();
    let child = Command::new(BIN)
        .arg("--repo")
        .arg(repo)
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .args(extra)
        .envs(envs.iter().copied())
        .env_remove("LAWGIT_TOKEN")
        .env_remove("LAWGIT_AUTOCOMMIT")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn server");
    let guard = Kill(child);
    for _ in 0..100 {
        if let Some(r) = http(port, "GET", "/api/v1/ping", "127.0.0.1", "", None) {
            if r.starts_with("HTTP/1.1") {
                return (guard, port);
            }
        }
        sleep(Duration::from_millis(50));
    }
    panic!("server did not start");
}

const JSON_CT: &str = "Content-Type: application/json\r\n";

#[test]
fn http_api() {
    let repo = init_repo("http");
    let (_srv, port) = start_server(&repo, &[], &[]);

    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("200 OK") && r.contains("\"ok\":true") && r.contains("\"version\""), "{}", r);

    let r = http(port, "POST", "/api/v1/rules", "localhost", JSON_CT, Some(r#"{"name":"API規則","category":"api","body":"x"}"#)).unwrap();
    assert!(r.contains("200 OK") && r.contains("API規則"), "{}", r);
    let r = http(port, "GET", "/api/v1/rules?category=api", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("API規則"), "{}", r);
    let r = http(port, "GET", "/api/v1/rules?category=%E4%BB%96", "127.0.0.1", "", None).unwrap();
    assert!(!r.contains("API規則"), "{}", r);

    let r = http(port, "POST", "/api/v1/laws", "127.0.0.1", JSON_CT, Some(r#"{"name":"テスト法","type":"法律"}"#)).unwrap();
    assert!(r.contains("200 OK") && r.contains("テスト法"), "{}", r);
    let r = http(port, "GET", "/api/v1/validate", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("\"valid\":true"), "{}", r);

    let r = http(port, "POST", "/api/v1/rules", "127.0.0.1", "Content-Type: text/plain\r\n", Some("{}")).unwrap();
    assert!(r.contains("415"), "{}", r);
    let r = http(port, "POST", "/api/v1/rules", "127.0.0.1", JSON_CT, Some("{not json")).unwrap();
    assert!(r.contains("400"), "{}", r);
    let r = http(port, "GET", "/api/v1/ping", "evil.example", "", None).unwrap();
    assert!(r.contains("403"), "{}", r);
    let r = http(port, "GET", "/api/v1/nope", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("404"), "{}", r);
    let r = http(port, "GET", "/api/v1/laws/not-a-uuid", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("400"), "{}", r);
    let r = http(port, "GET", "/api/v1/rules/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("404"), "{}", r);
    let r = http(port, "POST", "/api/v1/rpc", "127.0.0.1", JSON_CT, Some(r#"{"method":"repo.check"}"#)).unwrap();
    assert!(r.contains("\"valid\":true"), "{}", r);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn http_token_required() {
    let repo = init_repo("token");
    let (_srv, port) = start_server(&repo, &["--token", "s3cret"], &[]);
    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "", None).unwrap();
    assert!(r.contains("401"), "{}", r);
    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "Authorization: Bearer wrong\r\n", None).unwrap();
    assert!(r.contains("401"), "{}", r);
    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "Authorization: Bearer s3cret\r\n", None).unwrap();
    assert!(r.contains("200 OK"), "{}", r);
    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "X-Lawgit-Token: s3cret\r\n", None).unwrap();
    assert!(r.contains("200 OK"), "{}", r);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn serve_refuses_public_bind_without_token() {
    let repo = init_repo("public");
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["serve", "--host", "0.0.0.0", "--port", "0"])
        .env_remove("LAWGIT_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("--token"), "{}", err);
    let _ = fs::remove_dir_all(&repo);
}

// ------------------------------------------------------------------ auto-commit, search, history

const ID_ENV: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "t"),
    ("GIT_AUTHOR_EMAIL", "t@example.com"),
    ("GIT_COMMITTER_NAME", "t"),
    ("GIT_COMMITTER_EMAIL", "t@example.com"),
];

fn run_id(repo: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(repo)
        .args(args)
        .envs(ID_ENV)
        .env_remove("LAWGIT_AUTOCOMMIT")
        .output()
        .expect("failed to run lawgit");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn git_out(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().expect("git failed to start");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn json_fields(text: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{}\":\"", key);
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(pat.as_str()) {
        let after = &rest[i + pat.len()..];
        match after.find('"') {
            Some(j) => {
                out.push(after[..j].to_string());
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

#[test]
fn auto_commit_cli() {
    if !git_available() {
        return;
    }
    let repo = init_repo("autocommit");
    assert!(run_id(&repo, &["config"]).1.contains("autocommit = off"));
    let (rc, out, _) = run_id(&repo, &["config", "autocommit", "on"]);
    assert_eq!(rc, 0, "{}", out);
    assert!(run_id(&repo, &["config"]).1.contains("autocommit = on"));
    assert_eq!(run_id(&repo, &["config", "autocommit", "maybe"]).0, 2);

    let (rc, out, err) = run_id(&repo, &["new", "--name", "道路交通法", "--type", "法律"]);
    assert_eq!(rc, 0, "{}{}", out, err);
    assert!(out.contains("Committed"), "{}", out);
    let law = uuid_from(&out);
    let subjects = git_out(&repo, &["log", "--format=%s"]);
    assert!(subjects.contains(&format!("法令を追加: 道路交通法 [{}]", law)), "{}", subjects);
    // only the new file is in the commit (README, .gitattributes, rules/.gitkeep stay untracked)
    assert_eq!(git_out(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), format!("laws/{}.xml", law));

    let (rc, out, err) = run_id(&repo, &["rule", "new", "--name", "休暇規則", "--body", "v1"]);
    assert_eq!(rc, 0, "{}{}", out, err);
    let rule = uuid_from(&out);
    assert_eq!(git_out(&repo, &["show", "--name-only", "--format=", "HEAD"]).trim(), format!("rules/{}.xml", rule));

    let (rc, out, _) = run_id(&repo, &["rule", "set", rule.as_str(), "--body", "v2 有給"]);
    assert_eq!(rc, 0);
    assert!(out.contains("Committed"), "{}", out);
    // setting the same content again creates no empty commit
    let before = git_out(&repo, &["rev-list", "--count", "HEAD"]);
    let (rc, out, _) = run_id(&repo, &["rule", "set", rule.as_str(), "--body", "v2 有給"]);
    assert_eq!(rc, 0);
    assert!(!out.contains("Committed"), "{}", out);
    assert_eq!(git_out(&repo, &["rev-list", "--count", "HEAD"]), before);

    // other staged work is never swept into an auto-commit
    fs::write(repo.join("notes.txt"), "x").unwrap();
    git_out(&repo, &["add", "notes.txt"]);
    let (rc, _, _) = run_id(&repo, &["rule", "new", "--name", "別規則", "-m", "独自メッセージ"]);
    assert_eq!(rc, 0);
    assert!(git_out(&repo, &["log", "-1", "--format=%s"]).contains("独自メッセージ"));
    assert!(git_out(&repo, &["status", "--porcelain"]).contains("A  notes.txt"));

    // --no-commit wins over the stored setting
    let (rc, out, _) = run_id(&repo, &["rule", "new", "--name", "未コミット", "--no-commit"]);
    assert_eq!(rc, 0);
    let pending = uuid_from(&out);
    assert!(git_out(&repo, &["status", "--porcelain"]).contains(pending.as_str()));

    // delete is recorded too, and history still works for deleted rules
    let (rc, out, _) = run_id(&repo, &["rule", "delete", rule.as_str(), "--yes"]);
    assert_eq!(rc, 0);
    assert!(out.contains("Committed"), "{}", out);
    assert!(git_out(&repo, &["log", "-1", "--format=%s"]).contains("ルールを削除"));
    let (rc, hist, _) = run_id(&repo, &["history", rule.as_str()]);
    assert_eq!(rc, 0);
    assert!(hist.contains("ルールを削除") && hist.contains("ルールを更新") && hist.contains("ルールを追加"), "{}", hist);
    let (_, hist_law, _) = run_id(&repo, &["history", law.as_str()]);
    assert!(hist_law.contains("法令を追加"), "{}", hist_law);

    let (rc, found, _) = run_id(&repo, &["search", "道路"]);
    assert_eq!(rc, 0);
    assert!(found.contains(law.as_str()), "{}", found);
    assert_eq!(run_id(&repo, &["search", "   "]).0, 2);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn auto_commit_via_environment_and_failure_reporting() {
    if !git_available() {
        return;
    }
    let repo = init_repo("autoenv");
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["rule", "new", "--name", "環境変数"])
        .envs(ID_ENV)
        .env("LAWGIT_AUTOCOMMIT", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Committed"));

    // --commit and --no-commit together are rejected
    assert_eq!(run_id(&repo, &["rule", "new", "--name", "x", "--commit", "--no-commit"]).0, 2);

    // a failing commit (no git identity) keeps the saved file and exits with 1
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(&repo)
        .args(["rule", "new", "--name", "身元なし", "--commit"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "")
        .env("GIT_COMMITTER_NAME", "")
        .env("GIT_AUTHOR_EMAIL", "")
        .env("GIT_COMMITTER_EMAIL", "")
        .env("EMAIL", "")
        .env("HOME", repo.as_os_str())
        .output()
        .unwrap();
    let rc = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    // Some systems can auto-detect an identity; accept that, but never a lost file or a crash.
    assert!(rc == 0 || rc == 1, "rc={} {}", rc, String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Created"), "{}", stdout);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn openapi_command() {
    let out = Command::new(BIN).arg("openapi").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let t = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(t.contains("\"openapi\":\"3.0.3\"") && t.contains("/rules/{uuid}"), "{}", t);
}

#[test]
fn http_extended_api() {
    if !git_available() {
        return;
    }
    let repo = init_repo("httpext");
    let (_srv, port) = start_server(&repo, &["--commit"], &ID_ENV);
    let get = |path: &str| http(port, "GET", path, "127.0.0.1", "", None).unwrap();

    let r = http(port, "POST", "/api/v1/rules", "127.0.0.1", JSON_CT, Some(r#"{"name":"同時編集","body":"v1"}"#)).unwrap();
    assert!(r.contains("200 OK") && r.contains("\"commit\":{\"hash\""), "{}", r);
    let uuid = json_fields(&r, "uuid")[0].clone();
    let v1 = json_fields(&r, "version")[0].clone();

    let r = get(&format!("/api/v1/rules/{}", uuid));
    assert_eq!(json_fields(&r, "version")[0], v1);

    let body = format!(r#"{{"body":"v2 有給","if_version":"{}"}}"#, v1);
    let r = http(port, "PUT", &format!("/api/v1/rules/{}", uuid), "127.0.0.1", JSON_CT, Some(body.as_str())).unwrap();
    assert!(r.contains("200 OK") && r.contains("ルールを更新"), "{}", r);
    let v2 = json_fields(&r, "version")[0].clone();
    assert_ne!(v1, v2);

    // stale version -> 409 and nothing changes
    let r = http(port, "PUT", &format!("/api/v1/rules/{}", uuid), "127.0.0.1", JSON_CT, Some(body.as_str())).unwrap();
    assert!(r.contains("409") && r.contains("conflict"), "{}", r);
    let r = get(&format!("/api/v1/rules/{}", uuid));
    assert!(r.contains("v2 有給"), "{}", r);

    // history, revisions
    let h = get(&format!("/api/v1/rules/{}/history", uuid));
    let hashes = json_fields(&h, "hash");
    assert_eq!(hashes.len(), 2, "{}", h);
    let old = get(&format!("/api/v1/rules/{}/revisions/{}", uuid, hashes[1]));
    assert!(old.contains("200 OK") && old.contains("v1") && !old.contains("v2 有給"), "{}", old);
    assert!(get(&format!("/api/v1/rules/{}/revisions/zzzz", uuid)).contains("400"));
    assert!(get(&format!("/api/v1/rules/{}/revisions/{}", uuid, "0".repeat(40))).contains("404"));

    // search + incremental changes
    let r = get("/api/v1/search?q=%E6%9C%89%E7%B5%A6");
    assert!(r.contains(uuid.as_str()) && r.contains("\"kind\":\"rule\""), "{}", r);
    assert!(get("/api/v1/search?q=").contains("400"));
    let all = get("/api/v1/changes");
    assert!(all.contains("\"files\"") && all.contains(&format!("rules/{}.xml", uuid)), "{}", all);
    let newer = get(&format!("/api/v1/changes?since={}", hashes[1]));
    assert!(newer.contains("ルールを更新") && !newer.contains("ルールを追加"), "{}", newer);
    assert!(get("/api/v1/changes?since=zz").contains("400"));

    // delete with a stale version is refused, with the right one it is committed
    let r = http(port, "DELETE", &format!("/api/v1/rules/{}?if_version={}", uuid, v1), "127.0.0.1", "", None).unwrap();
    assert!(r.contains("409"), "{}", r);
    let r = http(port, "DELETE", &format!("/api/v1/rules/{}?if_version={}", uuid, v2), "127.0.0.1", "", None).unwrap();
    assert!(r.contains("200 OK") && r.contains("ルールを削除"), "{}", r);
    let h = get(&format!("/api/v1/rules/{}/history", uuid));
    assert_eq!(json_fields(&h, "hash").len(), 3, "{}", h);

    // custom commit message, and OpenAPI document
    let r = http(port, "POST", "/api/v1/laws", "127.0.0.1", JSON_CT, Some(r#"{"name":"新法","type":"法律","commit_message":"API経由で制定"}"#)).unwrap();
    assert!(r.contains("API経由で制定"), "{}", r);
    let r = get("/api/v1/openapi.json");
    assert!(r.contains("200 OK") && r.contains("\"openapi\":\"3.0.3\"") && !r.contains("\"ok\":true"), "{}", r);
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn http_cors_is_opt_in_and_needs_token() {
    let repo = init_repo("cors");
    for bad in [&["--cors-origin", "http://app.example"][..], &["--cors-origin", "*", "--token", "t"][..]] {
        let out = Command::new(BIN)
            .arg("--repo")
            .arg(&repo)
            .arg("serve")
            .args(["--port", "0"])
            .args(bad)
            .env_remove("LAWGIT_TOKEN")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{:?}", bad);
    }

    let (_srv, port) = start_server(&repo, &["--token", "t", "--cors-origin", "http://app.example"], &[]);
    let pre = "Origin: http://app.example\r\nAccess-Control-Request-Method: POST\r\n";
    let r = http(port, "OPTIONS", "/api/v1/rules", "127.0.0.1", pre, None).unwrap();
    assert!(r.contains("204") && r.contains("Access-Control-Allow-Origin: http://app.example"), "{}", r);
    assert!(r.contains("Access-Control-Allow-Headers") && r.contains("Authorization"), "{}", r);
    let r = http(port, "OPTIONS", "/api/v1/rules", "127.0.0.1", "Origin: http://evil.example\r\n", None).unwrap();
    assert!(r.contains("403") && !r.contains("Access-Control-Allow-Origin"), "{}", r);
    let r = http(
        port,
        "GET",
        "/api/v1/ping",
        "127.0.0.1",
        "Origin: http://app.example\r\nAuthorization: Bearer t\r\n",
        None,
    )
    .unwrap();
    assert!(r.contains("200 OK") && r.contains("Access-Control-Allow-Origin: http://app.example"), "{}", r);
    let r = http(port, "GET", "/api/v1/ping", "127.0.0.1", "Origin: http://evil.example\r\nAuthorization: Bearer t\r\n", None).unwrap();
    assert!(r.contains("200 OK") && !r.contains("Access-Control-Allow-Origin"), "{}", r);
    let _ = fs::remove_dir_all(&repo);
}
