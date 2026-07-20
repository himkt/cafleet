"""Manual model pins (``resolve_manual_override``) and the underpowered-member
replacement mode (``select_replacement``)."""

import pytest

from cafleet.model_selection import (
    ModelSelectionError,
    parse_model_list_markdown,
    resolve_manual_override,
    select_replacement,
)

from ._helpers import READY_BACKENDS, SELECTION_NOW, model_list_text, row

ESTIMATE = {"input": 10000, "cached_input": 0, "cache_write": 0, "output": 1000}


def _model_list(model_rows, **kwargs):
    return parse_model_list_markdown(model_list_text(model_rows, **kwargs))


def _pin_fixture(**kwargs):
    return _model_list(
        [row(model="claude-sonnet-5", aliases="sonnet", rank=10, out="2.0")], **kwargs
    )


# --- manual override ---


def test_model_pin_resolves_backend_and_estimate():
    for token in ("claude-sonnet-5", "sonnet"):
        result = resolve_manual_override(
            _pin_fixture(), model=token, now=SELECTION_NOW, token_estimate=ESTIMATE
        )
        assert result.policy == "manual_override"
        assert result.backend == "claude"
        assert result.model == "claude-sonnet-5"
        assert result.estimated_usd == pytest.approx(0.012)


def test_matching_backend_pin_accepted_and_conflicting_pin_rejected():
    result = resolve_manual_override(
        _pin_fixture(), model="sonnet", backend="claude", now=SELECTION_NOW
    )
    assert result.backend == "claude"
    with pytest.raises(ModelSelectionError) as exc_info:
        resolve_manual_override(
            _pin_fixture(), model="sonnet", backend="codex", now=SELECTION_NOW
        )
    assert exc_info.value.code == "MODEL_SELECTION_INVALID_REQUEST"


def test_cross_backend_ambiguous_token_requires_a_backend_pin():
    model_list = _model_list(
        [
            row(model="shared-model", rank=10),
            row(backend="codex", model="shared-model", rank=20),
        ]
    )
    with pytest.raises(ModelSelectionError) as exc_info:
        resolve_manual_override(model_list, model="shared-model", now=SELECTION_NOW)
    assert exc_info.value.code == "MODEL_SELECTION_INVALID_REQUEST"
    result = resolve_manual_override(
        model_list, model="shared-model", backend="codex", now=SELECTION_NOW
    )
    assert result.backend == "codex"


def test_unmapped_token_is_permitted_with_estimate_unavailable():
    result = resolve_manual_override(
        _pin_fixture(), model="totally-custom-model", now=SELECTION_NOW
    )
    assert result.policy == "manual_override"
    assert result.model_key is None
    assert result.estimate_status == "unavailable"


def test_stale_model_list_makes_manual_estimate_unavailable():
    result = resolve_manual_override(
        _pin_fixture(retrieved_at="2026-06-01T00:00:00Z"),
        model="sonnet",
        now=SELECTION_NOW,
        token_estimate=ESTIMATE,
    )
    assert result.policy == "manual_override"
    assert result.estimate_status == "unavailable"


# --- replacement ---


def _replace(model_rows, **kwargs):
    kwargs.setdefault("ready_backends", READY_BACKENDS)
    kwargs.setdefault("now", SELECTION_NOW)
    kwargs.setdefault("role", "programmer")
    kwargs.setdefault("failed_dimensions", ["coding"])
    return select_replacement(_model_list(model_rows), **kwargs)


def test_replacement_raises_failed_floor_and_requires_strictly_greater_rank():
    result = _replace(
        [
            row(model="failed-model", rank=10, inp="0.5", out="1.0"),
            row(model="lateral-model", rank=20, inp="0.5", out="1.0"),
            row(model="stronger-model", rank=30, cod=5, inp="2.0", out="4.0"),
        ],
        failed_model_key="claude:failed-model",
    )
    assert result.policy == "replacement_upgrade"
    assert result.selected.key == "claude:stronger-model"
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace(
            [
                row(model="failed-model", rank=30),
                row(model="lower-rank-stronger-model", rank=20, cod=5),
            ],
            failed_model_key="claude:failed-model",
        )
    assert exc_info.value.code == "MODEL_UPGRADE_UNAVAILABLE"


def test_replacement_minimizes_cost_within_stronger_set_and_skips_attempted():
    rows = [
        row(model="failed-model", rank=10),
        row(model="cheap-upgrade-model", rank=40, cod=5, inp="0.5", out="1.0"),
        row(model="costly-upgrade-model", rank=50, cod=5, inp="5.0", out="10.0"),
    ]
    result = _replace(rows, failed_model_key="claude:failed-model")
    assert result.selected.key == "claude:cheap-upgrade-model"
    result = _replace(
        rows,
        failed_model_key="claude:failed-model",
        attempted_model_keys=frozenset({"claude:cheap-upgrade-model"}),
    )
    assert result.selected.key == "claude:costly-upgrade-model"


def test_replacement_with_no_stronger_candidate_is_typed_upgrade_unavailable():
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace(
            [row(model="failed-model", rank=10)],
            failed_model_key="claude:failed-model",
        )
    assert exc_info.value.code == "MODEL_UPGRADE_UNAVAILABLE"


def test_replacement_with_unlisted_failed_model_is_invalid_request():
    with pytest.raises(ModelSelectionError) as exc_info:
        _replace([row(model="a-model", rank=10)], failed_model_key="claude:ghost-model")
    assert exc_info.value.code == "MODEL_SELECTION_INVALID_REQUEST"
