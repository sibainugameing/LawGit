#!/usr/bin/env python3
"""Offline tests for egov_import.py (no network). Run: python3 tools/test_egov_import.py"""
import sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
import egov_import as E

SAMPLE = """<?xml version="1.0" encoding="UTF-8"?>
<Law Era="Showa" Year="35" Num="105" LawType="Act" Lang="ja">
<LawNum>昭和三十五年法律第百五号</LawNum>
<LawBody>
<LawTitle Kana="どうろこうつうほう">道路交通法</LawTitle>
<MainProvision>
<Chapter Num="1"><ChapterTitle>第一章　総則</ChapterTitle>
<Article Num="1"><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第一条</ArticleTitle>
<Paragraph Num="1"><ParagraphNum/><ParagraphSentence><Sentence>この法律は、&lt;道路&gt; &amp; 交通の安全を図る。</Sentence></ParagraphSentence></Paragraph>
</Article></Chapter>
</MainProvision>
</LawBody></Law>"""

class T(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp()); (self.d / "laws").mkdir()

    def test_create_then_skip_then_update(self):
        known = E.existing_by_egov_id(self.d / "laws")
        r = E.import_one(self.d, "335AC0000000105", SAMPLE.encode(), True, False, known)
        self.assertTrue(r.startswith("CREATE"), r)
        files = list((self.d / "laws").glob("*.xml")); self.assertEqual(len(files), 1)
        text = files[0].read_text(encoding="utf-8")
        self.assertNotIn("\r", text)
        self.assertIn("<type>法律</type>", text)
        self.assertIn("<law-number>昭和三十五年法律第百五号</law-number>", text)
        self.assertIn("&lt;道路&gt; &amp; 交通", text)
        E.self_check(text, files[0].stem, "335AC0000000105")
        known = E.existing_by_egov_id(self.d / "laws")
        self.assertTrue(E.import_one(self.d, "335AC0000000105", SAMPLE.encode(), True, False, known).startswith("SKIP"))
        r = E.import_one(self.d, "335AC0000000105", SAMPLE.encode(), True, True, known)
        self.assertTrue(r.startswith("UPDATE"))
        self.assertEqual([p.name for p in (self.d / "laws").glob("*.xml")], [files[0].name])  # UUID kept

    def test_dry_run_writes_nothing(self):
        r = E.import_one(self.d, "X1", SAMPLE.encode(), False, False, {})
        self.assertTrue(r.startswith("DRY-RUN")); self.assertEqual(list((self.d / "laws").iterdir()), [])

    def test_rejects_doctype_and_bad_ids(self):
        evil = b'<?xml version="1.0"?><!DOCTYPE a [<!ENTITY x "y">]><Law/>'
        with self.assertRaises(E.ImportError_): E.parse_untrusted_xml(evil)
        with self.assertRaises(E.ImportError_): E.fetch_law_xml("../etc/passwd", None)
        with self.assertRaises(E.ImportError_): E.fetch_law_xml("415AC0000000057", "2025/01/01")

    def test_missing_title(self):
        with self.assertRaises(E.ImportError_):
            E.import_one(self.d, "X1", b"<Law><LawBody><MainProvision/></LawBody></Law>", False, False, {})

    def test_wrapped_in_dataroot(self):
        wrapped = ("<DataRoot><ApplData><LawFullText>" + SAMPLE.split("?>", 1)[1] + "</LawFullText></ApplData></DataRoot>").encode()
        self.assertTrue(E.import_one(self.d, "X1", wrapped, False, False, {}).startswith("DRY-RUN"))

class AllTests(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp()); (self.d / "laws").mkdir()
        self.orig_get, self.orig_sleep = E._get, E.time.sleep
        E.time.sleep = lambda s: None
        self.calls = []

        def fake(url, accept):
            self.calls.append(url)
            if "/laws?" in url:
                off = int(url.split("offset=")[1].split("&")[0])
                pages = {0: ([f"A{i}" for i in range(3)] + ["../bad"], 4), 4: (["A3", "A0", "A4"], None)}
                ids, nxt = pages[off]
                body = {"laws": [{"law_info": {"law_id": i}} for i in ids] + [{"x": 1}, "junk"]}
                if nxt is not None: body["next_offset"] = nxt
                return E.json.dumps(body).encode()
            return SAMPLE.encode()
        E._get = fake

    def tearDown(self):
        E._get, E.time.sleep = self.orig_get, self.orig_sleep

    def test_list_paginates_validates_dedupes(self):
        self.assertEqual(E.list_all_law_ids(), ["A0", "A1", "A2", "A3", "A4"])
        self.assertEqual(E.list_all_law_ids(max_items=2), ["A0", "A1"])

    def test_promulgation_date_filters_are_sent_to_api(self):
        self.assertEqual(
            E.list_all_law_ids(promulgated_from="2020-01-01", promulgated_to="2020-12-31"),
            ["A0", "A1", "A2", "A3", "A4"],
        )
        query = E.urllib.parse.parse_qs(E.urllib.parse.urlparse(self.calls[-2]).query)
        self.assertEqual(query["promulgation_date_from"], ["2020-01-01"])
        self.assertEqual(query["promulgation_date_to"], ["2020-12-31"])

    def test_promulgation_date_filter_validation(self):
        with self.assertRaises(E.ImportError_):
            E.list_all_law_ids(promulgated_from="2020-02-30")
        with self.assertRaises(E.ImportError_):
            E.list_all_law_ids(promulgated_from="2021-01-01", promulgated_to="2020-12-31")

    def test_all_dry_run_downloads_nothing(self):
        rc = E.main(["--repo", str(self.d), "--all"])
        self.assertEqual(rc, 0)
        self.assertFalse(any("law_file" in c for c in self.calls))
        self.assertEqual(list((self.d / "laws").iterdir()), [])

    def test_all_write_needs_yes(self):
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "--write"]), 2)
        self.assertEqual(list((self.d / "laws").iterdir()), [])

    def test_all_write_then_resume_skips(self):
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "--write", "--yes", "--max", "3"]), 0)
        self.assertEqual(len(list((self.d / "laws").glob("*.xml"))), 3)
        before = len([c for c in self.calls if "law_file" in c])
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "--write", "--yes"]), 0)
        self.assertEqual(len(list((self.d / "laws").glob("*.xml"))), 5)
        self.assertEqual(len([c for c in self.calls if "law_file" in c]) - before, 2)  # only A3, A4

    def test_promulgation_filter_can_be_used_without_all(self):
        self.assertEqual(E.main(["--repo", str(self.d), "--promulgated-from", "2020-01-01"]), 0)
        list_calls = [c for c in self.calls if "/laws?" in c]
        self.assertTrue(list_calls)
        query = E.urllib.parse.parse_qs(E.urllib.parse.urlparse(list_calls[0]).query)
        self.assertEqual(query["promulgation_date_from"], ["2020-01-01"])

    def test_promulgation_filter_can_dispatch_history_mode(self):
        original_list, original_history = E.list_all_law_ids, E.run_history
        captured = {}
        E.list_all_law_ids = lambda *a, **kw: ["A1", "A2"]
        E.run_history = lambda args, repo, law_ids, known: captured.update(
            history=args.history, ids=law_ids
        ) or 0
        try:
            rc = E.main(["--repo", str(self.d), "--promulgated-from", "2020-01-01", "--history"])
        finally:
            E.list_all_law_ids, E.run_history = original_list, original_history
        self.assertEqual(rc, 0)
        self.assertTrue(captured["history"])
        self.assertEqual(captured["ids"], ["A1", "A2"])
        self.assertFalse(any("law_file" in c for c in self.calls))
        self.assertEqual(list((self.d / "laws").glob("*.xml")), [])

    def test_option_conflicts(self):
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "A1"]), 2)
        self.assertEqual(E.main(["--repo", str(self.d), "--max", "3", "A1"]), 2)
        self.assertEqual(E.main(["--repo", str(self.d), "--promulgated-from", "2020-01-01", "A1"]), 2)
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "--delay", "0.1"]), 2)

    def test_abort_after_consecutive_failures(self):
        def failing(url, accept):
            if "/laws?" in url:
                return E.json.dumps({"laws": [{"law_info": {"law_id": f"B{i}"}} for i in range(30)]}).encode()
            raise E.ImportError_("boom")
        E._get = failing
        self.assertEqual(E.main(["--repo", str(self.d), "--all", "--write", "--yes"]), 1)

import subprocess, json

def sh(d, *a):
    return subprocess.run(["git", "-C", str(d), *a], capture_output=True, text=True, check=True).stdout

class HistoryTests(unittest.TestCase):
    # L1: 1947 (new) -> 2020-04-01 (amended by AM1) -> 2022-01-01 (amended by AM2) -> 2099 (not yet in force)
    # L2: 2020-04-01 (amended by AM1, same act as L1)  [law "new" in 2018 is also listed]
    REVS = {
        "L1": [("L1_19470503_000000000000000", "1947-05-03", "1947-05-03", "", "", "", "CurrentEnforced"),
               ("L1_20200401_502AC0000000001", "2020-04-01", "2019-12-01", "AM1", "第一改正法", "令和元年法律第一号", "CurrentEnforced"),
               ("L1_20220101_504AC0000000002", "2022-01-01", "2021-06-01", "AM2", "第二改正法", "令和三年法律第二号", "CurrentEnforced"),
               ("L1_20990101_999AC0000000003", "2099-01-01", "2098-01-01", "AM3", "未来改正法", "令和八十年法律第三号", "UnEnforced")],
        "L2": [("L2_20180401_000000000000000", "2018-04-01", "", "", "", "", "CurrentEnforced"),
               ("L2_20200401_502AC0000000001", "2020-04-01", "2019-12-01", "AM1", "第一改正法", "令和元年法律第一号", "CurrentEnforced")],
    }

    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        (self.d / "laws").mkdir(); (self.d / "laws" / ".gitkeep").write_text("")
        subprocess.run(["git", "init", "-q", str(self.d)], check=True)
        self.orig_get, self.orig_sleep = E._get, E.time.sleep
        E.time.sleep = lambda s: None
        self.calls = []

        def fake(url, accept):
            self.calls.append(url)
            if "/law_revisions/" in url:
                lid = url.rsplit("/", 1)[1]
                revs = [{"revision_info": {"law_revision_id": r[0], "amendment_enforcement_date": r[1],
                         "amendment_promulgate_date": r[2], "amendment_law_id": r[3], "amendment_law_title": r[4],
                         "amendment_law_num": r[5], "current_revision_status": r[6], "law_title": "道路交通法"}}
                        for r in self.REVS[lid]]
                return json.dumps({"law_info": {"law_id": lid}, "revisions": revs}).encode()
            rid = url.rsplit("/", 1)[1]
            return SAMPLE.replace("図る", "図る[" + rid + "]").encode()
        E._get = fake

    def tearDown(self):
        E._get, E.time.sleep = self.orig_get, self.orig_sleep

    def run_h(self, *extra):
        return E.main(["--repo", str(self.d), "--history", "L1", "L2", *extra])

    def test_dry_run_changes_nothing(self):
        self.assertEqual(self.run_h(), 0)
        self.assertFalse(any("law_file" in c for c in self.calls))
        self.assertEqual(list((self.d / "laws").glob("*.xml")), [])

    def test_history_commits(self):
        self.assertEqual(self.run_h("--write"), 0)
        log = sh(self.d, "log", "--reverse", "--format=%at|%aI|%an|%s").strip().splitlines()
        # 1947 clamped, L2 2018, shared 2020 commit (2 laws), 2022; future one skipped
        self.assertEqual(len(log), 4, log)
        self.assertEqual(log[0].split("|")[0], "86400")
        self.assertIn("制定・初出版", log[0])
        self.assertTrue(log[1].split("|")[1].startswith("2018-04-01T00:00:00+09:00"))
        self.assertIn("第一改正法による改正 (2法令)", log[2])
        self.assertIn("2022-01-01", log[3].split("|")[1])
        self.assertEqual(log[3].split("|")[2], "e-Gov Import")
        shared = sh(self.d, "show", "--stat", "--format=%B", "HEAD~1")
        self.assertEqual(shared.count("laws/"), 2)
        self.assertIn("Enforced: 2020-04-01", shared)
        self.assertIn("E-Gov-Revision: L1_20200401_502AC0000000001", shared)
        self.assertIn("Amending-Law: 令和元年法律第一号 AM1", shared)
        # final state = latest enforced revision, constant UUID per law
        files = sorted((self.d / "laws").glob("*.xml"))
        self.assertEqual(len(files), 2)
        l1 = [f for f in files if "<e-gov-id>L1<" in f.read_text(encoding="utf-8")][0]
        self.assertIn("L1_20220101_504AC0000000002", l1.read_text(encoding="utf-8"))
        self.assertEqual(len(sh(self.d, "log", "--format=%H", "--", f"laws/{l1.name}").split()), 3)
        self.assertEqual(sh(self.d, "status", "--porcelain", "--", "laws/" + l1.name).strip(), "")

    def test_rerun_is_idempotent_and_resumable(self):
        self.assertEqual(self.run_h("--write"), 0)
        n = len(sh(self.d, "log", "--format=%H").split())
        before = len([c for c in self.calls if "law_file" in c])
        self.assertEqual(self.run_h("--write"), 0)
        self.assertEqual(len(sh(self.d, "log", "--format=%H").split()), n)
        self.assertEqual(len([c for c in self.calls if "law_file" in c]), before)
        self.assertEqual(len([c for c in self.calls if "law_revisions" in c]), 2)  # lists came from cache

    def test_include_unenforced(self):
        self.assertEqual(self.run_h("--write", "--include-unenforced"), 0)
        self.assertEqual(len(sh(self.d, "log", "--format=%H").split()), 5)

    def test_refuses_dirty_laws_dir(self):
        (self.d / "laws" / "stray.xml").write_text("<x/>")
        self.assertEqual(self.run_h("--write"), 2)
        self.assertEqual(subprocess.run(["git", "-C", str(self.d), "log"], capture_output=True).returncode, 128)

    def test_refuses_staged_changes(self):
        (self.d / "notes.txt").write_text("x"); sh(self.d, "add", "notes.txt")
        self.assertEqual(self.run_h("--write"), 2)

    def test_options_validation(self):
        self.assertEqual(E.main(["--repo", str(self.d), "--refresh", "L1"]), 2)
        self.assertEqual(E.main(["--repo", str(self.d), "--history", "--from-file", "x", "--law-id", "L1"]), 2)

    def test_all_history_dry_run_makes_no_requests(self):
        orig = E.list_all_law_ids
        E.list_all_law_ids = lambda *a, **k: ["L1", "L2"]
        try:
            self.assertEqual(E.main(["--repo", str(self.d), "--all", "--history"]), 0)
        finally:
            E.list_all_law_ids = orig
        self.assertEqual(self.calls, [])

    def test_hostile_metadata_is_sanitized(self):
        rec = E._clean_record("L1", {"law_revision_id": "L1_20200401_X", "amendment_law_title": "a\x00b\nc\x1b",
                                     "amendment_law_id": "../x", "amendment_enforcement_date": "2020-13-45"})
        self.assertEqual(rec["amend_title"], "a b c")
        self.assertEqual(rec["amend_id"], "")
        self.assertEqual(rec["enf"], "2020-04-01")  # falls back to the revision id's date
        self.assertIsNone(E._clean_record("L1", {"law_revision_id": "OTHER_20200401_X"}))
        self.assertIsNone(E._clean_record("L1", {"law_revision_id": "L1_20200401_../x"}))

import os, stat

class VerifyTests(unittest.TestCase):
    def fake_bin(self, code):
        p = Path(tempfile.mkdtemp()) / "fake-lawgit"
        p.write_text(f"#!/bin/sh\necho checked \"$@\"\nexit {code}\n")
        p.chmod(p.stat().st_mode | stat.S_IEXEC)
        return str(p)

    def run_verify(self, code):
        d = Path(tempfile.mkdtemp()); (d / "laws").mkdir()
        f = d / "src.xml"; f.write_text(SAMPLE, encoding="utf-8")
        return E.main(["--repo", str(d), "--from-file", str(f), "--law-id", "X1", "--write",
                       "--verify", "--lawgit-bin", self.fake_bin(code)])

    def test_verify_pass(self):
        self.assertEqual(self.run_verify(0), 0)

    def test_verify_fail_sets_exit_code(self):
        self.assertEqual(self.run_verify(1), 1)

    def test_verify_missing_binary(self):
        d = Path(tempfile.mkdtemp()); (d / "laws").mkdir()
        f = d / "src.xml"; f.write_text(SAMPLE, encoding="utf-8")
        self.assertEqual(E.main(["--repo", str(d), "--from-file", str(f), "--law-id", "X1", "--write",
                                 "--verify", "--lawgit-bin", "/nonexistent/lawgit"]), 1)

if __name__ == "__main__":
    unittest.main()
