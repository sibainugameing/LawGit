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

本文 (<body>) は e-Gov 法令 XML の LawBody 配下をそのまま構造要素として保持します
(要素語彙は e-Gov のもの。lawgit 側の語彙定義は別途仕様化の課題)。
"""
from __future__ import annotations

import argparse
import os
import re
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
LAW_ID_RE = re.compile(r"^[0-9A-Za-z]{1,32}$")
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


def fetch_law_xml(law_id: str, asof: str | None) -> bytes:
    if not LAW_ID_RE.match(law_id):
        raise ImportError_(f"invalid law id: {law_id!r}")
    url = f"{API_BASE}/law_file/xml/{urllib.parse.quote(law_id)}"
    if asof:
        if not ASOF_RE.match(asof):
            raise ImportError_("--asof must be YYYY-MM-DD")
        url += "?" + urllib.parse.urlencode({"asof": asof})
    opener = urllib.request.build_opener(_SameHostRedirect())
    req = urllib.request.Request(url, headers={"User-Agent": "lawgit-egov-import/0.1", "Accept": "application/xml"})
    try:
        with opener.open(req, timeout=TIMEOUT) as resp:
            data = resp.read(MAX_BYTES + 1)
    except urllib.error.HTTPError as e:
        raise ImportError_(f"e-Gov returned HTTP {e.code} for {law_id}")
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        raise ImportError_(f"network error for {law_id}: {e}")
    if len(data) > MAX_BYTES:
        raise ImportError_(f"response for {law_id} exceeds {MAX_BYTES} bytes")
    return data


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


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Import e-Gov laws into a lawgit repository (separate from the Rust CLI).")
    ap.add_argument("law_ids", nargs="*", help="e-Gov law IDs, e.g. 415AC0000000057")
    ap.add_argument("--repo", default=".", help="lawgit repository (must contain laws/)")
    ap.add_argument("--from-file", help="convert a local e-Gov XML file instead of downloading (offline)")
    ap.add_argument("--law-id", help="e-Gov law ID to record when using --from-file")
    ap.add_argument("--asof", help="point-in-time snapshot YYYY-MM-DD (enforcement date)")
    ap.add_argument("--write", action="store_true", help="actually write files (default: dry-run)")
    ap.add_argument("--update", action="store_true", help="overwrite laws that are already imported")
    args = ap.parse_args(argv)

    repo = Path(args.repo)
    if not (repo / "laws").is_dir():
        print(f"error: {repo}/laws not found; run `lawgit --repo {repo} init` first", file=sys.stderr)
        return 2
    if bool(args.from_file) == bool(args.law_ids):
        print("error: give law IDs, or --from-file (not both)", file=sys.stderr)
        return 2
    if args.from_file and not (args.law_id and LAW_ID_RE.match(args.law_id)):
        print("error: --from-file requires a valid --law-id", file=sys.stderr)
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
    else:
        jobs = [(i, None) for i in args.law_ids]

    failures = 0
    for n, (egov_id, raw) in enumerate(jobs):
        try:
            if raw is None:
                if n > 0:
                    time.sleep(1.0)  # be polite to the public API
                raw = fetch_law_xml(egov_id, args.asof)
            print(import_one(repo, egov_id, raw, args.write, args.update, known))
        except ImportError_ as e:
            failures += 1
            print(f"ERROR   {egov_id}: {e}", file=sys.stderr)
    if args.write:
        print("\nNext: lawgit validate; review with `git diff`; then git add / git commit.")
    else:
        print("\n(dry-run: nothing was written; add --write to apply)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
