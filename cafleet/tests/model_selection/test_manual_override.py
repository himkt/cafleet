"""Manual-override resolution: a model pin bypasses automatic selection and is
estimated only when a fresh mapped rate card exists."""

import pytest

from cafleet.model_selection import ModelSelectionError, resolve_manual_override

from ._helpers import (
    SELECTION_NOW,
    catalog_with,
    make_model,
    parsed_catalog,
    primary_token_entry,
)


def _catalog():
    model = make_model(
        sku="claude-sonnet-5", rank=10, input_price=1.0, output_price=2.0
    )
    alias = {
        "backend": "claude",
        "token": "sonnet-alias",
        "model_key": "claude:claude-sonnet-5",
        "primary": False,
    }
    return catalog_with(
        [model],
        tokens=[primary_token_entry(model, token="sonnet")],
        extra_tokens=[alias],
    )


def test_model_pin_resolves_backend_and_estimate():
    result = resolve_manual_override(
        parsed_catalog(_catalog()),
        model="sonnet",
        now=SELECTION_NOW,
        token_estimate={
            "input": 10000,
            "cached_input": 0,
            "cache_write": 0,
            "output": 1000,
        },
    )
    assert result.policy == "manual_override"
    assert result.backend == "claude"
    assert result.estimated_usd == pytest.approx(0.012)


def test_alias_token_resolves_to_same_mapped_record():
    result = resolve_manual_override(
        parsed_catalog(_catalog()),
        model="sonnet-alias",
        now=SELECTION_NOW,
        token_estimate={
            "input": 10000,
            "cached_input": 0,
            "cache_write": 0,
            "output": 1000,
        },
    )
    assert result.backend == "claude"
    assert result.estimated_usd == pytest.approx(0.012)


def test_matching_backend_model_pair_accepted():
    result = resolve_manual_override(
        parsed_catalog(_catalog()),
        model="sonnet",
        backend="claude",
        now=SELECTION_NOW,
    )
    assert result.policy == "manual_override"
    assert result.backend == "claude"


def test_conflicting_backend_model_pair_rejected():
    with pytest.raises(ModelSelectionError) as exc_info:
        resolve_manual_override(
            parsed_catalog(_catalog()),
            model="sonnet",
            backend="codex",
            now=SELECTION_NOW,
        )
    assert exc_info.value.code == "MODEL_SELECTION_INVALID_REQUEST"


def test_unmapped_token_is_permitted_with_estimate_unavailable():
    result = resolve_manual_override(
        parsed_catalog(_catalog()),
        model="totally-custom-model",
        now=SELECTION_NOW,
    )
    assert result.policy == "manual_override"
    assert result.estimate_status == "unavailable"


def test_stale_catalog_makes_manual_estimate_unavailable():
    catalog = _catalog()
    catalog["sources"]["anthropic"]["retrieved_at"] = "2026-06-01T00:00:00Z"
    result = resolve_manual_override(
        parsed_catalog(catalog),
        model="sonnet",
        now=SELECTION_NOW,
        token_estimate={
            "input": 10000,
            "cached_input": 0,
            "cache_write": 0,
            "output": 1000,
        },
    )
    assert result.policy == "manual_override"
    assert result.estimate_status == "unavailable"
