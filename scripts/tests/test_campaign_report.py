import re
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import campaign_report  # noqa: E402

FIXTURES = Path(__file__).resolve().parent / "fixtures"
REPORT = FIXTURES / "reviews" / "demo" / "audit-demo-2026-07-20.md"
TRANSCRIPTS = FIXTURES / "transcripts"

FIELD_RE = re.compile(r"^\| (\S+) \| (.*) \|$")


def _read_fields(text):
    fields = {}
    for line in text.splitlines():
        m = FIELD_RE.match(line)
        if m and m.group(1) != "field":
            fields[m.group(1)] = m.group(2)
    return fields


class TestCampaignReport(unittest.TestCase):
    def test_cost_attribution(self):
        with tempfile.TemporaryDirectory() as out_dir:
            rc = campaign_report.main([
                str(REPORT),
                "--transcripts", str(TRANSCRIPTS),
                "--out", out_dir,
            ])
            self.assertEqual(rc, 0)

            out_path = Path(out_dir) / "demo-2026-07-20.md"
            self.assertTrue(out_path.is_file())
            text = out_path.read_text(encoding="utf-8")
            self.assertIn("## Cost", text)

            fields = _read_fields(text)
            self.assertEqual(fields["spawns"], "3")
            self.assertEqual(fields["spawns_finding_worker"], "1")
            self.assertEqual(fields["spawns_rework_planner"], "2")
            self.assertEqual(fields["unattributed_spawns_in_window"], "1")
            self.assertEqual(fields["output_tokens"], "450")
            self.assertEqual(fields["output_max"], "300")
            self.assertEqual(fields["output_median_nonzero"], "150")
            self.assertEqual(fields["cache_create_tokens"], "1000")
            self.assertEqual(fields["cache_read_tokens"], "10000")
            self.assertEqual(fields["dead_spawns"], "1")

    def test_missing_transcripts_dir_fails_without_writing(self):
        with tempfile.TemporaryDirectory() as out_dir:
            missing = Path(out_dir) / "does-not-exist"
            rc = campaign_report.main([
                str(REPORT),
                "--transcripts", str(missing),
                "--out", out_dir,
            ])
            self.assertNotEqual(rc, 0)
            self.assertEqual(list(Path(out_dir).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
