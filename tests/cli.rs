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

fn start_server(repo: &Path, extra: &[&str]) -> (Kill, u16) {
    let port = free_port();
    let child = Command::new(BIN)
        .arg("--repo")
        .arg(repo)
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .args(extra)
        .env_remove("LAWGIT_TOKEN")
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
    let (_srv, port) = start_server(&repo, &[]);

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
    let (_srv, port) = start_server(&repo, &["--token", "s3cret"]);
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
