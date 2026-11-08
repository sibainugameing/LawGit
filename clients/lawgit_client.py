#!/usr/bin/env python3
"""lawgit_client.py - tiny dependency-free client for the `lawgit serve` HTTP API.

    from lawgit_client import LawGitClient
    c = LawGitClient("http://127.0.0.1:8787", token=None)
    rule = c.create_rule("休暇規則", category="人事", body="# 休暇")
    c.edit_rule(rule["uuid"], lambda r: {"body": r["body"] + "\n- 追記"})   # conflict-safe edit
    for commit in c.watch():                                                  # follow new commits
        print(commit["subject"], [f["path"] for f in commit["files"]])

Errors raise LawGitError (status, message); 404 -> NotFound, 409 -> Conflict.
"""
from __future__ import annotations

import json
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Iterator, Optional


class LawGitError(Exception):
    def __init__(self, message: str, status: int = 0):
        super().__init__(message)
        self.status = status
        self.message = message


class NotFound(LawGitError):
    pass


class Conflict(LawGitError):
    pass


class LawGitClient:
    def __init__(self, base_url: str = "http://127.0.0.1:8787", token: Optional[str] = None, timeout: float = 15.0):
        self.base = base_url.rstrip("/") + "/api/v1"
        self.token = token
        self.timeout = timeout

    # ------------------------------------------------------------ transport
    def _request(self, method: str, path: str, query: Optional[dict] = None, body: Optional[dict] = None, raw: bool = False):
        url = self.base + path
        if query:
            q = {k: v for k, v in query.items() if v is not None}
            if q:
                url += "?" + urllib.parse.urlencode(q)
        data = None
        headers = {"Accept": "application/json"}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        if body is not None:
            data = json.dumps({k: v for k, v in body.items() if v is not None}).encode("utf-8")
            headers["Content-Type"] = "application/json"
        elif method in ("POST", "PUT", "PATCH"):
            data = b"{}"
            headers["Content-Type"] = "application/json"
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                payload = resp.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            payload = e.read().decode("utf-8", "replace")
            status = e.code
        except (urllib.error.URLError, OSError) as e:
            raise LawGitError(f"cannot reach {url}: {e}")
        else:
            status = 200
        try:
            doc = json.loads(payload)
        except ValueError:
            raise LawGitError(f"non-JSON response (HTTP {status})", status)
        if raw and status == 200:
            return doc
        if not isinstance(doc, dict) or doc.get("ok") is not True:
            msg = doc.get("error", "unknown error") if isinstance(doc, dict) else "unknown error"
            cls = NotFound if status == 404 else Conflict if status == 409 else LawGitError
            raise cls(msg, status)
        return doc["result"]

    def rpc(self, method: str, params: Optional[dict] = None):
        return self._request("POST", "/rpc", body={"method": method, "params": params or {}})

    # ------------------------------------------------------------ general
    def ping(self):
        return self._request("GET", "/ping")

    def validate(self):
        return self._request("GET", "/validate")

    def openapi(self) -> dict:
        return self._request("GET", "/openapi.json", raw=True)

    def search(self, q: str, limit: Optional[int] = None):
        return self._request("GET", "/search", {"q": q, "limit": limit})["hits"]

    def changes(self, since: Optional[str] = None, limit: Optional[int] = None):
        return self._request("GET", "/changes", {"since": since, "limit": limit})

    def git_status(self):
        return self._request("GET", "/git/status")

    def git_log(self, limit: Optional[int] = None):
        return self._request("GET", "/git/log", {"limit": limit})["commits"]

    def commit(self, message: str):
        return self._request("POST", "/git/commit", body={"message": message})

    # ------------------------------------------------------------ laws
    def laws(self):
        return self._request("GET", "/laws")["laws"]

    def law(self, uuid: str):
        return self._request("GET", f"/laws/{uuid}")

    def create_law(self, name: str, type: str, law_number: Optional[str] = None,
                   e_gov_id: Optional[str] = None, commit_message: Optional[str] = None):
        return self._request("POST", "/laws", body={"name": name, "type": type, "law_number": law_number,
                                                    "e_gov_id": e_gov_id, "commit_message": commit_message})

    # ------------------------------------------------------------ rules
    def rules(self, category: Optional[str] = None, query: Optional[str] = None):
        return self._request("GET", "/rules", {"category": category, "query": query})["rules"]

    def rule(self, uuid: str):
        return self._request("GET", f"/rules/{uuid}")

    def create_rule(self, name: str, category: Optional[str] = None, body: str = "",
                    commit_message: Optional[str] = None):
        return self._request("POST", "/rules", body={"name": name, "category": category, "body": body,
                                                     "commit_message": commit_message})

    def update_rule(self, uuid: str, name: Optional[str] = None, category: Optional[str] = None,
                    body: Optional[str] = None, if_version: Optional[str] = None,
                    commit_message: Optional[str] = None):
        return self._request("PUT", f"/rules/{uuid}", body={"name": name, "category": category, "body": body,
                                                            "if_version": if_version,
                                                            "commit_message": commit_message})

    def delete_rule(self, uuid: str, if_version: Optional[str] = None, commit_message: Optional[str] = None):
        return self._request("DELETE", f"/rules/{uuid}", {"if_version": if_version, "commit_message": commit_message})

    def edit_rule(self, uuid: str, change: Callable[[dict], Optional[dict]], retries: int = 5,
                  commit_message: Optional[str] = None):
        """Read-modify-write without lost updates: `change(rule)` returns the fields to update
        (name / category / body) or None to do nothing. Retries when someone else edited meanwhile."""
        for attempt in range(retries):
            current = self.rule(uuid)
            fields = change(current)
            if not fields:
                return current
            try:
                return self.update_rule(uuid, if_version=current["version"], commit_message=commit_message,
                                        **{k: v for k, v in fields.items() if k in ("name", "category", "body")})
            except Conflict:
                if attempt == retries - 1:
                    raise
        raise Conflict("too many conflicting edits", 409)

    # ------------------------------------------------------------ history
    def history(self, kind: str, uuid: str, limit: Optional[int] = None):
        if kind not in ("laws", "rules"):
            raise ValueError("kind must be 'laws' or 'rules'")
        return self._request("GET", f"/{kind}/{uuid}/history", {"limit": limit})["commits"]

    def revision(self, kind: str, uuid: str, rev: str) -> str:
        if kind not in ("laws", "rules"):
            raise ValueError("kind must be 'laws' or 'rules'")
        return self._request("GET", f"/{kind}/{uuid}/revisions/{rev}")["xml"]

    def watch(self, since: Optional[str] = None, interval: float = 2.0,
              stop: Optional[Callable[[], bool]] = None, from_beginning: bool = False) -> Iterator[dict]:
        """Poll /changes and yield each new commit (oldest first). By default starts from "now"."""
        if since is None and not from_beginning:
            since = self.changes(limit=1).get("head")
        while True:
            res = self.changes(since=since, limit=500) if since else self.changes(limit=500)
            for commit in reversed(res["commits"]):
                yield commit
            if res.get("head"):
                since = res["head"]
            if stop is not None and stop():
                return
            time.sleep(interval)
