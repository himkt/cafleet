"""Rate-card activity windows, token caps, unsupported components, unknown cards,
and the UTC source-freshness rule."""

import pytest

from cafleet.model_selection import ModelSelectionError, select_model

from ._helpers import (
    READY_BACKENDS,
    SELECTION_NOW,
    catalog_with,
    known_rate_card,
    make_model,
    opencode_gateway_model,
    parsed_catalog,
)


def _select(payload, **kwargs):
    kwargs.setdefault("ready_backends", READY_BACKENDS)
    kwargs.setdefault("now", SELECTION_NOW)
    return select_model(parsed_catalog(payload), **kwargs)


def _code(exc_info):
    return exc_info.value.code


# --- source freshness (evaluated per required source, in UTC) ---


def test_source_older_than_freshness_days_raises_stale():
    model = make_model(sku="any-model", rank=10)
    catalog = catalog_with([model])
    catalog["sources"]["anthropic"]["retrieved_at"] = "2026-06-01T00:00:00Z"
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog, role="programmer")
    assert _code(exc_info) == "MODEL_CATALOG_STALE"


def test_source_more_than_five_minutes_in_future_raises_stale():
    model = make_model(sku="any-model", rank=10)
    catalog = catalog_with([model])
    catalog["sources"]["openai"]["retrieved_at"] = "2026-07-20T12:10:00Z"
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog, role="programmer")
    assert _code(exc_info) == "MODEL_CATALOG_STALE"


def test_source_within_five_minutes_after_now_is_fresh():
    model = make_model(sku="any-model", rank=10)
    catalog = catalog_with([model])
    catalog["sources"]["openai"]["retrieved_at"] = "2026-07-20T12:03:00Z"
    result = _select(catalog, role="programmer")
    assert result.selected.key == "claude:any-model"


# --- rate-card effective-date windows ---


def test_expired_rate_card_excludes_model():
    expired = make_model(
        sku="expired-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
        effective_until="2026-07-10",
    )
    current = make_model(
        sku="current-model", rank=20, input_price=2.0, output_price=4.0
    )
    result = _select(catalog_with([expired, current]), role="programmer")
    assert result.selected.key == "claude:current-model"
    excluded = next(c for c in result.candidates if c.key == "claude:expired-model")
    assert excluded.eligible is False


def test_not_yet_effective_rate_card_excludes_model():
    future = make_model(
        sku="future-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
        effective_from="2026-08-01",
    )
    current = make_model(
        sku="current-model", rank=20, input_price=2.0, output_price=4.0
    )
    result = _select(catalog_with([future, current]), role="programmer")
    assert result.selected.key == "claude:current-model"


# --- token caps and card choice ---


def _two_card_model():
    return make_model(
        sku="two-card-model",
        rank=10,
        rate_cards=[
            known_rate_card(
                card_id="short",
                input_price=0.5,
                output_price=1.0,
                max_total_tokens=10000,
            ),
            known_rate_card(
                card_id="long",
                input_price=2.0,
                output_price=4.0,
                max_total_tokens=200000,
            ),
        ],
    )


def test_least_cost_active_card_is_used_when_under_its_cap():
    result = _select(catalog_with([_two_card_model()]), role="scout")
    assert result.selected.estimated_usd == pytest.approx(0.003)


def test_request_beyond_card_cap_falls_to_larger_card():
    result = _select(catalog_with([_two_card_model()]), role="programmer")
    assert result.selected.estimated_usd == pytest.approx(0.096)


def test_request_beyond_every_card_cap_excludes_model():
    capped = make_model(
        sku="capped-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
        max_total_tokens=10000,
    )
    roomy = make_model(sku="roomy-model", rank=20, input_price=2.0, output_price=4.0)
    result = _select(catalog_with([capped, roomy]), role="programmer")
    assert result.selected.key == "claude:roomy-model"


# --- unsupported components ---


def _unsupported_cache_write_model(**kwargs):
    card = known_rate_card(input_price=1.0, cached_input_price=0.1, output_price=2.0)
    card["components"]["cache_write"] = {"mode": "unsupported", "usd_per_mtok": None}
    return make_model(sku="no-cache-write-model", rate_cards=[card], **kwargs)


def test_unsupported_component_with_nonzero_tokens_excludes_card():
    fallback = make_model(
        sku="fallback-model", rank=20, input_price=2.0, output_price=4.0
    )
    result = _select(
        catalog_with([_unsupported_cache_write_model(rank=10), fallback]),
        role="drafter",
        token_estimate={
            "input": 10000,
            "cached_input": 0,
            "cache_write": 4000,
            "output": 1000,
        },
    )
    assert result.selected.key == "claude:fallback-model"


def test_unsupported_component_with_zero_tokens_is_eligible():
    result = _select(
        catalog_with([_unsupported_cache_write_model(rank=10)]),
        role="drafter",
        token_estimate={
            "input": 10000,
            "cached_input": 0,
            "cache_write": 0,
            "output": 1000,
        },
    )
    assert result.selected.key == "claude:no-cache-write-model"
    assert result.selected.estimated_usd == pytest.approx(0.012)


# --- unknown-cost cards ---


def test_unknown_rate_card_model_not_an_automatic_candidate():
    gateway = opencode_gateway_model()
    known = make_model(sku="known-model", rank=20)
    catalog = catalog_with([known])
    catalog["models"].append(gateway)
    catalog["model_tokens"].append(
        {
            "backend": "opencode",
            "token": "opencode/gpt-5.5",
            "model_key": "opencode:opencode/gpt-5.5",
            "primary": True,
        }
    )
    result = _select(catalog, role="tester")
    assert result.selected.key == "claude:known-model"
    excluded = next(
        c for c in result.candidates if c.key == "opencode:opencode/gpt-5.5"
    )
    assert excluded.eligible is False


def test_only_unknown_rate_cards_raises_no_eligible_candidate():
    gateway = opencode_gateway_model()
    catalog = catalog_with([gateway])
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(catalog, role="tester")
    assert _code(exc_info) == "MODEL_NO_ELIGIBLE_CANDIDATE"
