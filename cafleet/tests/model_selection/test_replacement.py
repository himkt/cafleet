"""Replacement-selection mode: raised capability floors, strictly greater rank,
no repeated model/task pairs, and the typed no-upgrade outcome."""

import pytest

from cafleet.model_selection import ModelSelectionError, select_replacement

from ._helpers import (
    READY_BACKENDS,
    SELECTION_NOW,
    catalog_with,
    make_model,
    parsed_catalog,
    uniform_levels,
)


def _replace(payload, **kwargs):
    kwargs.setdefault("ready_backends", READY_BACKENDS)
    kwargs.setdefault("now", SELECTION_NOW)
    kwargs.setdefault("role", "programmer")
    kwargs.setdefault("failed_dimensions", ["coding"])
    return select_replacement(parsed_catalog(payload), **kwargs)


def _code(exc_info):
    return exc_info.value.code


def test_replacement_raises_failed_floor_and_selects_stronger_model():
    failed = make_model(sku="failed-model", rank=10, input_price=0.5, output_price=1.0)
    lateral = make_model(
        sku="lateral-model", rank=20, input_price=0.5, output_price=1.0
    )
    stronger = make_model(
        sku="stronger-model",
        rank=30,
        levels=uniform_levels(coding=5),
        input_price=2.0,
        output_price=4.0,
    )
    result = _replace(
        catalog_with([failed, lateral, stronger]),
        failed_model_key="claude:failed-model",
    )
    assert result.selected.key == "claude:stronger-model"


def test_replacement_requires_strictly_greater_rank():
    failed = make_model(sku="failed-model", rank=30)
    lower_rank_stronger = make_model(
        sku="lower-rank-stronger-model",
        rank=20,
        levels=uniform_levels(coding=5),
    )
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace(
            catalog_with([failed, lower_rank_stronger]),
            failed_model_key="claude:failed-model",
        )
    assert _code(exc_info) == "MODEL_UPGRADE_UNAVAILABLE"


def test_replacement_minimizes_cost_within_stronger_set():
    failed = make_model(sku="failed-model", rank=10)
    cheap_upgrade = make_model(
        sku="cheap-upgrade-model",
        rank=40,
        levels=uniform_levels(coding=5),
        input_price=0.5,
        output_price=1.0,
    )
    costly_upgrade = make_model(
        sku="costly-upgrade-model",
        rank=50,
        levels=uniform_levels(coding=5),
        input_price=5.0,
        output_price=10.0,
    )
    result = _replace(
        catalog_with([failed, cheap_upgrade, costly_upgrade]),
        failed_model_key="claude:failed-model",
    )
    assert result.selected.key == "claude:cheap-upgrade-model"


def test_replacement_skips_already_attempted_models():
    failed = make_model(sku="failed-model", rank=10)
    cheap_upgrade = make_model(
        sku="cheap-upgrade-model",
        rank=40,
        levels=uniform_levels(coding=5),
        input_price=0.5,
        output_price=1.0,
    )
    costly_upgrade = make_model(
        sku="costly-upgrade-model",
        rank=50,
        levels=uniform_levels(coding=5),
        input_price=5.0,
        output_price=10.0,
    )
    result = _replace(
        catalog_with([failed, cheap_upgrade, costly_upgrade]),
        failed_model_key="claude:failed-model",
        attempted_model_keys=frozenset({"claude:cheap-upgrade-model"}),
    )
    assert result.selected.key == "claude:costly-upgrade-model"


def test_replacement_excludes_unready_backend():
    failed = make_model(sku="failed-model", rank=10)
    codex_upgrade = make_model(
        backend="codex",
        sku="codex-upgrade-model",
        rank=40,
        levels=uniform_levels(coding=5),
    )
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace(
            catalog_with([failed, codex_upgrade]),
            failed_model_key="claude:failed-model",
            ready_backends=frozenset({"claude"}),
        )
    assert _code(exc_info) == "MODEL_UPGRADE_UNAVAILABLE"


def test_no_stronger_candidate_is_typed_upgrade_unavailable():
    failed = make_model(sku="failed-model", rank=10)
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace(catalog_with([failed]), failed_model_key="claude:failed-model")
    assert _code(exc_info) == "MODEL_UPGRADE_UNAVAILABLE"
