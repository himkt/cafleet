"""``select_model``: cost minimization over capability floors, the monitor and
reviewer policy exceptions, freshness, pricing edge cases, and deterministic ties."""

import pytest

from cafleet.model_selection import (
    ModelSelectionError,
    parse_model_list_markdown,
    select_model,
)

from ._helpers import (
    READY_BACKENDS,
    SELECTION_NOW,
    model_list_text,
    row,
    unpriced_row,
)


def _select(model_rows, *, retrieved_at="2026-07-19T00:00:00Z", **kwargs):
    kwargs.setdefault("ready_backends", READY_BACKENDS)
    kwargs.setdefault("now", SELECTION_NOW)
    model_list = parse_model_list_markdown(
        model_list_text(model_rows, retrieved_at=retrieved_at)
    )
    return select_model(model_list, **kwargs)


def _code(exc_info):
    return exc_info.value.code


def test_cheapest_eligible_model_selected():
    result = _select(
        [
            row(model="cheap-model", rank=10, inp="1.0", out="2.0"),
            row(model="pricey-model", rank=20, inp="2.0", out="4.0"),
        ],
        role="programmer",
    )
    assert result.selected.key == "claude:cheap-model"
    assert result.selected.estimated_usd == pytest.approx(24000 / 1e6 + 12000 / 1e6 * 2)


def test_estimated_cost_covers_all_four_components():
    result = _select(
        [
            row(
                model="a-model",
                rank=10,
                inp="1.0",
                cached="0.1",
                write="1.25",
                out="6.0",
            )
        ],
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
    result = _select(
        [
            row(model="weak-model", rank=10, cod=3, inp="0.5", out="1.0"),
            row(model="strong-model", rank=20, inp="2.0", out="4.0"),
        ],
        role="programmer",
    )
    assert result.selected.key == "claude:strong-model"
    excluded = next(c for c in result.candidates if c.key == "claude:weak-model")
    assert excluded.eligible is False
    assert "coding" in excluded.reason


def test_requires_can_only_raise_the_role_floor():
    rows = [
        row(model="mid-model", rank=10, inp="0.5", out="1.0"),
        row(model="top-model", rank=20, cod=5, inp="2.0", out="4.0"),
    ]
    result = _select(rows, role="programmer", requires={"coding": 5})
    assert result.selected.key == "claude:top-model"
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(rows, role="programmer", requires={"coding": 3})
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


@pytest.mark.parametrize(
    "kwargs",
    [
        {"role": "wizard"},
        {"role": "programmer", "requires": {"coding": 6}},
        {"role": "programmer", "requires": {"vibes": 3}},
        {"role": "programmer", "token_estimate": {"input": -1}},
        {"role": "programmer", "backend": "gemini"},
    ],
)
def test_invalid_request_rejected(kwargs):
    with pytest.raises(ModelSelectionError) as exc_info:
        _select([row(model="any-model", rank=10)], **kwargs)
    assert _code(exc_info) == "MODEL_SELECTION_INVALID_REQUEST"


def test_token_estimate_defaults_from_role_token_profile():
    result = _select([row(model="any-model", rank=10, out="2.0")], role="drafter")
    assert result.token_estimate.input == 12000
    assert result.token_estimate.output == 6000
    assert result.selected.estimated_usd == pytest.approx(0.024)


def test_cost_tie_broken_by_higher_rank():
    result = _select(
        [
            row(model="alpha-model", rank=10, out="2.0"),
            row(model="beta-model", rank=20, out="2.0"),
        ],
        role="programmer",
    )
    assert result.selected.key == "claude:beta-model"


def test_backend_override_and_readiness_filter_candidates():
    rows = [
        row(model="cheap-claude-model", rank=10, inp="0.5", out="1.0"),
        row(backend="codex", model="codex-model", rank=20, inp="2.0", out="4.0"),
    ]
    assert (
        _select(rows, role="programmer", backend="codex").selected.key
        == "codex:codex-model"
    )
    assert (
        _select(
            rows, role="programmer", ready_backends=frozenset({"codex"})
        ).selected.key
        == "codex:codex-model"
    )
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(rows, role="programmer", ready_backends=frozenset())
    assert _code(exc_info) == "MODEL_BACKEND_UNAVAILABLE"


def test_no_capable_candidate_raises_no_eligible_candidate():
    with pytest.raises(ModelSelectionError) as exc_info:
        _select([row(model="weak-model", rank=10, cod=3)], role="programmer")
    assert _code(exc_info) == "MODEL_NO_ELIGIBLE_CANDIDATE"


def test_inactive_model_is_not_a_candidate():
    result = _select(
        [
            row(model="inactive-model", rank=10, active="no", inp="0.1", out="0.2"),
            row(model="active-model", rank=20, inp="2.0", out="4.0"),
        ],
        role="programmer",
    )
    assert result.selected.key == "claude:active-model"


def test_ordinary_policy_and_task_profile_recorded():
    result = _select([row(model="any-model", rank=10)], role="drafter")
    assert result.policy == "cost_minimized_subject_to_capability"
    assert result.task_profile == {"planning": 3, "research": 2, "review": 1}
    assert result.selected.model == "any-model"
    assert result.selected.effort is None


# --- freshness ---


def test_stale_source_raises_model_list_stale():
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(
            [row(model="any-model", rank=10)],
            retrieved_at="2026-06-01T00:00:00Z",
            role="programmer",
        )
    assert _code(exc_info) == "MODEL_LIST_STALE"


def test_source_more_than_five_minutes_in_future_raises_stale():
    with pytest.raises(ModelSelectionError) as exc_info:
        _select(
            [row(model="any-model", rank=10)],
            retrieved_at="2026-07-20T12:10:00Z",
            role="programmer",
        )
    assert _code(exc_info) == "MODEL_LIST_STALE"


def test_source_within_five_minutes_after_now_is_fresh():
    result = _select(
        [row(model="any-model", rank=10)],
        retrieved_at="2026-07-20T12:03:00Z",
        role="programmer",
    )
    assert result.selected.key == "claude:any-model"


# --- pricing edge cases ---


def test_estimate_beyond_max_total_tokens_excludes_model():
    result = _select(
        [
            row(model="capped-model", rank=10, inp="0.5", out="1.0", max_tokens=10000),
            row(model="roomy-model", rank=20, inp="2.0", out="4.0"),
        ],
        role="programmer",
    )
    assert result.selected.key == "claude:roomy-model"


def test_unpriced_component_with_nonzero_tokens_excludes_model():
    rows = [
        row(model="no-cache-write-model", rank=10, write="—", out="2.0"),
        row(model="fallback-model", rank=20, inp="2.0", out="4.0"),
    ]
    estimate = {"input": 10000, "cached_input": 0, "cache_write": 4000, "output": 1000}
    result = _select(rows, role="drafter", token_estimate=estimate)
    assert result.selected.key == "claude:fallback-model"
    result = _select(
        rows, role="drafter", token_estimate={**estimate, "cache_write": 0}
    )
    assert result.selected.key == "claude:no-cache-write-model"


def test_unpriced_gateway_model_is_never_an_automatic_candidate():
    result = _select([unpriced_row(), row(model="known-model", rank=20)], role="tester")
    assert result.selected.key == "claude:known-model"
    excluded = next(
        c for c in result.candidates if c.key == "opencode:opencode/gpt-5.5"
    )
    assert excluded.eligible is False
    with pytest.raises(ModelSelectionError) as exc_info:
        _select([unpriced_row()], role="tester")
    assert _code(exc_info) == "MODEL_NO_ELIGIBLE_CANDIDATE"


# --- monitor and reviewer policy exceptions ---


def test_monitor_selects_cheapest_meeting_monitor_baseline():
    result = _select(
        [
            row(model="unfit-model", rank=10, mon=1, inp="0.1", out="0.2"),
            row(model="fit-cheap-model", rank=20, mon=2, inp="0.5", out="1.0"),
            row(model="fit-costly-model", rank=30, mon=5, inp="5.0", out="10.0"),
        ],
        role="monitor",
    )
    assert result.selected.key == "claude:fit-cheap-model"
    assert result.token_estimate.input == 4000
    assert result.token_estimate.output == 1000


def test_reviewer_selects_highest_rank_meeting_baseline():
    result = _select(
        [
            row(
                model="cheap-low-rank-model",
                rank=10,
                cod=5,
                pln=5,
                rsc=5,
                rev=5,
                mon=5,
                inp="0.5",
                out="1.0",
            ),
            row(model="costly-high-rank-model", rank=30, inp="5.0", out="10.0"),
            row(model="weak-top-rank-model", rank=40, rev=3),
        ],
        role="reviewer",
    )
    assert result.policy == "reviewer_maximum_capability"
    assert result.selected.key == "claude:costly-high-rank-model"
