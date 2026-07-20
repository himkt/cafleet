"""``parse_model_list_markdown``: the three fixed tables, cell validation, and
the committed ``skills/cafleet/reference/model-list.md`` seed file."""

from pathlib import Path

import pytest

from cafleet.model_selection import (
    ROLE_PROFILES,
    TOKEN_PROFILES,
    ModelListInvalidError,
    parse_model_list_markdown,
)

from ._helpers import (
    MODELS_HEADER,
    MODELS_SEPARATOR,
    model_list_text,
    row,
    unpriced_row,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
MODEL_LIST_PATH = REPO_ROOT / "skills" / "cafleet" / "reference" / "model-list.md"


def _reject(text: str):
    with pytest.raises(ModelListInvalidError):
        parse_model_list_markdown(text)


def _base_text(**kwargs):
    return model_list_text(
        [
            row(model="claude-sonnet-5", aliases="sonnet", rank=20),
            row(backend="codex", model="gpt-5.6-luna", rank=30, out="6.0"),
        ],
        **kwargs,
    )


# --- structure ---


def test_valid_model_list_parses_with_typed_models():
    model_list = parse_model_list_markdown(_base_text())
    assert model_list.schema_version == 1
    assert set(model_list.sources) == {"anthropic", "openai"}
    sonnet = next(m for m in model_list.models if m.backend == "claude")
    assert sonnet.key == "claude:claude-sonnet-5"
    assert sonnet.tokens == ("claude-sonnet-5", "sonnet")
    assert sonnet.prices["output"] == 6.0
    assert sonnet.levels["review"] == 4


def test_missing_section_rejected():
    text = _base_text()
    _reject(text[: text.index("## Models")])


def test_out_of_order_or_unknown_section_rejected():
    _reject(_base_text().replace("## Sources", "## Extras"))
    _reject(_base_text() + "\n## Extras\n\n| A |\n|---|\n")


def test_prose_inside_a_section_rejected():
    _reject(_base_text().replace("## Sources", "## Sources\n\nIntervening prose."))


def test_wrong_column_header_rejected():
    _reject(_base_text().replace("| Rank |", "| Grade |"))


def test_missing_separator_row_rejected():
    _reject(
        _base_text().replace(MODELS_HEADER + "\n" + MODELS_SEPARATOR, MODELS_HEADER)
    )


def test_wrong_cell_count_rejected():
    _reject(_base_text() + "| claude | stray-model |\n")


def test_table_row_outside_a_section_rejected():
    _reject("| A | B |\n" + _base_text())


def test_unknown_schema_version_rejected():
    _reject(_base_text().replace("| schema_version | 1 |", "| schema_version | 2 |"))


def test_metadata_extra_or_missing_field_rejected():
    _reject(_base_text().replace("| freshness_days | 30 |", "| currency | USD |"))
    _reject(_base_text().replace("| freshness_days | 30 |\n", ""))


def test_naive_or_malformed_timestamp_rejected():
    _reject(_base_text(retrieved_at="2026-07-19T00:00:00"))
    _reject(_base_text(retrieved_at="yesterday"))


# --- sources ---


def test_missing_required_source_rejected():
    text = _base_text()
    openai_line = next(
        line for line in text.splitlines() if line.startswith("| openai")
    )
    _reject(text.replace(openai_line + "\n", ""))


def test_duplicate_source_rejected():
    text = _base_text()
    openai_line = next(
        line for line in text.splitlines() if line.startswith("| openai")
    )
    _reject(text.replace(openai_line, openai_line + "\n" + openai_line))


def test_unapproved_source_url_rejected():
    _reject(
        _base_text().replace(
            "https://developers.openai.com/api/docs/pricing",
            "https://example.com/pricing",
        )
    )


def test_malformed_source_hash_rejected():
    _reject(_base_text().replace("a" * 64, "abc123"))


# --- models ---


def test_empty_models_table_rejected():
    _reject(model_list_text([]))


def test_unknown_backend_rejected():
    _reject(model_list_text([row(backend="gemini", model="some-model", rank=10)]))


def test_duplicate_model_key_rejected():
    _reject(
        model_list_text(
            [row(model="dup-model", rank=10), row(model="dup-model", rank=20)]
        )
    )


def test_duplicate_rank_rejected():
    _reject(
        model_list_text([row(model="a-model", rank=10), row(model="b-model", rank=10)])
    )


def test_duplicate_backend_token_pair_rejected():
    _reject(
        model_list_text(
            [
                row(model="a-model", aliases="shared", rank=10),
                row(model="b-model", aliases="shared", rank=20),
            ]
        )
    )


def test_empty_alias_rejected():
    _reject(model_list_text([row(model="a-model", aliases="a,,b", rank=10)]))


def test_capability_level_out_of_range_rejected():
    _reject(model_list_text([row(model="a-model", rank=10, cod=6)]))
    _reject(model_list_text([row(model="a-model", rank=10, cod=-1)]))


def test_malformed_active_cell_rejected():
    _reject(model_list_text([row(model="a-model", rank=10, active="maybe")]))


def test_negative_or_malformed_price_rejected():
    _reject(model_list_text([row(model="a-model", rank=10, inp="-1.0")]))
    _reject(model_list_text([row(model="a-model", rank=10, inp="$1")]))


def test_zero_price_is_an_explicitly_free_component():
    model_list = parse_model_list_markdown(
        model_list_text([row(model="a-model", rank=10, write="0.0")])
    )
    assert model_list.models[0].prices["cache_write"] == 0.0


def test_unpriced_gateway_row_parses_with_none_prices():
    model_list = parse_model_list_markdown(model_list_text([unpriced_row()]))
    assert all(price is None for price in model_list.models[0].prices.values())


def test_non_positive_max_tokens_rejected():
    _reject(model_list_text([row(model="a-model", rank=10, max_tokens=0)]))


# --- code-constant profiles ---


def test_token_profiles_are_the_three_reviewed_profiles():
    assert set(TOKEN_PROFILES) == {"small", "standard", "large"}
    assert TOKEN_PROFILES["standard"].input == 12000
    assert TOKEN_PROFILES["standard"].output == 6000


def test_role_profiles_cover_every_workflow_role():
    assert set(ROLE_PROFILES) == {
        "monitor",
        "drafter",
        "reviewer",
        "analyzer",
        "programmer",
        "tester",
        "verifier",
        "manager",
        "scout",
        "researcher",
        "web_researcher",
        "transcript",
        "presentation",
        "visual_reviewer",
    }
    assert ROLE_PROFILES["monitor"].requires == {"monitor": 2}
    assert ROLE_PROFILES["programmer"].requires == {
        "coding": 4,
        "planning": 3,
        "review": 2,
    }
    assert ROLE_PROFILES["reviewer"].requires == {"review": 4, "planning": 3}
    assert all(
        profile.token_profile in TOKEN_PROFILES for profile in ROLE_PROFILES.values()
    )


# --- committed seed file ---


@pytest.fixture(scope="module")
def committed_model_list():
    return parse_model_list_markdown(MODEL_LIST_PATH.read_text(encoding="utf-8"))


def test_committed_model_list_parses_with_approved_sources(committed_model_list):
    assert committed_model_list.schema_version == 1
    assert set(committed_model_list.sources) == {"anthropic", "openai"}


def test_committed_model_list_has_active_priced_claude_and_codex_models(
    committed_model_list,
):
    for backend in ("claude", "codex"):
        models = [
            m for m in committed_model_list.models if m.backend == backend and m.active
        ]
        assert models, f"no active {backend} model in the seeded model list"
        assert any(
            all(price is not None for price in model.prices.values())
            for model in models
        )


def test_committed_model_list_maps_the_documented_direct_tokens(committed_model_list):
    tokens = {
        (model.backend, token)
        for model in committed_model_list.models
        for token in model.tokens
    }
    assert {("claude", t) for t in ("fable", "opus", "sonnet", "haiku")} <= tokens
    assert {
        ("codex", t)
        for t in (
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
        )
    } <= tokens


def test_committed_gateway_models_are_unpriced(committed_model_list):
    for model in committed_model_list.models:
        if model.backend == "opencode":
            assert all(price is None for price in model.prices.values())
