"""Compact rendered envelope.

Tests for ``output.render_task`` compact projection, ``output.format_json``
compact rendering, and the 2-line ``output.format_task`` text rendering. The
5-task fixture budget assertion enforces that the compact slim envelope stays
materially smaller than the compact full (untruncated) envelope.
"""

import json
import uuid

import pytest

from cafleet import output


def _typed_task(
    *,
    task_id: str | None = None,
    from_agent_id: str | None = None,
    to_agent_id: str | None = None,
    context_id: str | None = None,
    text: str = "the body",
    type_: str = "unicast",
    status_state: str = "input_required",
    status_timestamp: str = "2026-05-05T12:00:00.000000+00:00",
    created_at: str = "2026-05-05T12:00:00.000000+00:00",
    origin_task_id: str | None = None,
) -> dict:
    tid = task_id or str(uuid.uuid4())
    fid = from_agent_id or str(uuid.uuid4())
    rid = to_agent_id or str(uuid.uuid4())
    return {
        "task_id": tid,
        "context_id": context_id or rid,
        "from_agent_id": fid,
        "to_agent_id": rid,
        "type": type_,
        "created_at": created_at,
        "status_state": status_state,
        "status_timestamp": status_timestamp,
        "origin_task_id": origin_task_id,
        "text": text,
    }


def test_render_task__compact_typical_unicast_shape():
    task = _typed_task(
        task_id="abcdef0123456789-tail",
        from_agent_id="zyxwvutsrq-tail",
        status_timestamp="2026-04-01T08:09:10.111213+00:00",
        text="the actual body content",
    )
    rendered = output.render_task(task, full=False)

    assert set(rendered.keys()) == {"id", "from", "ts", "text"}
    assert rendered["id"] == "abcdef01"
    assert rendered["from"] == "zyxwvuts"
    assert rendered["ts"] == "2026-04-01T08:09:10.111213+00:00"
    assert rendered["text"] == "the actual body content"


@pytest.mark.parametrize(
    "forbidden_key",
    [
        "to_agent_id",
        "context_id",
        "status_state",
        "created_at",
        "type",
        "metadata",
        "artifacts",
        "history",
        "contextId",
        "fromAgentId",
        "origin_task_id",
    ],
)
def test_render_task__compact_drops_default_state_and_metadata(forbidden_key):
    task = _typed_task(status_state="input_required", origin_task_id="origin01-tail")
    rendered = output.render_task(task, full=False)
    assert forbidden_key not in rendered


def test_render_task__compact_broadcast_summary_shape_includes_kind_and_origin():
    task = _typed_task(
        type_="broadcast_summary",
        origin_task_id="origin01-tail-stuff-here",
    )
    rendered = output.render_task(task, full=False)
    assert set(rendered.keys()) == {"id", "from", "ts", "text", "kind", "origin"}
    assert rendered["kind"] == "broadcast_summary"
    assert rendered["origin"] == "origin01"


@pytest.mark.parametrize(
    ("type_", "origin_task_id", "expects_kind", "expects_origin"),
    [
        ("unicast", None, False, False),
        ("unicast", "origin01-tail-stuff-here", False, True),
        ("broadcast_summary", None, True, False),
        ("broadcast_summary", "origin01-tail-stuff-here", True, True),
    ],
)
def test_render_task__compact_kind_and_origin_present_only_when_meaningful(
    type_, origin_task_id, expects_kind, expects_origin
):
    task = _typed_task(type_=type_, origin_task_id=origin_task_id)
    rendered = output.render_task(task, full=False)
    assert ("kind" in rendered) is expects_kind
    assert ("origin" in rendered) is expects_origin


@pytest.mark.parametrize(
    "long_form_key",
    ["to_agent_id", "context_id", "status_state", "task_id", "text"],
)
def test_render_task__full_preserves_long_form_keys(long_form_key):
    task = _typed_task()
    rendered = output.render_task(task, full=True)
    assert long_form_key in rendered


def test_format_json__compact_no_whitespace_and_round_trips():
    data = {"a": 1, "b": [2, 3], "c": {"d": "ok"}}
    out = output.format_json(data)
    assert "\n" not in out
    assert ", " not in out
    assert ": " not in out
    assert json.loads(out) == data


@pytest.mark.parametrize(
    ("line", "needle"),
    [
        (0, "["),
        (0, "abcdef01"),
        (0, "from:zyxwvuts"),
        (0, "2026-04-01T08:09:10.111213+00:00"),
        (1, "the body of the message"),
    ],
)
def test_format_task__compact_two_lines_with_expected_fields(line, needle):
    task = _typed_task(
        task_id="abcdef0123456789-tail",
        from_agent_id="zyxwvutsrq-tail",
        status_timestamp="2026-04-01T08:09:10.111213+00:00",
        text="the body of the message",
    )
    rendered = output.format_task(task, full=False)
    parts = rendered.split("\n")
    assert len(parts) == 2
    if line == 0 and needle == "[":
        assert parts[0].startswith("[")
    elif line == 1 and needle == "the body of the message":
        assert parts[1] == needle
    else:
        assert needle in parts[line]
    # Compact default matches full=False and envelope unwrap works.
    assert output.format_task(task) == rendered
    assert output.format_task({"task": task}, full=False) == rendered


def test_format_task__full_legacy_layout_has_more_lines_and_field_labels():
    task = _typed_task(text="legacy body")
    compact = output.format_task(task, full=False)
    full = output.format_task(task, full=True)
    assert full.count("\n") > compact.count("\n")
    for needle in ("id:", "state:", "from:", "to:", "type:", "text:"):
        assert needle in full


@pytest.fixture
def five_task_fixture() -> list[dict]:
    summary_id = "11111111-2222-3333-4444-555555555555"
    return [
        _typed_task(
            task_id="aaaaaaaa-1111-2222-3333-444444444444",
            from_agent_id="ffffffff-1111-2222-3333-444444444444",
            text="Did the API schema change?",
            type_="unicast",
        ),
        _typed_task(
            task_id="bbbbbbbb-1111-2222-3333-444444444444",
            from_agent_id="11111111-2222-3333-4444-555555555555",
            text="Yes — see migration 0042.",
            type_="unicast",
        ),
        _typed_task(
            task_id="cccccccc-1111-2222-3333-444444444444",
            from_agent_id="dddddddd-1111-2222-3333-444444444444",
            text="Build failed on main branch.",
            type_="unicast",
            origin_task_id=summary_id,
        ),
        _typed_task(
            task_id="eeeeeeee-1111-2222-3333-444444444444",
            from_agent_id="dddddddd-1111-2222-3333-444444444444",
            text="Build failed on main branch.",
            type_="unicast",
            origin_task_id=summary_id,
        ),
        _typed_task(
            task_id=summary_id,
            from_agent_id="dddddddd-1111-2222-3333-444444444444",
            text="Broadcast sent to 2 recipients",
            type_="broadcast_summary",
            origin_task_id=summary_id,
            status_state="completed",
        ),
    ]


def test_budget__compact_slim_json_smaller_than_compact_full(five_task_fixture):
    """Compact slim (projected ``render_task``) JSON stays materially smaller
    than the compact full (untruncated typed-column) JSON for the same fixture."""
    compact_full = output.format_json(five_task_fixture)
    compact_slim = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture]
    )
    full_len = len(compact_full)
    slim_len = len(compact_slim)
    ratio = slim_len / full_len
    assert ratio <= 0.40, (
        f"compact slim is {ratio:.0%} of compact full "
        f"(slim={slim_len} chars, full={full_len} chars); budget ≤ 40%."
    )


def test_budget__compact_json_no_whitespace_separators(five_task_fixture):
    compact = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture]
    )
    assert "\n" not in compact
    assert ", " not in compact
    assert ": " not in compact


def test_budget__compact_json_round_trips(five_task_fixture):
    compact = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture]
    )
    parsed = json.loads(compact)
    assert isinstance(parsed, list)
    assert len(parsed) == 5
