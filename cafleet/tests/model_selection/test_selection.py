"""Ordinary cost-mode selection: constrained cost minimization over capability
floors, deterministic ties, override handling, and the structured result shape."""

import pytest

from cafleet.model_selection import ModelSelectionError, select_model

from ._helpers import (
    READY_BACKENDS,
    SELECTION_NOW,
    catalog_with,
    make_model,
    parsed_catalog,
    uniform_levels,
)


def _select(payload, **kwargs):
    kwargs.setdefault("ready_backends", READY_BACKENDS)
    kwargs.setdefault("now", SELECTION_NOW)
    return select_model(parsed_catalog(payload), **kwargs)


def _code(exc_info):
    return exc_info.value.code


def test_cheapest_eligible_model_selected():
    cheap = make_model(sku="cheap-model", rank=10, input_price=1.0, output_price=2.0)
    pricey = make_model(sku="pricey-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(catalog_with([cheap, pricey]), role="programmer")
    assert result.selected.key == "claude:cheap-model"
    assert result.selected.estimated_usd == pytest.approx(0.048)


def test_estimated_cost_matches_component_formula():
    model = make_model(
        backend="codex",
        sku="gpt-5.6-luna",
        rank=30,
        input_price=1.0,
        cached_input_price=0.1,
        cache_write_price=1.25,
        output_price=6.0,
    )
    result = _select(
        catalog_with([model]),
        role="drafter",
        token_estimate={
            "input": 12000,
            "cached_input": 0,
            "cache_write": 0,
            "output": 6000,
        },
    )
    assert result.selected.estimated_usd == pytest.approx(0.048)


def test_estimated_cost_includes_cached_and_cache_write_components():
    model = make_model(
        sku="cache-model",
        rank=10,
        input_price=1.0,
        cached_input_price=0.1,
        cache_write_price=1.25,
        output_price=6.0,
    )
    result = _select(
        catalog_with([model]),
        role="drafter",
        token_estimate={
            "input": 10000,
            "cached_input": 20000,
            "cache_write": 4000,
            "output": 1000,
        },
    )
    assert result.selected.estimated_usd == pytest.approx(0.023)


def test_capability_floor_excludes_cheaper_model():
    weak = make_model(
        sku="weak-model",
        rank=10,
        levels=uniform_levels(coding=3),
        input_price=0.5,
        output_price=1.0,
    )
    strong = make_model(sku="strong-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(catalog_with([weak, strong]), role="programmer")
    assert result.selected.key == "claude:strong-model"
    excluded = next(c for c in result.candidates if c.key == "claude:weak-model")
    assert excluded.eligible is False
    assert "coding" in excluded.reason


def test_requires_can_raise_listed_dimension():
    mid = make_model(sku="mid-model", rank=10, input_price=0.5, output_price=1.0)
    top = make_model(
        sku="top-model",
        rank=20,
        levels=uniform_levels(coding=5),
        input_price=2.0,
        output_price=4.0,
    )
    result = _select(
        catalog_with([mid, top]), role="programmer", requires={"coding": 5}
    )
    assert result.selected.key == "claude:top-model"


def test_requires_can_raise_previously_zero_dimension():
    lowres = make_model(
        sku="lowres-model",
        rank=10,
        levels=uniform_levels(research=3),
        input_price=0.5,
        output_price=1.0,
    )
    hires = make_model(sku="hires-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(
        catalog_with([lowres, hires]), role="tester", requires={"research": 4}
    )
    assert result.selected.key == "claude:hires-model"


def test_requires_below_profile_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="programmer", requires={"coding": 3})
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_unknown_role_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="wizard")
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_requires_above_five_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="programmer", requires={"coding": 6})
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_unknown_requires_dimension_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="programmer", requires={"vibes": 3})
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_negative_token_estimate_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(
            catalog_with([model]),
            role="programmer",
            token_estimate={"input": -1, "output": 100},
        )
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_token_estimate_defaults_from_role_token_profile():
    model = make_model(sku="any-model", rank=10, input_price=1.0, output_price=2.0)
    result = _select(catalog_with([model]), role="drafter")
    assert result.token_estimate.input == 12000
    assert result.token_estimate.output == 6000
    assert result.selected.estimated_usd == pytest.approx(0.024)


def test_explicit_token_estimate_replaces_profile():
    model = make_model(sku="any-model", rank=10, input_price=1.0, output_price=2.0)
    result = _select(
        catalog_with([model]),
        role="drafter",
        token_estimate={"input": 1000, "output": 500},
    )
    assert result.token_estimate.input == 1000
    assert result.token_estimate.output == 500
    assert result.selected.estimated_usd == pytest.approx(0.002)


def test_cost_tie_broken_by_higher_global_rank():
    alpha = make_model(sku="alpha-model", rank=10, input_price=1.0, output_price=2.0)
    beta = make_model(sku="beta-model", rank=20, input_price=1.0, output_price=2.0)
    result = _select(catalog_with([alpha, beta]), role="programmer")
    assert result.selected.key == "claude:beta-model"


def test_unready_backend_excluded():
    cheap_codex = make_model(
        backend="codex",
        sku="cheap-codex-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
    )
    claude = make_model(sku="claude-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(
        catalog_with([cheap_codex, claude]),
        role="programmer",
        ready_backends=frozenset({"claude"}),
    )
    assert result.selected.key == "claude:claude-model"


def test_no_ready_backend_raises_backend_unavailable():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="programmer", ready_backends=frozenset())
    assert _code(exc_info) == "MODEL_BACKEND_UNAVAILABLE"


def test_no_capable_candidate_raises_no_eligible_candidate():
    weak = make_model(sku="weak-model", rank=10, levels=uniform_levels(coding=3))
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([weak]), role="programmer")
    assert _code(exc_info) == "MODEL_NO_ELIGIBLE_CANDIDATE"


def test_backend_only_override_filters_candidates():
    cheap_claude = make_model(
        sku="cheap-claude-model", rank=10, input_price=0.5, output_price=1.0
    )
    codex = make_model(
        backend="codex",
        sku="codex-model",
        rank=20,
        input_price=2.0,
        output_price=4.0,
    )
    result = _select(
        catalog_with([cheap_claude, codex]), role="programmer", backend="codex"
    )
    assert result.selected.key == "codex:codex-model"


def test_unknown_backend_override_rejected():
    model = make_model(sku="any-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog_with([model]), role="programmer", backend="gemini")
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_selected_model_is_canonical_primary_token():
    model = make_model(sku="claude-sonnet-5", rank=10)
    alias = {
        "backend": "claude",
        "token": "sonnet-alias",
        "model_key": "claude:claude-sonnet-5",
        "primary": False,
    }
    result = _select(catalog_with([model], extra_tokens=[alias]), role="drafter")
    assert result.selected.backend == "claude"
    assert result.selected.model == "claude-sonnet-5"
    assert result.selected.canonical_token == "claude-sonnet-5"
    assert result.selected.effort is None


def test_ordinary_policy_and_task_profile_recorded():
    model = make_model(sku="any-model", rank=10)
    result = _select(catalog_with([model]), role="drafter")
    assert result.policy == "cost_minimized_subject_to_capability"
    assert result.role == "drafter"
    assert result.task_profile == {"planning": 3, "research": 2, "review": 1}


def test_inactive_model_is_not_a_candidate():
    inactive = make_model(
        sku="inactive-model",
        rank=10,
        active=False,
        input_price=0.1,
        output_price=0.2,
    )
    active = make_model(sku="active-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(catalog_with([inactive, active]), role="programmer")
    assert result.selected.key == "claude:active-model"
