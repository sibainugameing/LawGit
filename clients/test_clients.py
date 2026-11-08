#!/usr/bin/env python3
"""Tests for the Python client against an in-process mock of the lawgit HTTP API."""
import json, sys, threading, unittest, urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
from lawgit_client import LawGitClient, LawGitError, NotFound, Conflict


class Mock(BaseHTTPRequestHandler):
    log = []
    version = 1
    changes_calls = 0

    def log_message(self, *a): pass

    def _send(self, code, doc):
        raw = json.dumps(doc, ensure_ascii=False).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw))); self.end_headers(); self.wfile.write(raw)

    def _handle(self):
        u = urllib.parse.urlparse(self.path); q = dict(urllib.parse.parse_qsl(u.query))
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n)) if n else None
        Mock.log.append((self.command, u.path, q, body, self.headers.get("Authorization"), self.headers.get("Content-Type")))
        if self.headers.get("Authorization") != "Bearer tok":
            return self._send(401, {"ok": False, "error": "unauthorized"})
        p = u.path
        if p == "/api/v1/ping": return self._send(200, {"ok": True, "result": {"name": "lawgit"}})
        if p == "/api/v1/openapi.json": return self._send(200, {"openapi": "3.0.3"})
        if p == "/api/v1/search": return self._send(200, {"ok": True, "result": {"hits": [{"uuid": "u", "q": q.get("q")}]}})
        if p == "/api/v1/rules/missing": return self._send(404, {"ok": False, "error": "not found: rule"})
        if p == "/api/v1/rules/u1" and self.command == "GET":
            return self._send(200, {"ok": True, "result": {"uuid": "u1", "body": "b", "version": f"v{Mock.version}"}})
        if p == "/api/v1/rules/u1" and self.command == "PUT":
            if body.get("if_version") != f"v{Mock.version}":
                return self._send(409, {"ok": False, "error": "conflict: modified"})
            Mock.version += 1
            return self._send(200, {"ok": True, "result": {"uuid": "u1", "body": body.get("body"), "version": f"v{Mock.version}"}})
        if p == "/api/v1/changes":
            Mock.changes_calls += 1
            if q.get("limit") == "1":
                return self._send(200, {"ok": True, "result": {"head": "h1", "commits": []}})
            if q.get("since") == "h1":
                return self._send(200, {"ok": True, "result": {"head": "h3", "commits": [
                    {"hash": "h3", "subject": "second", "files": []}, {"hash": "h2", "subject": "first", "files": []}]}})
            return self._send(200, {"ok": True, "result": {"head": "h3", "commits": []}})
        return self._send(404, {"ok": False, "error": "not found: route"})

    do_GET = do_POST = do_PUT = do_DELETE = _handle


class ClientTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = HTTPServer(("127.0.0.1", 0), Mock)
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()
        cls.url = f"http://127.0.0.1:{cls.srv.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        Mock.log.clear(); Mock.version = 1
        self.c = LawGitClient(self.url, token="tok")

    def test_auth_and_query_encoding(self):
        self.assertEqual(self.c.ping()["name"], "lawgit")
        hits = self.c.search("有給 休暇", limit=5)
        self.assertEqual(hits[0]["q"], "有給 休暇")
        self.assertEqual(Mock.log[-1][2], {"q": "有給 休暇", "limit": "5"})
        self.assertEqual(Mock.log[-1][4], "Bearer tok")

    def test_wrong_token_is_error(self):
        with self.assertRaises(LawGitError) as cm:
            LawGitClient(self.url, token="bad").ping()
        self.assertEqual(cm.exception.status, 401)

    def test_not_found_and_raw(self):
        with self.assertRaises(NotFound):
            self.c.rule("missing")
        self.assertEqual(self.c.openapi()["openapi"], "3.0.3")

    def test_unreachable(self):
        with self.assertRaises(LawGitError):
            LawGitClient("http://127.0.0.1:1", timeout=1).ping()

    def test_edit_rule_retries_on_conflict(self):
        calls = []
        def change(r):
            calls.append(r["version"])
            if len(calls) == 1:
                Mock.version += 5            # somebody else edits after our read
            return {"body": "new"}
        out = self.c.edit_rule("u1", change)
        self.assertEqual(len(calls), 2)
        self.assertEqual(out["body"], "new")
        puts = [e for e in Mock.log if e[0] == "PUT"]
        self.assertEqual([p[3]["if_version"] for p in puts], ["v1", "v6"])
        self.assertEqual(puts[0][5], "application/json")

    def test_edit_rule_gives_up_and_noop(self):
        def always_stale(r):
            Mock.version += 1
            return {"body": "x"}
        with self.assertRaises(Conflict):
            self.c.edit_rule("u1", always_stale, retries=2)
        Mock.log.clear()
        self.c.edit_rule("u1", lambda r: None)
        self.assertFalse([e for e in Mock.log if e[0] == "PUT"])

    def test_watch_yields_oldest_first(self):
        stop = {"n": 0}
        def should_stop():
            stop["n"] += 1
            return True
        got = list(self.c.watch(interval=0, stop=should_stop))
        self.assertEqual([c["subject"] for c in got], ["first", "second"])

    def test_bad_kind(self):
        with self.assertRaises(ValueError):
            self.c.history("oops", "u")


if __name__ == "__main__":
    unittest.main()
