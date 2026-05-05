"""Surface 1 — compact rendered envelope (design 0000049 Step 3).

Tests for the new ``output.render_task`` projection helper, the new
``output.format_json`` ``pretty`` parameter, and the updated 2-line
``output.format_task`` text rendering. The 5-task fixture-based budget
assertion at the bottom is the load-bearing reduction check named in the
design doc — compact mode must be ≤ 30 % of indented-JSON length.
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
    """Build a flat typed-column task dict matching the post-Surface-14 shape."""
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


# ---------------------------------------------------------------------------
# 1. render_task — compact projection
# ---------------------------------------------------------------------------


def test_render_task__compact_has_required_minimum_keys():
    task = _typed_task()
    rendered = output.render_task(task, full=False)
    for key in ("id", "from", "ts", "text"):
        assert key in rendered, f"required key {key!r} missing from compact render"


def test_render_task__compact_id_is_first_8_chars_of_task_id():
    task = _typed_task(task_id="abcdef0123456789-tail")
    rendered = output.render_task(task, full=False)
    assert rendered["id"] == "abcdef01"


def test_render_task__compact_from_is_first_8_chars_of_from_agent_id():
    task = _typed_task(from_agent_id="zyxwvutsrq-tail")
    rendered = output.render_task(task, full=False)
    assert rendered["from"] == "zyxwvuts"


def test_render_task__compact_ts_equals_status_timestamp():
    ts = "2026-04-01T08:09:10.111213+00:00"
    task = _typed_task(status_timestamp=ts)
    rendered = output.render_task(task, full=False)
    assert rendered["ts"] == ts


def test_render_task__compact_text_equals_body():
    task = _typed_task(text="the actual body content")
    rendered = output.render_task(task, full=False)
    assert rendered["text"] == "the actual body content"


def test_render_task__compact_drops_to_agent_id():
    task = _typed_task()
    rendered = output.render_task(task, full=False)
    assert "to_agent_id" not in rendered


def test_render_task__compact_drops_context_id():
    task = _typed_task()
    rendered = output.render_task(task, full=False)
    assert "context_id" not in rendered


def test_render_task__compact_drops_default_status_state():
    task = _typed_task(status_state="input_required")
    rendered = output.render_task(task, full=False)
    assert "status_state" not in rendered


def test_render_task__compact_drops_created_at():
    """``created_at`` is metadata for migration / DB; not surfaced in compact render."""
    task = _typed_task()
    rendered = output.render_task(task, full=False)
    assert "created_at" not in rendered


def test_render_task__compact_omits_kind_when_type_is_unicast():
    task = _typed_task(type_="unicast")
    rendered = output.render_task(task, full=False)
    assert "kind" not in rendered


def test_render_task__compact_includes_kind_when_type_is_not_unicast():
    task = _typed_task(type_="broadcast_summary")
    rendered = output.render_task(task, full=False)
    assert rendered.get("kind") == "broadcast_summary"


def test_render_task__compact_drops_type_key_in_favor_of_kind():
    """``type`` is the storage column; the rendered alias is ``kind``."""
    task = _typed_task(type_="broadcast_summary")
    rendered = output.render_task(task, full=False)
    assert "type" not in rendered


def test_render_task__compact_omits_origin_when_origin_task_id_is_none():
    task = _typed_task(origin_task_id=None)
    rendered = output.render_task(task, full=False)
    assert "origin" not in rendered


def test_render_task__compact_includes_origin_when_origin_task_id_present():
    task = _typed_task(origin_task_id="origin01-tail-stuff-here")
    rendered = output.render_task(task, full=False)
    assert rendered["origin"] == "origin01"


def test_render_task__compact_drops_origin_task_id_long_form():
    task = _typed_task(origin_task_id="origin01-tail-stuff-here")
    rendered = output.render_task(task, full=False)
    assert "origin_task_id" not in rendered


def test_render_task__compact_drops_legacy_camelcase_aliases():
    """Forbidden in compact: any legacy camelCase or wrapper keys."""
    task = _typed_task()
    rendered = output.render_task(task, full=False)
    for forbidden in ("metadata", "artifacts", "history", "contextId", "fromAgentId"):
        assert forbidden not in rendered


def test_render_task__compact_unicast_typical_shape_has_only_4_keys():
    """Unicast with no origin: minimal shape is exactly {id, from, ts, text}."""
    task = _typed_task(type_="unicast", origin_task_id=None)
    rendered = output.render_task(task, full=False)
    assert set(rendered.keys()) == {"id", "from", "ts", "text"}


def test_render_task__compact_broadcast_summary_with_origin_has_6_keys():
    """broadcast_summary + origin: shape is {id, from, ts, text, kind, origin}."""
    task = _typed_task(
        type_="broadcast_summary",
        origin_task_id="origin01-tail-stuff-here",
    )
    rendered = output.render_task(task, full=False)
    assert set(rendered.keys()) == {"id", "from", "ts", "text", "kind", "origin"}


def test_render_task__full_true_includes_to_agent_id():
    task = _typed_task()
    rendered = output.render_task(task, full=True)
    assert "to_agent_id" in rendered


def test_render_task__full_true_includes_context_id():
    task = _typed_task()
    rendered = output.render_task(task, full=True)
    assert "context_id" in rendered


def test_render_task__full_true_includes_status_state():
    task = _typed_task()
    rendered = output.render_task(task, full=True)
    assert "status_state" in rendered


def test_render_task__full_true_keeps_long_uuids_unprefixed():
    """The full mode preserves original UUIDs; no prefix-rendering."""
    long_task_id = "abcdef0123456789-tail"
    task = _typed_task(task_id=long_task_id)
    rendered = output.render_task(task, full=True)
    assert rendered["task_id"] == long_task_id


def test_render_task__full_true_does_not_drop_text():
    task = _typed_task(text="full-mode body")
    rendered = output.render_task(task, full=True)
    assert rendered["text"] == "full-mode body"


# ---------------------------------------------------------------------------
# 2. format_json — compact-by-default with pretty kwarg
# ---------------------------------------------------------------------------


def test_format_json__default_is_compact_no_whitespace():
    """Default ``format_json`` uses ``separators=(',',':')`` — no spaces, no newlines."""
    out = output.format_json({"a": 1, "b": [2, 3]})
    assert "\n" not in out
    assert ", " not in out
    assert ": " not in out


def test_format_json__default_round_trips_via_json_loads():
    data = {"a": 1, "b": [2, 3], "c": {"d": "ok"}}
    out = output.format_json(data)
    assert json.loads(out) == data


def test_format_json__pretty_false_explicit_matches_default():
    data = {"a": 1, "b": [2, 3]}
    assert output.format_json(data) == output.format_json(data, pretty=False)


def test_format_json__pretty_true_emits_indented_form():
    out = output.format_json({"a": 1, "b": [2, 3]}, pretty=True)
    assert "\n" in out
    # indent=2 places two spaces before every nested key/element
    assert "  " in out


def test_format_json__pretty_true_round_trips_via_json_loads():
    data = {"a": 1, "b": [2, 3], "c": {"d": "ok"}}
    out = output.format_json(data, pretty=True)
    assert json.loads(out) == data


def test_format_json__pretty_true_is_strictly_longer_than_default():
    data = {"a": 1, "b": [2, 3], "c": {"d": "ok"}}
    assert len(output.format_json(data, pretty=True)) > len(output.format_json(data))


# ---------------------------------------------------------------------------
# 3. format_task — text mode 2 lines per task; full=True legacy 5-line
# ---------------------------------------------------------------------------


def test_format_task__compact_renders_two_lines():
    task = _typed_task(
        task_id="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        from_agent_id="ffffffff-1111-2222-3333-444444444444",
        text="hello world",
    )
    rendered = output.format_task(task, full=False)
    assert rendered.count("\n") == 1, (
        f"expected exactly 2 lines (1 newline) in compact format_task, "
        f"got: {rendered!r}"
    )


def test_format_task__compact_first_line_starts_with_open_bracket():
    task = _typed_task()
    rendered = output.format_task(task, full=False)
    line1 = rendered.split("\n")[0]
    assert line1.startswith("[")


def test_format_task__compact_first_line_contains_id8():
    task = _typed_task(task_id="abcdef0123456789-tail")
    rendered = output.format_task(task, full=False)
    line1 = rendered.split("\n")[0]
    assert "abcdef01" in line1


def test_format_task__compact_first_line_contains_from8_with_label():
    task = _typed_task(from_agent_id="zyxwvutsrq-tail")
    rendered = output.format_task(task, full=False)
    line1 = rendered.split("\n")[0]
    assert "from:zyxwvuts" in line1


def test_format_task__compact_first_line_contains_timestamp():
    ts = "2026-04-01T08:09:10.111213+00:00"
    task = _typed_task(status_timestamp=ts)
    rendered = output.format_task(task, full=False)
    line1 = rendered.split("\n")[0]
    assert ts in line1


def test_format_task__compact_second_line_is_text_body():
    task = _typed_task(text="the body of the message")
    rendered = output.format_task(task, full=False)
    line2 = rendered.split("\n")[1]
    assert line2 == "the body of the message"


def test_format_task__compact_unwraps_envelope_shape():
    """``format_task({'task': <flat-dict>})`` works just like passing the dict directly."""
    task = _typed_task(text="envelope body")
    envelope = {"task": task}
    rendered = output.format_task(envelope, full=False)
    assert "envelope body" in rendered
    assert rendered.count("\n") == 1


def test_format_task__full_true_emits_legacy_verbose_layout():
    task = _typed_task(
        text="legacy body",
        type_="unicast",
        status_state="input_required",
    )
    rendered = output.format_task(task, full=True)
    # Legacy verbose layout used dedicated "id:", "state:", "from:", "to:",
    # "type:", and "text:" prefixed lines.
    for needle in ("id:", "state:", "from:", "to:", "type:", "text:"):
        assert needle in rendered, (
            f"legacy field label {needle!r} missing from full output"
        )


def test_format_task__full_true_has_more_lines_than_compact():
    task = _typed_task()
    compact = output.format_task(task, full=False)
    full = output.format_task(task, full=True)
    assert full.count("\n") > compact.count("\n")


def test_format_task__compact_default_value_when_full_kwarg_absent():
    """Missing ``full`` kwarg defaults to the compact 2-line render."""
    task = _typed_task()
    assert output.format_task(task) == output.format_task(task, full=False)


# ---------------------------------------------------------------------------
# 4. Budget assertion — design doc Step 3, 5-task fixture
# ---------------------------------------------------------------------------


@pytest.fixture
def five_task_fixture() -> list[dict]:
    """Five typed-column tasks chosen to be representative.

    Two unicast (no origin), two unicast that came from a broadcast (origin
    set), one broadcast_summary. Bodies and timestamps are realistic length.
    """
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


def test_budget__compact_json_at_most_30_percent_of_pretty_baseline(five_task_fixture):
    """Surface 1 reduction target: compact JSON ≤ 30 % of indented-JSON length."""
    pretty_baseline = output.format_json(five_task_fixture, pretty=True)
    compact_rendered = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture],
        pretty=False,
    )
    pretty_len = len(pretty_baseline)
    compact_len = len(compact_rendered)
    ratio = compact_len / pretty_len
    assert ratio <= 0.30, (
        f"compact mode is {ratio:.0%} of pretty baseline "
        f"(compact={compact_len} chars, pretty={pretty_len} chars); "
        "Surface 1 target is ≤ 30 %."
    )


def test_budget__compact_json_no_whitespace_separators(five_task_fixture):
    compact = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture],
        pretty=False,
    )
    assert "\n" not in compact
    assert ", " not in compact
    assert ": " not in compact


def test_budget__compact_json_round_trips(five_task_fixture):
    """The compact render must remain valid JSON that round-trips through json.loads."""
    compact = output.format_json(
        [output.render_task(t, full=False) for t in five_task_fixture],
        pretty=False,
    )
    parsed = json.loads(compact)
    assert isinstance(parsed, list)
    assert len(parsed) == 5
