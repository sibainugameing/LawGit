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

if __name__ == "__main__":
    unittest.main()
