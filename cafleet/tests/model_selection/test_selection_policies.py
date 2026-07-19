"""Special-role policies: monitor minimum-cost and reviewer maximum-capability selection."""

import pytest

from cafleet.model_selection import select_model

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


def test_monitor_selects_cheapest_meeting_monitor_baseline():
    unfit = make_model(
        sku="unfit-model",
        rank=10,
        levels=uniform_levels(monitor=1),
        input_price=0.1,
        output_price=0.2,
    )
    fit_cheap = make_model(
        sku="fit-cheap-model",
        rank=20,
        levels=uniform_levels(monitor=2),
        input_price=0.5,
        output_price=1.0,
    )
    fit_costly = make_model(
        sku="fit-costly-model",
        rank=30,
        levels=uniform_levels(monitor=5),
        input_price=5.0,
        output_price=10.0,
    )
    result = _select(catalog_with([unfit, fit_cheap, fit_costly]), role="monitor")
    assert result.selected.key == "claude:fit-cheap-model"
    assert result.selected.estimated_usd == pytest.approx(0.003)


def test_monitor_uses_small_token_profile():
    model = make_model(sku="any-model", rank=10)
    result = _select(catalog_with([model]), role="monitor")
    assert result.token_estimate.input == 4000
    assert result.token_estimate.output == 1000


def test_reviewer_selects_highest_rank_meeting_baseline():
    cheap_low_rank = make_model(
        sku="cheap-low-rank-model",
        rank=10,
        levels=uniform_levels(5),
        input_price=0.5,
        output_price=1.0,
    )
    costly_high_rank = make_model(
        sku="costly-high-rank-model",
        rank=30,
        input_price=5.0,
        output_price=10.0,
    )
    result = _select(catalog_with([cheap_low_rank, costly_high_rank]), role="reviewer")
    assert result.selected.key == "claude:costly-high-rank-model"
    assert result.selected.estimated_usd == pytest.approx(5.0 * 0.012 + 10.0 * 0.006)


def test_reviewer_baseline_still_filters_top_rank():
    weak_top_rank = make_model(
        sku="weak-top-rank-model",
        rank=30,
        levels=uniform_levels(review=3),
    )
    fit_low_rank = make_model(sku="fit-low-rank-model", rank=10)
    result = _select(catalog_with([weak_top_rank, fit_low_rank]), role="reviewer")
    assert result.selected.key == "claude:fit-low-rank-model"
