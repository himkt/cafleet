"""Envelope contract of ``parse_catalog_markdown``: one sentinel marker line
immediately followed by exactly one canonical-JSON fenced payload block."""

import json

import pytest

from cafleet.model_selection import CatalogInvalidError, parse_catalog_markdown

from ._helpers import (
    CATALOG_MARKER,
    WORKFLOW_ROLE_PROFILES,
    base_catalog,
    canonical_payload,
    catalog_markdown,
    render_markdown,
)


def test_valid_catalog_parses_as_schema_v1():
    catalog = parse_catalog_markdown(catalog_markdown(base_catalog()))
    assert catalog.schema_version == 1


def test_parsed_catalog_exposes_typed_sections():
    catalog = parse_catalog_markdown(catalog_markdown(base_catalog()))
    assert set(catalog.token_profiles) == {"small", "standard", "large"}
    assert set(catalog.role_profiles) == set(WORKFLOW_ROLE_PROFILES)
    assert {model.key for model in catalog.models} == {
        "claude:claude-sonnet-5",
        "codex:gpt-5.6-luna",
    }
    assert {entry.token for entry in catalog.model_tokens} == {"sonnet", "gpt-5.6-luna"}
    assert set(catalog.sources) == {"anthropic", "openai"}


def test_missing_marker_rejected():
    text = render_markdown(canonical_payload(base_catalog()), marker="")
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_wrong_marker_version_rejected():
    text = render_markdown(
        canonical_payload(base_catalog()),
        marker="<!-- cafleet-model-catalog: v2 -->",
    )
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_duplicate_marker_rejected():
    text = f"{CATALOG_MARKER}\n\n" + render_markdown(canonical_payload(base_catalog()))
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_marker_not_immediately_followed_by_payload_rejected():
    payload = canonical_payload(base_catalog())
    text = (
        "# CAFleet Model Catalog\n"
        "\n"
        f"{CATALOG_MARKER}\n"
        "An intervening paragraph.\n"
        "```json\n"
        f"{payload}"
        "```\n"
    )
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_non_json_payload_fence_rejected():
    payload = canonical_payload(base_catalog())
    text = f"# CAFleet Model Catalog\n\n{CATALOG_MARKER}\n```text\n{payload}```\n"
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_duplicate_payload_block_rejected():
    payload = canonical_payload(base_catalog())
    text = render_markdown(payload) + "\n```json\n" + payload + "```\n"
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_payload_not_json_rejected():
    text = render_markdown("not json at all\n")
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_payload_with_comments_rejected():
    text = render_markdown('{\n  "schema_version": 1 // inline comment\n}\n')
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(text)


def test_unsorted_keys_rejected():
    payload = json.dumps(base_catalog(), indent=2, ensure_ascii=False) + "\n"
    assert payload != canonical_payload(base_catalog())
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(render_markdown(payload))


def test_four_space_indent_rejected():
    payload = (
        json.dumps(base_catalog(), sort_keys=True, indent=4, ensure_ascii=False) + "\n"
    )
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(render_markdown(payload))


def test_compact_serialization_rejected():
    payload = (
        json.dumps(
            base_catalog(), sort_keys=True, separators=(",", ":"), ensure_ascii=False
        )
        + "\n"
    )
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(render_markdown(payload))


def test_trailing_blank_line_in_payload_rejected():
    payload = canonical_payload(base_catalog()) + "\n"
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(render_markdown(payload))


def test_unknown_schema_version_rejected():
    catalog = base_catalog()
    catalog["schema_version"] = 2
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(catalog_markdown(catalog))
