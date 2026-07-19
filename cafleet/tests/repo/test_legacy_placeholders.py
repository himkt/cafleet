"""Repository drift guards: the legacy fixed-model policy tokens are gone from
every tracked CAFleet skill, workflow prompt, doc, and test, and the skills
document selector-driven monitor/reviewer model choice instead."""

import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]

# Concatenated so this guard file never matches its own scan.
LEGACY_PLACEHOLDERS = tuple(
    placeholder.encode("utf-8")
    for placeholder in ("{monitor_" + "model}", "{reviewer_" + "model}")
)

SCAN_PREFIXES = ("skills/", "docs/", "cafleet/", ".claude/", "README.md", "SPEC.md")

# Migration fixtures allowed to name a legacy placeholder, by repo-relative path.
PLACEHOLDER_ALLOWLIST: frozenset[str] = frozenset()


def _tracked_files():
    output = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [line for line in output.splitlines() if line]


def _cafleet_skill_texts():
    return {
        rel: (REPO_ROOT / rel).read_text(encoding="utf-8")
        for rel in _tracked_files()
        if rel.startswith("skills/cafleet/") and rel.endswith(".md")
    }


def test_no_legacy_model_placeholders_outside_allowlist():
    offenders = []
    for rel in _tracked_files():
        if not rel.startswith(SCAN_PREFIXES) or rel in PLACEHOLDER_ALLOWLIST:
            continue
        data = (REPO_ROOT / rel).read_bytes()
        if any(placeholder in data for placeholder in LEGACY_PLACEHOLDERS):
            offenders.append(rel)
    assert offenders == []


def test_monitor_spawn_examples_carry_no_fixed_model_pin():
    offenders = [
        rel
        for rel, text in _cafleet_skill_texts().items()
        if "--role monitor" in text and "--model haiku" in text
    ]
    assert offenders == []


def test_skills_document_selector_driven_monitor_and_reviewer_models():
    corpus = "\n".join(_cafleet_skill_texts().values())
    assert "model select" in corpus
    assert "--role monitor" in corpus
    assert "--role reviewer" in corpus
