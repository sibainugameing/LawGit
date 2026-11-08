#!/usr/bin/env python3
"""egov_import.py - e-Gov 法令 -> lawgit 形式 XML 変換・取り込みツール (独立スクリプト)

Rust 製の lawgit 本体にはネットワーク機能がありません。外部データの取得はこのスクリプトだけが
行い、書き込み先は <repo>/laws/<UUID>.xml のみです。標準ライブラリだけで動作します (Python 3.9+)。

安全方針:
  * 既定は dry-run (何も書かない)。実際に書くには --write が必要。
  * 通信は https://laws.e-gov.go.jp のみ。別ホストへのリダイレクトは拒否。サイズ上限・タイムアウトあり。
  * DOCTYPE / ENTITY を含む XML は解析前に拒否 (XXE / entity 展開攻撃対策)。
  * 取得元データは「データ」として扱い、実行・評価しない。
  * 既存法令は --update を付けない限り上書きしない。UUID は既存のものを保持 (不変)。
  * 自動 commit はしない。書き込み後は `lawgit validate` で検証し、Git で差分を確認してから commit。
  * ネットワークを使いたくない場合: e-Gov から手動で XML を保存し --from-file で変換のみ行う。

使い方:
  python3 tools/egov_import.py --repo legal-data 415AC0000000057            # dry-run
  python3 tools/egov_import.py --repo legal-data 415AC0000000057 --write
  python3 tools/egov_import.py --repo legal-data --from-file law.xml --write # オフライン
  python3 tools/egov_import.py --repo legal-data 335AC0000000105 --asof 2025-04-01 --write --update
  python3 tools/egov_import.py --repo legal-data --all --max 5 --write --yes     # 全法令 (まず少数で試す)
  python3 tools/egov_import.py --repo legal-data 415AC0000000057 --history --write  # 改正履歴を Git に復元

本文 (<body>) は e-Gov 法令 XML の LawBody 配下をそのまま構造要素として保持します
(要素語彙は e-Gov のもの。lawgit 側の語彙定義は別途仕様化の課題)。
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

API_BASE = "https://laws.e-gov.go.jp/api/2"
ALLOWED_HOST = "laws.e-gov.go.jp"
MAX_BYTES = 64 * 1024 * 1024
TIMEOUT = 60
LIST_PAGE_SIZE = 100
MAX_LIST_PAGES = 500
MAX_CONSECUTIVE_FAILURES = 10
MIN_DELAY = 0.5
RETRIES = 3
JST = datetime.timezone(datetime.timedelta(hours=9))
HISTORY_AUTHOR = ("e-Gov Import", "egov-import@lawgit.invalid")
LAW_ID_RE = re.compile(r"^[0-9A-Za-z]{1,32}$")
REVISION_ID_RE = re.compile(r"^[0-9A-Za-z]{1,32}_\d{8}_[0-9A-Za-z]{1,32}$")
UUID_V4_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
ASOF_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")

LAW_TYPE_MAP = {
    "Constitution": "憲法",
    "Act": "法律",
    "CabinetOrder": "政令",
    "ImperialOrdinance": "勅令",
    "MinisterialOrdinance": "省令",
    "Rule": "規則",
    "Misc": "その他",
}


class ImportError_(Exception):
    pass


# ---------------------------------------------------------------- fetching
class _SameHostRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        p = urllib.parse.urlparse(newurl)
        if p.scheme != "https" or p.hostname != ALLOWED_HOST:
            raise ImportError_(f"refusing redirect to {newurl}")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _get(url: str, accept: str) -> bytes:
    p = urllib.parse.urlparse(url)
    if p.scheme != "https" or p.hostname != ALLOWED_HOST:
        raise ImportError_(f"refusing to contact {url}")
    opener = urllib.request.build_opener(_SameHostRedirect())
    req = urllib.request.Request(url, headers={"User-Agent": "lawgit-egov-import/0.2", "Accept": accept})
    try:
        with opener.open(req, timeout=TIMEOUT) as resp:
            data = resp.read(MAX_BYTES + 1)
    except urllib.error.HTTPError as e:
        raise ImportError_(f"e-Gov returned HTTP {e.code} for {p.path}")
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        raise ImportError_(f"network error for {p.path}: {e}")
    if len(data) > MAX_BYTES:
        raise ImportError_(f"response for {p.path} exceeds {MAX_BYTES} bytes")
    return data


def fetch_law_xml(law_id: str, asof: str | None) -> bytes:
    if not (LAW_ID_RE.match(law_id) or REVISION_ID_RE.match(law_id)):
        raise ImportError_(f"invalid law id / revision id: {law_id!r}")
    url = f"{API_BASE}/law_file/xml/{urllib.parse.quote(law_id)}"
    if asof:
        if not ASOF_RE.match(asof):
            raise ImportError_("--asof must be YYYY-MM-DD")
        url += "?" + urllib.parse.urlencode({"asof": asof})
    return _get(url, "application/xml")


def validate_promulgation_range(promulgated_from: str | None, promulgated_to: str | None) -> None:
    """Validate inclusive YYYY-MM-DD bounds for the /laws promulgation-date filters."""
    parsed = {}
    for option, value in (("--promulgated-from", promulgated_from),
                          ("--promulgated-to", promulgated_to)):
        if value is None:
            continue
        if not ASOF_RE.match(value):
            raise ImportError_(f"{option} must be YYYY-MM-DD")
        try:
            parsed[option] = datetime.date.fromisoformat(value)
        except ValueError:
            raise ImportError_(f"{option} is not a valid calendar date: {value}")
    if ("--promulgated-from" in parsed and "--promulgated-to" in parsed
            and parsed["--promulgated-from"] > parsed["--promulgated-to"]):
        raise ImportError_("--promulgated-from must be on or before --promulgated-to")


def list_all_law_ids(asof: str | None = None, max_items: int | None = None, delay: float = 1.0,
                     promulgated_from: str | None = None,
                     promulgated_to: str | None = None) -> list[str]:
    """Page through GET /laws and collect IDs matching optional inclusive promulgation-date bounds."""
    if asof and not ASOF_RE.match(asof):
        raise ImportError_("--asof must be YYYY-MM-DD")
    validate_promulgation_range(promulgated_from, promulgated_to)
    ids: list[str] = []
    seen: set[str] = set()
    offset = 0
    for page in range(MAX_LIST_PAGES):
        q = {"limit": LIST_PAGE_SIZE, "offset": offset, "response_format": "json"}
        if asof:
            q["asof"] = asof
        if promulgated_from:
            q["promulgation_date_from"] = promulgated_from
        if promulgated_to:
            q["promulgation_date_to"] = promulgated_to
        raw = _get(f"{API_BASE}/laws?" + urllib.parse.urlencode(q), "application/json")
        try:
            data = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as e:
            raise ImportError_(f"law list is not valid JSON: {e}")
        laws = data.get("laws") if isinstance(data, dict) else None
        if not isinstance(laws, list) or not laws:
            break
        for item in laws:
            info = item.get("law_info") if isinstance(item, dict) else None
            lid = info.get("law_id") if isinstance(info, dict) else None
            if isinstance(lid, str) and LAW_ID_RE.match(lid) and lid not in seen:
                seen.add(lid)
                ids.append(lid)
        if max_items is not None and len(ids) >= max_items:
            break
        nxt = data.get("next_offset")
        if not isinstance(nxt, int) or nxt <= offset:
            break
        offset = nxt
        time.sleep(delay)
    return ids[:max_items] if max_items is not None else ids


# ---------------------------------------------------------------- parsing
def parse_untrusted_xml(data: bytes) -> ET.Element:
    head = data[:]  # scan whole payload; DOCTYPE may appear after comments
    if re.search(rb"<!\s*(DOCTYPE|ENTITY)", head, re.IGNORECASE):
        raise ImportError_("XML with DOCTYPE/ENTITY declarations is rejected")
    try:
        return ET.fromstring(data)
    except ET.ParseError as e:
        raise ImportError_(f"XML parse error: {e}")


def find_law_element(root: ET.Element) -> ET.Element:
    if root.tag == "Law":
        return root
    for el in root.iter("Law"):
        return el
    raise ImportError_("no <Law> element found in the data")


def text_of(el: ET.Element | None) -> str:
    if el is None:
        return ""
    return "".join(el.itertext()).strip()


def one_line(s: str) -> str:
    return re.sub(r"\s+", " ", s).strip()


def law_metadata(law: ET.Element) -> dict:
    body = law.find("LawBody")
    if body is None:
        raise ImportError_("<Law> has no <LawBody>")
    name = one_line(text_of(body.find("LawTitle")))
    if not name:
        raise ImportError_("law title (LawBody/LawTitle) is empty")
    num = one_line(text_of(law.find("LawNum")))
    ltype = LAW_TYPE_MAP.get(law.attrib.get("LawType", ""), "その他")
    return {"name": name, "law_number": num, "type": ltype}


# ---------------------------------------------------------------- serializing
def _inline(el: ET.Element) -> str:
    """Serialize an element (without its tail) on one line."""
    saved = el.tail
    el.tail = None
    try:
        s = ET.tostring(el, encoding="unicode", short_empty_elements=True)
    finally:
        el.tail = saved
    return s


def _is_structural(el: ET.Element) -> bool:
    """True if the element has child elements and no meaningful character data."""
    if len(el) == 0:
        return False
    if el.text and el.text.strip():
        return False
    return not any(c.tail and c.tail.strip() for c in el)


def serialize(el: ET.Element, depth: int, out: list[str]) -> None:
    pad = "    " * depth
    if not _is_structural(el):
        out.append(pad + _inline(el))
        return
    attrs = "".join(f" {k}={quoteattr(v)}" for k, v in el.attrib.items())
    out.append(f"{pad}<{el.tag}{attrs}>")
    for c in el:
        serialize(c, depth + 1, out)
    out.append(f"{pad}</{el.tag}>")


def build_lawgit_xml(law: ET.Element, uuid_s: str, egov_id: str) -> str:
    meta = law_metadata(law)
    body = law.find("LawBody")
    assert body is not None
    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        "<law>",
        "    <identifiers>",
        f"        <uuid>{uuid_s}</uuid>",
        f"        <e-gov-id>{escape(egov_id)}</e-gov-id>",
        "    </identifiers>",
        "",
        "    <metadata>",
        f"        <name>{escape(meta['name'])}</name>",
    ]
    if meta["law_number"]:
        lines.append(f"        <law-number>{escape(meta['law_number'])}</law-number>")
    lines += [f"        <type>{escape(meta['type'])}</type>", "    </metadata>", "", "    <body>"]
    for child in body:
        serialize(child, 2, lines)
    lines += ["    </body>", "</law>", ""]
    return "\n".join(lines)


def self_check(text: str, uuid_s: str, egov_id: str) -> None:
    """Re-parse the generated document and verify the standard's requirements."""
    if "\r" in text:
        raise ImportError_("generated text contains CR")
    root = ET.fromstring(text)
    if root.tag != "law":
        raise ImportError_("root is not <law>")
    u = text_of(root.find("identifiers/uuid"))
    if u != uuid_s or not UUID_V4_RE.match(u):
        raise ImportError_("UUID check failed")
    if text_of(root.find("identifiers/e-gov-id")) != egov_id or not egov_id:
        raise ImportError_("e-gov-id check failed")
    if not text_of(root.find("metadata/name")) or not text_of(root.find("metadata/type")):
        raise ImportError_("metadata check failed")
    body = root.find("body")
    if body is None or len(body) == 0:
        raise ImportError_("body has no structural elements")
    if (body.text and body.text.strip()) or any(c.tail and c.tail.strip() for c in body):
        raise ImportError_("plain text directly under <body>")


# ---------------------------------------------------------------- repository
def existing_by_egov_id(laws_dir: Path) -> dict[str, Path]:
    """Map e-Gov ID -> existing file, using a safe parse of each lawgit file."""
    found: dict[str, Path] = {}
    for p in sorted(laws_dir.glob("*.xml")):
        try:
            data = p.read_bytes()
            if re.search(rb"<!\s*(DOCTYPE|ENTITY)", data, re.IGNORECASE):
                continue
            root = ET.fromstring(data)
        except (ET.ParseError, OSError):
            continue
        eid = text_of(root.find("identifiers/e-gov-id"))
        if eid:
            found.setdefault(eid, p)
    return found


def fresh_uuid(repo: Path) -> str:
    for _ in range(16):
        u = str(uuid.uuid4())
        if not (repo / "laws" / f"{u}.xml").exists() and not (repo / "rules" / f"{u}.xml").exists():
            return u
    raise ImportError_("could not generate an unused UUID")


def write_atomic(path: Path, text: str) -> None:
    tmp = path.with_name(path.name + ".tmp")
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    os.replace(tmp, path)


def import_one(repo: Path, egov_id: str, data: bytes, write: bool, update: bool, known: dict[str, Path]) -> str:
    law = find_law_element(parse_untrusted_xml(data))
    laws_dir = repo / "laws"
    existing = known.get(egov_id)
    if existing is not None:
        uuid_s = existing.stem
        if not UUID_V4_RE.match(uuid_s):
            raise ImportError_(f"existing file {existing.name} has a non-UUID name; fix it first")
        action = "update" if update else "skip"
    else:
        uuid_s = fresh_uuid(repo)
        action = "create"
    text = build_lawgit_xml(law, uuid_s, egov_id)
    self_check(text, uuid_s, egov_id)
    name = law_metadata(law)["name"]
    if action == "skip":
        return f"SKIP    {egov_id} {name}: already imported as {uuid_s} (use --update to overwrite)"
    target = laws_dir / f"{uuid_s}.xml"
    if target.resolve().parent != laws_dir.resolve():
        raise ImportError_("refusing to write outside laws/")
    if not write:
        return f"DRY-RUN {action} {egov_id} {name} -> laws/{uuid_s}.xml ({len(text)} bytes)"
    write_atomic(target, text)
    known[egov_id] = target
    return f"{action.upper():7} {egov_id} {name} -> laws/{uuid_s}.xml"


# ---------------------------------------------------------------- amendment history -> Git
def clean_text(v, limit: int = 200) -> str:
    if not isinstance(v, str):
        return ""
    return re.sub(r"[\x00-\x1f\x7f]+", " ", v).strip()[:limit]


def norm_date(v) -> str:
    if isinstance(v, str) and ASOF_RE.match(v):
        try:
            datetime.date.fromisoformat(v)
            return v
        except ValueError:
            return ""
    return ""


def _clean_record(law_id: str, info: dict):
    """Validate one revision record (from the API or from the local cache)."""
    rid = info.get("law_revision_id") or info.get("rid")
    if not isinstance(rid, str) or not REVISION_ID_RE.match(rid) or rid.split("_")[0] != law_id:
        return None
    ymd = rid.split("_")[1]
    enf = norm_date(info.get("amendment_enforcement_date") or info.get("enf")) or norm_date(f"{ymd[:4]}-{ymd[4:6]}-{ymd[6:]}")
    if not enf:
        return None
    aid = clean_text(info.get("amendment_law_id") or info.get("amend_id"), 64)
    return {
        "law_id": law_id,
        "rid": rid,
        "enf": enf,
        "prom": norm_date(info.get("amendment_promulgate_date") or info.get("prom")),
        "amend_id": aid if LAW_ID_RE.match(aid) else "",
        "amend_title": clean_text(info.get("amendment_law_title") or info.get("amend_title")),
        "amend_num": clean_text(info.get("amendment_law_num") or info.get("amend_num")),
        "status": clean_text(info.get("current_revision_status") or info.get("status"), 40),
        "title": clean_text(info.get("law_title") or info.get("title")),
    }


def parse_revisions(data, law_id: str) -> list:
    revs = data.get("revisions") if isinstance(data, dict) else None
    if not isinstance(revs, list):
        raise ImportError_(f"revision list for {law_id} has an unexpected format")
    out = []
    for r in revs:
        if not isinstance(r, dict):
            continue
        info = r.get("revision_info")
        rec = _clean_record(law_id, info if isinstance(info, dict) else r)
        if rec:
            out.append(rec)
    if not out:
        raise ImportError_(f"no usable revisions for {law_id}")
    return out


def _retry(fn):
    """Retry transient failures; never retry HTTP 4xx."""
    last = None
    for attempt in range(RETRIES):
        try:
            return fn()
        except ImportError_ as e:
            last = e
            if "HTTP 4" in str(e) or attempt == RETRIES - 1:
                break
            time.sleep(min(30, 2 * (attempt + 1)))
    raise last


def fetch_revision_list(law_id: str, cache_dir, refresh: bool):
    """Return (records, used_network). Records are cached under .git/ (never in the work tree)."""
    if not LAW_ID_RE.match(law_id):
        raise ImportError_(f"invalid law id: {law_id!r}")
    cache = cache_dir / f"{law_id}.json" if cache_dir else None
    if cache is not None and cache.is_file() and not refresh:
        try:
            cached = json.loads(cache.read_text(encoding="utf-8"))
            recs = [r for r in (_clean_record(law_id, c) for c in cached if isinstance(c, dict)) if r]
            if recs:
                return recs, False
        except (OSError, ValueError, TypeError):
            pass
    raw = _retry(lambda: _get(f"{API_BASE}/law_revisions/{urllib.parse.quote(law_id)}", "application/json"))
    try:
        data = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeDecodeError) as e:
        raise ImportError_(f"revision list for {law_id} is not valid JSON: {e}")
    recs = parse_revisions(data, law_id)
    if cache is not None:
        cache_dir.mkdir(parents=True, exist_ok=True)
        write_atomic(cache, json.dumps(recs, ensure_ascii=False))
    return recs, True


def git(repo: Path, *args: str, env_extra=None, input_text=None) -> str:
    env = {k: v for k, v in os.environ.items()
           if k not in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR")}
    env.update(env_extra or {})
    try:
        r = subprocess.run(["git", "-C", str(repo), *args], input=input_text, capture_output=True,
                           text=True, encoding="utf-8", env=env, check=False)
    except OSError as e:
        raise ImportError_(f"cannot run git: {e}")
    if r.returncode != 0:
        raise ImportError_(f"git {args[0]} failed: {(r.stderr or r.stdout).strip()}")
    return r.stdout


def history_preflight(repo: Path) -> None:
    if not (repo / ".git").exists():
        raise ImportError_("--history --write needs a Git repository (run `lawgit init` first)")
    top = git(repo, "rev-parse", "--show-toplevel").strip()
    if Path(top).resolve() != repo.resolve():
        raise ImportError_(f"{repo} is not the top level of its Git repository ({top}); refusing to commit there")
    for line in git(repo, "status", "--porcelain=v1", "--untracked-files=all").splitlines():
        path = line[3:]
        if path.endswith(".gitkeep"):
            continue
        if line[:2] != "??" and line[0] != " ":
            raise ImportError_("there are staged changes; commit or unstage them first")
        if path.startswith("laws/") or path.startswith('"laws/'):
            raise ImportError_(f"uncommitted changes in laws/ ({path}); commit or discard them first")


def done_revisions(repo: Path) -> set:
    try:
        out = git(repo, "log", "--format=%B")
    except ImportError_ as e:
        if "does not have any commits" in str(e) or "bad default revision" in str(e):
            return set()
        raise
    return {l.split(":", 1)[1].strip() for l in out.splitlines() if l.startswith("E-Gov-Revision:")}


def epoch_for(date_s: str) -> int:
    y, m, d = (int(x) for x in date_s.split("-"))
    ts = int(datetime.datetime(y, m, d, tzinfo=JST).timestamp())
    return max(ts, 86400)  # Git cannot represent dates before 1970; the real date stays in the message


def group_key(r: dict) -> tuple:
    if r["amend_id"]:
        return (r["enf"], r["prom"], r["amend_id"], "")
    return (r["enf"], r["prom"], "", r["law_id"])


def make_groups(recs: list) -> list:
    recs = sorted(recs, key=lambda r: group_key(r) + (r["law_id"], r["rid"]))
    groups: list = []
    last = None
    for r in recs:
        k = group_key(r)
        if k != last:
            groups.append([])
            last = k
        groups[-1].append(r)
    return groups


def build_message(group: list, names: dict) -> str:
    first = group[0]
    n = len(group)
    amend = first["amend_title"] or first["amend_num"]
    titles = [names.get(r["rid"]) or r["title"] or r["law_id"] for r in group]
    if first["amend_id"] or amend:
        subject = f"{titles[0]}: {amend or '改正'}による改正" if n == 1 else f"{amend or '改正'}による改正 ({n}法令)"
    else:
        subject = f"{titles[0]}: 制定・初出版" if n == 1 else f"制定・初出版 ({n}法令)"
    lines = [clean_text(subject), ""]
    lines += [f"- {t} [{r['law_id']}]" for r, t in zip(group, titles)]
    lines.append("")
    if first["amend_num"] or first["amend_id"]:
        lines.append(f"Amending-Law: {first['amend_num']} {first['amend_id']}".strip())
    if first["prom"]:
        lines.append(f"Promulgated: {first['prom']}")
    lines.append(f"Enforced: {first['enf']}")
    lines += [f"E-Gov-Revision: {r['rid']}" for r in group]
    return "\n".join(lines) + "\n"


def render_revision(data: bytes, egov_id: str, uuid_s: str):
    law = find_law_element(parse_untrusted_xml(data))
    text = build_lawgit_xml(law, uuid_s, egov_id)
    self_check(text, uuid_s, egov_id)
    return text, law_metadata(law)["name"]


def uuid_for(repo: Path, uuid_of: dict, law_id: str) -> str:
    if law_id not in uuid_of:
        u = fresh_uuid(repo)
        while u in uuid_of.values():
            u = fresh_uuid(repo)
        uuid_of[law_id] = u
    return uuid_of[law_id]


def run_history(args, repo: Path, law_ids: list, known: dict) -> int:
    """Restore past amendments as Git commits (oldest first, one commit per amending act)."""
    delay = args.delay
    is_git = (repo / ".git").exists()
    cache_dir = repo / ".git" / "lawgit-egov-cache" if (repo / ".git").is_dir() else None
    if args.write:
        history_preflight(repo)
    elif args.all:
        print(f"history plan: would fetch the revision list of {len(law_ids)} law(s) and then every missing "
              f"revision (dry-run: no requests made; try explicit IDs to see a real plan)")
        return 0
    done = done_revisions(repo) if is_git else set()

    recs: list = []
    failures = streak = 0
    for n, law_id in enumerate(law_ids):
        try:
            rs, net = fetch_revision_list(law_id, cache_dir, args.refresh)
            recs.extend(rs)
            streak = 0
            if net:
                time.sleep(delay)
            if (n + 1) % 100 == 0:
                print(f"  revision lists: {n + 1}/{len(law_ids)}", flush=True)
        except ImportError_ as e:
            failures += 1
            streak += 1
            print(f"ERROR   {law_id}: {e}", file=sys.stderr, flush=True)
            if streak >= MAX_CONSECUTIVE_FAILURES:
                print("aborting: too many consecutive failures (re-run later; cached lists are reused)", file=sys.stderr)
                return 1

    today = datetime.date.today().isoformat()
    sel = [r for r in recs
           if r["rid"] not in done
           and (args.include_unenforced or (r["status"] != "UnEnforced" and r["enf"] <= today))]
    groups = make_groups(sel)
    print(f"history: {len(recs)} revision(s) known, {len(recs) - len(sel)} skipped (done/not yet in force), "
          f"{len(sel)} to restore in {len(groups)} commit(s)", flush=True)
    if not args.write:
        for g in groups[:10]:
            print(f"  {g[0]['enf']}  {len(g)} law(s)  {build_message(g, {}).splitlines()[0]}")
        print("\n(dry-run: nothing was downloaded, written or committed; add --write)")
        return 1 if failures else 0

    uuid_of = {}
    for eid, path in known.items():
        if UUID_V4_RE.match(path.stem):
            uuid_of[eid] = path.stem
    committed = 0
    streak = 0
    for gi, group in enumerate(groups):
        items = []
        try:
            for rec in group:
                try:
                    time.sleep(delay)
                    data = _retry(lambda rid=rec["rid"]: fetch_law_xml(rid, None))
                    u = uuid_for(repo, uuid_of, rec["law_id"])
                    text, name = render_revision(data, rec["law_id"], u)
                    items.append((rec, u, text, name))
                    streak = 0
                except ImportError_ as e:
                    failures += 1
                    streak += 1
                    print(f"ERROR   {rec['rid']}: {e}", file=sys.stderr, flush=True)
                    if streak >= MAX_CONSECUTIVE_FAILURES:
                        print("aborting: too many consecutive failures (re-run later; finished commits are kept)", file=sys.stderr)
                        return 1
            changed = []
            for rec, u, text, name in items:
                target = repo / "laws" / f"{u}.xml"
                if target.resolve().parent != (repo / "laws").resolve():
                    raise ImportError_("refusing to write outside laws/")
                if target.exists() and target.read_text(encoding="utf-8") == text:
                    continue
                write_atomic(target, text)
                changed.append(f"laws/{u}.xml")
            if not changed:
                continue
            git(repo, "add", "--", *changed)
            stamp = f"@{epoch_for(group[0]['enf'])} +0900"
            env = {"GIT_AUTHOR_NAME": HISTORY_AUTHOR[0], "GIT_AUTHOR_EMAIL": HISTORY_AUTHOR[1],
                   "GIT_COMMITTER_NAME": HISTORY_AUTHOR[0], "GIT_COMMITTER_EMAIL": HISTORY_AUTHOR[1],
                   "GIT_AUTHOR_DATE": stamp, "GIT_COMMITTER_DATE": stamp}
            msg = build_message([i[0] for i in items], {i[0]["rid"]: i[3] for i in items})
            git(repo, "-c", "commit.gpgsign=false", "commit", "--no-verify", "-q", "-F", "-",
                env_extra=env, input_text=msg)
            committed += 1
            print(f"[{gi + 1}/{len(groups)}] COMMIT {group[0]['enf']} {len(changed)} file(s): {msg.splitlines()[0]}", flush=True)
        except ImportError_ as e:
            print(f"error: {e}", file=sys.stderr)
            print("stopped; fix the problem and re-run (finished revisions are skipped).", file=sys.stderr)
            return 2
        except KeyboardInterrupt:
            print("\ninterrupted; if laws/ has uncommitted changes run `git checkout -- laws` before re-running.", file=sys.stderr)
            return 1
    print(f"\nhistory summary: {committed} commit(s) created, {failures} failure(s)")
    return 1 if failures else 0


def verify_with_lawgit(repo: Path, binary: str) -> bool:
    """Cross-check the written files with the Rust validator (`lawgit check`)."""
    try:
        r = subprocess.run([binary, "--repo", str(repo), "check"], capture_output=True,
                           text=True, encoding="utf-8", check=False)
    except OSError as e:
        print(f"verify: cannot run {binary!r}: {e}", file=sys.stderr)
        return False
    out = (r.stdout or "").strip()
    if r.returncode == 0:
        print(f"verify: {out or 'OK'}")
        return True
    print(f"verify: lawgit check FAILED (exit {r.returncode})", file=sys.stderr)
    if out:
        print(out, file=sys.stderr)
    if r.stderr.strip():
        print(r.stderr.strip(), file=sys.stderr)
    return False


def _main_impl(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Import e-Gov laws into a lawgit repository (separate from the Rust CLI).")
    ap.add_argument("law_ids", nargs="*", help="e-Gov law IDs, e.g. 415AC0000000057")
    ap.add_argument("--repo", default=".", help="lawgit repository (must contain laws/)")
    ap.add_argument("--all", action="store_true",
                    help="import EVERY law listed by e-Gov (thousands of requests; needs --yes together with --write)")
    ap.add_argument("--promulgated-from", metavar="YYYY-MM-DD",
                    help="with list import: include laws promulgated on or after this date (inclusive)")
    ap.add_argument("--promulgated-to", metavar="YYYY-MM-DD",
                    help="with list import: include laws promulgated on or before this date (inclusive)")
    ap.add_argument("--max", type=int, default=None, metavar="N",
                    help="with --all or promulgation-date filters: stop after N matching laws (for trial runs)")
    ap.add_argument("--delay", type=float, default=1.0, metavar="SEC",
                    help=f"seconds between requests (default 1.0, minimum {MIN_DELAY})")
    ap.add_argument("--yes", action="store_true", help="confirm a bulk --all --write run")
    ap.add_argument("--from-file", help="convert a local e-Gov XML file instead of downloading (offline)")
    ap.add_argument("--law-id", help="e-Gov law ID to record when using --from-file")
    ap.add_argument("--asof", help="point-in-time snapshot YYYY-MM-DD (enforcement date)")
    ap.add_argument("--history", action="store_true",
                    help="restore past amendments as Git commits (oldest first; one commit per amending act)")
    ap.add_argument("--include-unenforced", action="store_true", help="with --history: also restore not-yet-in-force revisions")
    ap.add_argument("--refresh", action="store_true", help="with --history: ignore the cached revision lists")
    ap.add_argument("--verify", action="store_true",
                    help="after --write, run `lawgit check` on the repository (cross-validation with the Rust validator)")
    ap.add_argument("--lawgit-bin", default="lawgit", help="lawgit executable used by --verify (default: lawgit)")
    ap.add_argument("--write", action="store_true", help="actually write files (default: dry-run)")
    ap.add_argument("--update", action="store_true", help="overwrite laws that are already imported")
    args = ap.parse_args(argv)

    repo = Path(args.repo)
    if not (repo / "laws").is_dir():
        print(f"error: {repo}/laws not found; run `lawgit --repo {repo} init` first", file=sys.stderr)
        return 2
    has_promulgation_filter = bool(args.promulgated_from or args.promulgated_to)
    list_mode = args.all or has_promulgation_filter
    modes = [bool(args.from_file), bool(args.law_ids), list_mode]
    if sum(modes) != 1:
        print("error: give exactly one of: law IDs, list filters (--all / --promulgated-from / --promulgated-to), --from-file",
              file=sys.stderr)
        return 2
    try:
        validate_promulgation_range(args.promulgated_from, args.promulgated_to)
    except ImportError_ as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if args.from_file and not (args.law_id and LAW_ID_RE.match(args.law_id)):
        print("error: --from-file requires a valid --law-id", file=sys.stderr)
        return 2
    if args.history and args.from_file:
        print("error: --history cannot be used with --from-file", file=sys.stderr)
        return 2
    if (args.include_unenforced or args.refresh) and not args.history:
        print("error: --include-unenforced / --refresh require --history", file=sys.stderr)
        return 2
    if args.max is not None and (not list_mode or args.max < 1):
        print("error: --max must be >= 1 and is only valid with --all or promulgation-date filters",
              file=sys.stderr)
        return 2
    if args.delay < MIN_DELAY:
        print(f"error: --delay must be at least {MIN_DELAY} seconds", file=sys.stderr)
        return 2
    if args.all and args.write and not args.yes:
        print("error: --all --write downloads every law; add --yes to confirm "
              "(try --all --max 5 --write --yes first)", file=sys.stderr)
        return 2

    known = existing_by_egov_id(repo / "laws")
    jobs: list[tuple[str, bytes | None]] = []
    if args.from_file:
        try:
            raw = Path(args.from_file).read_bytes()
        except OSError as e:
            print(f"error: cannot read {args.from_file}: {e}", file=sys.stderr)
            return 2
        if len(raw) > MAX_BYTES:
            print("error: file too large", file=sys.stderr)
            return 2
        jobs.append((args.law_id, raw))
    elif list_mode:
        try:
            all_ids = list_all_law_ids(
                args.asof, args.max, args.delay,
                promulgated_from=args.promulgated_from,
                promulgated_to=args.promulgated_to,
            )
        except ImportError_ as e:
            print(f"error: cannot list laws: {e}", file=sys.stderr)
            return 2
        if args.history:
            try:
                return run_history(args, repo, all_ids, known)
            except ImportError_ as e:
                print(f"error: {e}", file=sys.stderr)
                return 2
        todo = [i for i in all_ids if args.update or i not in known]
        print(f"e-Gov lists {len(all_ids)} law(s); {len(all_ids) - len(todo)} already imported; "
              f"{len(todo)} to process (~{int(len(todo) * (args.delay + 1) / 60)} min at --delay {args.delay}).")
        if not args.write:
            print("first IDs: " + ", ".join(todo[:10]))
            print("\n(dry-run: nothing was downloaded or written; add --write --yes to import)")
            return 0
        jobs = [(i, None) for i in todo]
    else:
        if args.history:
            try:
                return run_history(args, repo, args.law_ids, known)
            except ImportError_ as e:
                print(f"error: {e}", file=sys.stderr)
                return 2
        jobs = [(i, None) for i in args.law_ids]

    done = failures = streak = 0
    for n, (egov_id, raw) in enumerate(jobs):
        try:
            if raw is None:
                if n > 0:
                    time.sleep(args.delay)  # be polite to the public API
                raw = fetch_law_xml(egov_id, args.asof)
            print(f"[{n + 1}/{len(jobs)}] " + import_one(repo, egov_id, raw, args.write, args.update, known), flush=True)
            done += 1
            streak = 0
        except ImportError_ as e:
            failures += 1
            streak += 1
            print(f"[{n + 1}/{len(jobs)}] ERROR   {egov_id}: {e}", file=sys.stderr, flush=True)
            if streak >= MAX_CONSECUTIVE_FAILURES:
                print(f"aborting: {streak} consecutive failures (re-run later; finished laws are skipped)", file=sys.stderr)
                break
        except KeyboardInterrupt:
            print("\ninterrupted (re-run to resume; finished laws are skipped)", file=sys.stderr)
            failures += 1
            break
    print(f"\nsummary: {done} processed, {failures} failed")
    if args.write:
        print("Next: lawgit validate; review with `git diff`; then git add / git commit.")
    else:
        print("(dry-run: nothing was written; add --write to apply)")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    code = _main_impl(argv)
    if "--verify" in argv and "--write" in argv and code in (0, 1):
        try:
            repo_arg = argv[argv.index("--repo") + 1] if "--repo" in argv else "."
            bin_arg = argv[argv.index("--lawgit-bin") + 1] if "--lawgit-bin" in argv else "lawgit"
        except IndexError:
            return code
        if not verify_with_lawgit(Path(repo_arg), bin_arg):
            return 1
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
