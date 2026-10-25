import copy
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
EXTRACTOR = SCRIPTS / "extract_hermes_portal.py"
FIXTURE = Path(__file__).parent / "fixtures" / "portal-models.html"
SOURCE_URL = "https://portal.nousresearch.com/models"
PRICE_FIELDS = (
    "promptDollarsPerMillion",
    "completionDollarsPerMillion",
    "originalPromptDollarsPerMillion",
    "originalCompletionDollarsPerMillion",
)
MODEL_FIELDS = {
    "source_index", "id", "displayName", "providerKey", "providerLabel", "modality",
    "isFree", "hasDiscount", "promptDiscountPercent", "minDiscountFraction",
    "isEmbeddingsModel", "usageRank", "isNousHosted", *PRICE_FIELDS,
}
PUSH = re.compile(r'self\.__next_f\.push\((\[1,"(?:\\.|[^"\\])*"\])\)')
READ_ONLY_DRIVER = r"""
import os
from pathlib import Path
import runpy
import sys

script, source = sys.argv[1:]
violations = []
mutations = {
    "os.system", "os.fork", "os.forkpty", "pty.spawn", "os.remove", "os.rename",
    "os.mkdir", "os.rmdir", "os.symlink", "os.link", "os.chmod", "os.chown",
    "os.utime", "os.truncate",
}

def enforce_read_only(event, arguments):
    forbidden = event in mutations or event.startswith(
        ("subprocess.", "socket.", "os.exec", "os.spawn", "os.posix_spawn")
    )
    if event == "open":
        flags = arguments[2]
        forbidden = bool(flags & (
            os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND
        ))
    if forbidden:
        violations.append(event)
        raise PermissionError("READ_ONLY_VIOLATION: " + event)

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(script).parent))
sys.argv = [script, source]
sys.addaudithook(enforce_read_only)
try:
    runpy.run_path(script, run_name="__main__")
finally:
    if violations:
        sys.stderr.write("READ_ONLY_VIOLATION: " + repr(violations) + "\n")
        raise SystemExit(91)
"""


def fixture_html():
    return FIXTURE.read_text(encoding="utf-8")


def flight_stream(html):
    return "".join(json.loads(match[1])[1] for match in PUSH.finditer(html))


def replace_flight(html, stream):
    without_flight = re.sub(
        r"<script data-fixture-flight>.*?</script>", "", html, flags=re.DOTALL
    )
    payload = json.dumps([1, stream], ensure_ascii=True, separators=(",", ":"))
    script = f"<script data-fixture-flight>self.__next_f.push({payload})</script>"
    return without_flight.replace("</body>", script + "</body>")


def fixture_models():
    stream = flight_stream(fixture_html())
    start = stream.index('"rows":') + len('"rows":')
    return json.JSONDecoder().raw_decode(stream, start)[0]


def replace_models(models):
    html = fixture_html()
    stream = flight_stream(html)
    start = stream.index('"rows":') + len('"rows":')
    _, end = json.JSONDecoder().raw_decode(stream, start)
    encoded = json.dumps(models, ensure_ascii=True, separators=(",", ":"))
    return replace_flight(html, stream[:start] + encoded + stream[end:])


class PortalExtractionTests(unittest.TestCase):
    def run_extractor(self, html):
        self.assertTrue(EXTRACTOR.is_file(), f"Missing offline extractor: {EXTRACTOR}")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "saved-models.html"
            source.write_text(html, encoding="utf-8")
            for name in ("catalog.md", "role-defaults.json", "freshness.txt"):
                (root / name).write_text("previous reviewed state\n", encoding="utf-8")
            before = {
                path.relative_to(root): path.read_bytes()
                for path in root.rglob("*") if path.is_file()
            }
            result = subprocess.run(
                [sys.executable, "-I", "-B", "-c", READ_ONLY_DRIVER, str(EXTRACTOR), str(source)],
                cwd=root, capture_output=True, text=True, timeout=10, check=False,
            )
            after = {
                path.relative_to(root): path.read_bytes()
                for path in root.rglob("*") if path.is_file()
            }
            self.assertEqual(after, before, "Extraction preserves the saved page and prior state")
            self.assertNotIn("READ_ONLY_VIOLATION", result.stderr)
            return result

    def extract(self, html):
        result = self.run_extractor(html)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        facts = json.loads(result.stdout)
        self.assertNotIn("PRIVATE_ANALYTICS_CANARY", result.stdout)
        return facts

    def assert_rejected(self, html):
        result = self.run_extractor(html)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "", "Validation finishes before any facts are emitted")
        self.assertTrue(result.stderr.strip(), "An invalid snapshot supplies a failure reason")

    def test_complete_flight_rows_reconcile_visible_types_totals_and_display_pages(self):
        facts = self.extract(fixture_html())
        metadata = facts["metadata"]
        self.assertEqual(metadata["source_url"], SOURCE_URL)
        self.assertEqual(metadata["visible_type_counts"], {"text": 4, "embeddings": 1})
        self.assertEqual(metadata["visible_page_range"], [1, 2, 5])
        self.assertEqual(metadata["visible_page_count"], 3)
        self.assertEqual(metadata["unique_id_count"], 5)
        self.assertEqual(metadata["selected_text_count"], 4)
        self.assertEqual(metadata["excluded_embedding_count"], 1)
        self.assertEqual([row["source_index"] for row in facts["models"]], list(range(5)))
        for row in facts["models"]:
            self.assertEqual(set(row), MODEL_FIELDS)
        self.assertEqual(
            [row["id"] for row in facts["models"] if row["modality"] == "text"],
            [
                "example/model-preview",
                "example/model-preview:free",
                "example/model-preview:US",
                "other/input-free",
            ],
        )

    def test_exact_ids_duplicate_names_and_billed_precision_survive_independently(self):
        rows = self.extract(fixture_html())["models"]
        self.assertEqual(rows[0]["displayName"], rows[1]["displayName"])
        self.assertEqual(rows[1]["displayName"], rows[2]["displayName"])
        self.assertEqual(rows[0]["promptDollarsPerMillion"], "0.19999999999999998")
        self.assertEqual(rows[2]["id"], "example/model-preview:US")
        self.assertEqual(rows[0]["originalPromptDollarsPerMillion"], "0.5")
        self.assertEqual(rows[0]["originalCompletionDollarsPerMillion"], "1.5")
        self.assertEqual(rows[0]["promptDiscountPercent"], 60)
        self.assertEqual(rows[0]["minDiscountFraction"], "0.5")
        self.assertTrue(rows[0]["hasDiscount"])
        for fraction in (0.49999999999999994, 0.5000000000000001):
            with self.subTest(fraction=fraction):
                models = fixture_models()
                models[0]["minDiscountFraction"] = fraction
                extracted = self.extract(replace_models(models))["models"][0]
                self.assertEqual(extracted["minDiscountFraction"], str(fraction))
                self.assertEqual(extracted["promptDollarsPerMillion"], "0.19999999999999998")

    def test_individual_zero_prices_and_unpublished_qualifiers_remain_explicit(self):
        facts = self.extract(fixture_html())
        rows = facts["models"]
        self.assertEqual(rows[1]["promptDollarsPerMillion"], "0.00")
        self.assertEqual(rows[1]["completionDollarsPerMillion"], "0.00")
        self.assertTrue(rows[1]["isFree"])
        self.assertEqual(rows[3]["promptDollarsPerMillion"], "0.00")
        self.assertEqual(rows[3]["completionDollarsPerMillion"], "1.25")
        self.assertFalse(rows[3]["isFree"])
        self.assertIsNone(rows[1]["originalPromptDollarsPerMillion"])
        self.assertIsNone(rows[1]["originalCompletionDollarsPerMillion"])
        self.assertEqual(facts["metadata"]["discount_end_dates"], "Not published")
        self.assertEqual(facts["metadata"]["time_or_context_price_tiers"], "Not published")

    def test_rows_property_wins_over_filter_and_larger_unrelated_arrays(self):
        html = fixture_html()
        decoys = [dict(fixture_models()[0], id=f"noise/{index}") for index in range(8)]
        stream = flight_stream(html) + "\n9:" + json.dumps(
            {"analyticsRows": decoys}, separators=(",", ":")
        )
        facts = self.extract(replace_flight(html, stream))
        self.assertEqual(facts["metadata"]["unique_id_count"], 5)
        self.assertFalse(any(row["id"].startswith("noise/") for row in facts["models"]))

    def test_complete_payload_also_reconciles_middle_and_final_visible_pages(self):
        for showing, page, expected_range in (
            ("Showing 3-4 of 5 models", "Page 2 of 3", [3, 4, 5]),
            ("Showing 5-5 of 5 models", "Page 3 of 3", [5, 5, 5]),
        ):
            with self.subTest(page=page):
                html = fixture_html().replace("Showing 1-2 of 5 models", showing)
                html = html.replace("Page 1 of 3", page)
                facts = self.extract(html)
                self.assertEqual(facts["metadata"]["visible_page_range"], expected_range)
                self.assertEqual(facts["metadata"]["unique_id_count"], 5)

    def test_visible_inline_markup_and_hidden_fake_counts_do_not_change_reconciliation(self):
        html = fixture_html().replace(
            "Showing 1-2 of 5 models",
            "Showing <span>1</span>-<span>2</span> of <span>5</span> models",
        )
        html = html.replace("Text (4)", "Text (<span>4</span>)")
        self.assertEqual(self.extract(html)["metadata"]["unique_id_count"], 5)

    def test_truncated_flight_and_first_page_only_payload_fail_without_output_or_writes(self):
        html = fixture_html()
        for candidate in (
            replace_flight(html, flight_stream(html)[:-20]),
            replace_models(fixture_models()[:2]),
        ):
            with self.subTest(candidate=candidate[-80:]):
                self.assert_rejected(candidate)

    def test_invalid_channel_payloads_are_json_failures_instead_of_executable_input(self):
        html = fixture_html()
        for payload in (
            '[1,42]',
            '[1,"unterminated]',
            '[1,(()=>{throw new Error("evaluate-is-forbidden")})()]',
        ):
            with self.subTest(payload=payload):
                invalid = f"<script>self.__next_f.push({payload})</script>"
                self.assert_rejected(html.replace("</body>", invalid + "</body>"))

    def test_missing_or_conflicting_visible_counts_and_page_ranges_fail_atomically(self):
        replacements = (
            ("Text (4)", "Text"),
            ("Embeddings (1)", "Embeddings"),
            ("Showing 1-2 of 5 models", ""),
            ("Page 1 of 3", ""),
            ("Text (4)", "Text (3)"),
            ("Showing 1-2 of 5 models", "Showing 1-2 of 6 models"),
            ("Page 1 of 3", "Page 1 of 2"),
            ("Page 1 of 3", "Page 0 of 3"),
            ("Showing 1-2 of 5 models", "Showing 0-2 of 5 models"),
            ("Showing 1-2 of 5 models", "Showing 1-6 of 5 models"),
        )
        for old, new in replacements:
            with self.subTest(replacement=(old, new)):
                self.assert_rejected(fixture_html().replace(old, new))
        self.assert_rejected(fixture_html().replace(
            "</main>", "<p>Showing 1-2 of 6 models</p></main>"
        ))

    def test_ids_remain_required_unique_and_unambiguous(self):
        for invalid in (None, "", " example/model", "example/model ", "example/\nmodel", False):
            with self.subTest(invalid=invalid):
                rows = fixture_models()
                rows[0]["id"] = invalid
                self.assert_rejected(replace_models(rows))
        rows = fixture_models()
        del rows[0]["id"]
        self.assert_rejected(replace_models(rows))
        for conflict in (False, True):
            with self.subTest(conflict=conflict):
                rows = fixture_models()
                rows[2] = copy.deepcopy(rows[0])
                if conflict:
                    rows[2]["completionDollarsPerMillion"] = 0.625
                self.assert_rejected(replace_models(rows))

    def test_types_and_native_boolean_flags_are_required_and_consistent(self):
        changes = (
            ("modality", None),
            ("modality", "image"),
            ("modality", "Text"),
            ("isEmbeddingsModel", True),
            ("isEmbeddingsModel", "false"),
            ("isFree", 0),
            ("hasDiscount", "true"),
        )
        for field, value in changes:
            with self.subTest(field=field, value=value):
                rows = fixture_models()
                rows[0][field] = value
                self.assert_rejected(replace_models(rows))
        rows = fixture_models()
        del rows[0]["modality"]
        self.assert_rejected(replace_models(rows))

    def test_each_billed_price_is_present_numeric_finite_and_nonnegative(self):
        for field in PRICE_FIELDS[:2]:
            for invalid in (None, -0.5, True, "0.5", float("nan"), float("inf")):
                with self.subTest(field=field, invalid=invalid):
                    rows = fixture_models()
                    rows[0][field] = invalid
                    self.assert_rejected(replace_models(rows))
            rows = fixture_models()
            del rows[0][field]
            self.assert_rejected(replace_models(rows))

    def test_free_and_discount_qualifier_conflicts_fail_atomically(self):
        changes = (
            ("isFree", True),
            ("originalPromptDollarsPerMillion", None),
            ("originalCompletionDollarsPerMillion", None),
            ("originalPromptDollarsPerMillion", -1),
            ("originalCompletionDollarsPerMillion", 0.1),
            ("hasDiscount", False),
            ("promptDiscountPercent", None),
            ("promptDiscountPercent", 1),
            ("promptDiscountPercent", 50),
            ("minDiscountFraction", None),
            ("minDiscountFraction", 0.1),
            ("minDiscountFraction", 0.6),
        )
        for field, value in changes:
            with self.subTest(field=field, value=value):
                rows = fixture_models()
                rows[0][field] = value
                self.assert_rejected(replace_models(rows))

    def test_new_pricing_qualifiers_require_review_instead_of_silent_fact_loss(self):
        rows = fixture_models()
        rows[0]["priceTiers"] = [{"afterTokens": 1000, "input": 2, "output": 4}]
        self.assert_rejected(replace_models(rows))

    def test_multiple_browser_rows_arrays_are_an_ambiguity_failure(self):
        html = fixture_html()
        second = json.dumps({"rows": fixture_models()}, separators=(",", ":"))
        self.assert_rejected(replace_flight(html, flight_stream(html) + "\n8:" + second))


class PortalProcedureTests(unittest.TestCase):
    def test_official_source_and_dedicated_procedure_point_to_the_offline_extractor(self):
        skill = (SCRIPTS.parent / "SKILL.md").read_text(encoding="utf-8")
        approved = skill.split("## Approved sources", 1)[1].split("\n## ", 1)[0]
        rows = [line for line in approved.splitlines() if line.startswith("|")]
        self.assertEqual(sum(SOURCE_URL in line for line in rows), 1)
        procedure = re.search(r"(?ms)^## Hermes[^\n]*\n(.*?)(?=^## |\Z)", skill)
        self.assertIsNotNone(procedure, "Hermes has its own refresh procedure")
        self.assertIn(SOURCE_URL, procedure[1])
        self.assertIn("scripts/extract_hermes_portal.py", procedure[1])


if __name__ == "__main__":
    unittest.main()
