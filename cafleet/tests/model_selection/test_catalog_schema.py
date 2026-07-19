"""Schema-v1 content validation: token/role profiles, capability policy, SKU/token
maps, rate cards, and the approved-source allowlist."""

import pytest

from cafleet.model_selection import CatalogInvalidError, parse_catalog_markdown

from ._helpers import (
    base_catalog,
    catalog_markdown,
    claude_model,
    opencode_gateway_model,
)


def _reject(catalog: dict):
    with pytest.raises(CatalogInvalidError):
        parse_catalog_markdown(catalog_markdown(catalog))


@pytest.mark.parametrize(
    "field",
    [
        "schema_version",
        "generated_at",
        "freshness_days",
        "currency",
        "token_profiles",
        "role_profiles",
        "sources",
        "models",
        "model_tokens",
    ],
)
def test_missing_top_level_field_rejected(field):
    catalog = base_catalog()
    del catalog[field]
    _reject(catalog)


# --- token profiles ---


def test_token_profiles_missing_named_profile_rejected():
    catalog = base_catalog()
    del catalog["token_profiles"]["large"]
    _reject(catalog)


def test_token_profiles_extra_profile_rejected():
    catalog = base_catalog()
    catalog["token_profiles"]["xlarge"] = {
        "input": 48000,
        "cached_input": 0,
        "cache_write": 0,
        "output": 24000,
    }
    _reject(catalog)


def test_token_profile_missing_component_rejected():
    catalog = base_catalog()
    del catalog["token_profiles"]["small"]["output"]
    _reject(catalog)


def test_token_profile_negative_component_rejected():
    catalog = base_catalog()
    catalog["token_profiles"]["small"]["input"] = -1
    _reject(catalog)


def test_token_profile_non_integer_component_rejected():
    catalog = base_catalog()
    catalog["token_profiles"]["small"]["input"] = 4000.5
    _reject(catalog)


# --- role profiles ---


def test_role_profile_unknown_role_key_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["wizard"] = {
        "task_kind": "monitoring",
        "requires": {"monitor": 2},
        "token_profile": "small",
    }
    _reject(catalog)


def test_role_profile_unknown_task_kind_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["monitor"]["task_kind"] = "sorcery"
    _reject(catalog)


def test_role_profile_empty_requires_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["monitor"]["requires"] = {}
    _reject(catalog)


def test_role_profile_unknown_dimension_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["monitor"]["requires"] = {"vibes": 2}
    _reject(catalog)


def test_role_profile_requirement_above_five_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["monitor"]["requires"] = {"monitor": 6}
    _reject(catalog)


def test_role_profile_unknown_token_profile_rejected():
    catalog = base_catalog()
    catalog["role_profiles"]["monitor"]["token_profile"] = "xlarge"
    _reject(catalog)


# --- models and capability policy ---


def test_duplicate_model_key_rejected():
    catalog = base_catalog()
    duplicate = claude_model()
    duplicate["capability"]["global_rank"] = 21
    catalog["models"].append(duplicate)
    _reject(catalog)


def test_duplicate_provider_sku_rejected():
    catalog = base_catalog()
    duplicate = claude_model()
    duplicate["key"] = "claude:claude-sonnet-5-dup"
    duplicate["capability"]["global_rank"] = 21
    catalog["models"].append(duplicate)
    catalog["model_tokens"].append(
        {
            "backend": "claude",
            "token": "sonnet-dup",
            "model_key": "claude:claude-sonnet-5-dup",
            "primary": True,
        }
    )
    _reject(catalog)


def test_unknown_backend_rejected():
    catalog = base_catalog()
    catalog["models"][0]["backend"] = "gemini"
    catalog["models"][0]["key"] = "gemini:claude-sonnet-5"
    catalog["models"][0]["availability"]["requires_backend"] = "gemini"
    catalog["model_tokens"][0]["backend"] = "gemini"
    catalog["model_tokens"][0]["model_key"] = "gemini:claude-sonnet-5"
    _reject(catalog)


def test_capability_missing_dimension_rejected():
    catalog = base_catalog()
    del catalog["models"][0]["capability"]["levels"]["monitor"]
    _reject(catalog)


def test_capability_extra_dimension_rejected():
    catalog = base_catalog()
    catalog["models"][0]["capability"]["levels"]["speed"] = 3
    _reject(catalog)


@pytest.mark.parametrize("level", [-1, 6])
def test_capability_level_out_of_range_rejected(level):
    catalog = base_catalog()
    catalog["models"][0]["capability"]["levels"]["coding"] = level
    _reject(catalog)


def test_duplicate_global_rank_rejected():
    catalog = base_catalog()
    catalog["models"][1]["capability"]["global_rank"] = catalog["models"][0][
        "capability"
    ]["global_rank"]
    _reject(catalog)


def test_capability_provenance_not_maintainer_judgment_rejected():
    catalog = base_catalog()
    catalog["models"][0]["capability"]["provenance"]["type"] = "provider_benchmark"
    _reject(catalog)


def test_empty_models_rejected():
    catalog = base_catalog()
    catalog["models"] = []
    catalog["model_tokens"] = []
    _reject(catalog)


# --- model_tokens map ---


def test_duplicate_backend_token_pair_rejected():
    catalog = base_catalog()
    catalog["model_tokens"].append(
        {
            "backend": "claude",
            "token": "sonnet",
            "model_key": "claude:claude-sonnet-5",
            "primary": False,
        }
    )
    _reject(catalog)


def test_token_to_unknown_model_key_rejected():
    catalog = base_catalog()
    catalog["model_tokens"].append(
        {
            "backend": "claude",
            "token": "ghost",
            "model_key": "claude:ghost",
            "primary": False,
        }
    )
    _reject(catalog)


def test_token_backend_mismatch_rejected():
    catalog = base_catalog()
    catalog["model_tokens"].append(
        {
            "backend": "codex",
            "token": "sonnet-x",
            "model_key": "claude:claude-sonnet-5",
            "primary": False,
        }
    )
    _reject(catalog)


def test_active_model_without_primary_token_rejected():
    catalog = base_catalog()
    catalog["model_tokens"][0]["primary"] = False
    _reject(catalog)


def test_two_primary_tokens_for_one_model_rejected():
    catalog = base_catalog()
    catalog["model_tokens"].append(
        {
            "backend": "claude",
            "token": "sonnet-latest",
            "model_key": "claude:claude-sonnet-5",
            "primary": True,
        }
    )
    _reject(catalog)


def test_multiple_aliases_for_one_model_valid():
    catalog = base_catalog()
    catalog["model_tokens"].append(
        {
            "backend": "claude",
            "token": "claude-sonnet-5",
            "model_key": "claude:claude-sonnet-5",
            "primary": False,
        }
    )
    parsed = parse_catalog_markdown(catalog_markdown(catalog))
    claude_tokens = [
        entry for entry in parsed.model_tokens if entry.backend == "claude"
    ]
    assert {entry.token for entry in claude_tokens} == {"sonnet", "claude-sonnet-5"}
    assert [entry.token for entry in claude_tokens if entry.primary] == ["sonnet"]


# --- rate cards ---


def test_rate_card_missing_component_rejected():
    catalog = base_catalog()
    del catalog["models"][0]["rate_cards"][0]["components"]["cache_write"]
    _reject(catalog)


def test_supported_component_without_price_rejected():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["components"]["input"] = {
        "mode": "supported",
        "usd_per_mtok": None,
    }
    _reject(catalog)


def test_unsupported_component_with_price_rejected():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["components"]["input"] = {
        "mode": "unsupported",
        "usd_per_mtok": 1.0,
    }
    _reject(catalog)


def test_negative_component_price_rejected():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["components"]["input"]["usd_per_mtok"] = -1.0
    _reject(catalog)


def test_zero_price_supported_component_valid():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["components"]["cached_input"][
        "usd_per_mtok"
    ] = 0
    parsed = parse_catalog_markdown(catalog_markdown(catalog))
    assert parsed.schema_version == 1


def test_unknown_rate_card_status_value_rejected():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["status"] = "mystery"
    _reject(catalog)


def test_malformed_effective_date_rejected():
    catalog = base_catalog()
    catalog["models"][0]["rate_cards"][0]["effective_from"] = "07/01/2026"
    _reject(catalog)


def test_gateway_model_with_unknown_rate_card_parses():
    catalog = base_catalog()
    catalog["models"].append(opencode_gateway_model())
    catalog["model_tokens"].append(
        {
            "backend": "opencode",
            "token": "opencode/gpt-5.5",
            "model_key": "opencode:opencode/gpt-5.5",
            "primary": True,
        }
    )
    parsed = parse_catalog_markdown(catalog_markdown(catalog))
    gateway = next(model for model in parsed.models if model.backend == "opencode")
    assert [card.status for card in gateway.rate_cards] == ["unknown"]


# --- sources ---


@pytest.mark.parametrize("source", ["anthropic", "openai"])
def test_missing_required_source_rejected(source):
    catalog = base_catalog()
    del catalog["sources"][source]
    _reject(catalog)


def test_unapproved_source_url_rejected():
    catalog = base_catalog()
    catalog["sources"]["anthropic"]["url"] = "https://example.com/pricing"
    _reject(catalog)


def test_malformed_source_hash_rejected():
    catalog = base_catalog()
    catalog["sources"]["anthropic"]["content_sha256"] = "abc123"
    _reject(catalog)
