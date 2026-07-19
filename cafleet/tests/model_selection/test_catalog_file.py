"""The committed seed catalog at ``skills/cafleet/reference/model-catalog.md``
and its Step-1 companions: the pruned Director reference and the no-packaged-copy rule."""

from pathlib import Path

import pytest

from cafleet.model_selection import parse_catalog_markdown

from ._helpers import (
    APPROVED_ANTHROPIC_URL,
    APPROVED_OPENAI_URL,
    WORKFLOW_ROLE_PROFILES,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
CATALOG_PATH = REPO_ROOT / "skills" / "cafleet" / "reference" / "model-catalog.md"
DIRECTOR_REFERENCE_PATH = REPO_ROOT / "skills" / "cafleet" / "reference" / "director.md"
PACKAGE_SRC = REPO_ROOT / "cafleet" / "src" / "cafleet"

# Initial inclusion authority: the direct Claude / Codex model tokens documented
# in skills/cafleet/reference/director.md at seeding time.
SEED_CLAUDE_TOKENS = {"fable", "opus", "sonnet", "haiku"}
SEED_CODEX_TOKENS = {
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
}


@pytest.fixture(scope="module")
def committed_catalog():
    return parse_catalog_markdown(CATALOG_PATH.read_text(encoding="utf-8"))


def test_committed_catalog_parses_as_schema_v1(committed_catalog):
    assert committed_catalog.schema_version == 1


def test_committed_catalog_sources_are_exactly_the_approved_urls(committed_catalog):
    sources = committed_catalog.sources
    assert set(sources) == {"anthropic", "openai"}
    assert sources["anthropic"].url == APPROVED_ANTHROPIC_URL
    assert sources["openai"].url == APPROVED_OPENAI_URL


def test_committed_catalog_covers_every_workflow_role(committed_catalog):
    role_profiles = committed_catalog.role_profiles
    assert set(role_profiles) == set(WORKFLOW_ROLE_PROFILES)
    for role, expected in WORKFLOW_ROLE_PROFILES.items():
        profile = role_profiles[role]
        assert profile.task_kind == expected["task_kind"]
        assert dict(profile.requires) == expected["requires"]
        assert profile.token_profile == expected["token_profile"]


def test_committed_catalog_has_active_direct_claude_and_codex_records(
    committed_catalog,
):
    for backend in ("claude", "codex"):
        models = [
            m for m in committed_catalog.models if m.backend == backend and m.active
        ]
        assert models, f"no active {backend} model in the seeded catalog"
        assert any(
            card.status == "known" for model in models for card in model.rate_cards
        )


def test_committed_catalog_maps_the_documented_direct_tokens(committed_catalog):
    claude_tokens = {
        e.token for e in committed_catalog.model_tokens if e.backend == "claude"
    }
    codex_tokens = {
        e.token for e in committed_catalog.model_tokens if e.backend == "codex"
    }
    assert claude_tokens >= SEED_CLAUDE_TOKENS
    assert codex_tokens >= SEED_CODEX_TOKENS


def test_committed_catalog_capability_provenance_is_maintainer_judgment(
    committed_catalog,
):
    for model in committed_catalog.models:
        assert model.capability.provenance.type == "maintainer_judgment"


def test_committed_catalog_gateway_models_have_no_known_rate_card(committed_catalog):
    for model in committed_catalog.models:
        if model.backend == "opencode":
            assert all(card.status != "known" for card in model.rate_cards)


def test_no_catalog_copy_ships_inside_the_python_package():
    assert [
        path for path in PACKAGE_SRC.rglob("*") if "model-catalog" in path.name
    ] == []


def test_director_reference_links_the_catalog_and_drops_the_model_tables():
    text = DIRECTOR_REFERENCE_PATH.read_text(encoding="utf-8")
    assert "model-catalog.md" in text
    assert "Available models per backend" not in text
