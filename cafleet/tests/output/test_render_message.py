"""Compact rendered envelope.

Tests for ``output.render_message`` compact projection, ``output.format_json``
compact rendering, and the 2-line ``output.format_message`` text rendering. The
5-message fixture budget assertion enforces that the compact slim envelope stays
materially smaller than the compact full (untruncated) envelope.
"""

import json

import pytest

from cafleet import output


def _typed_message(
    *,
    message_id: int = 101,
    from_member_id: int = 202,
    to_member_id: int = 303,
    owner_member_id: int | None = None,
    text: str = "the body",
    type_: str = "unicast",
    status_state: str = "input_required",
    status_timestamp: str = "2026-05-05T12:00:00.000000+00:00",
    created_at: str = "2026-05-05T12:00:00.000000+00:00",
    origin_message_id: int | None = None,
) -> dict:
    return {
        "message_id": message_id,
        "owner_member_id": owner_member_id
        if owner_member_id is not None
        else to_member_id,
        "from_member_id": from_member_id,
        "to_member_id": to_member_id,
        "type": type_,
        "created_at": created_at,
        "status_state": status_state,
        "status_timestamp": status_timestamp,
        "origin_message_id": origin_message_id,
        "text": text,
    }


def test_render_message__compact_typical_unicast_shape():
    message = _typed_message(
        message_id=42,
        from_member_id=7,
        status_timestamp="2026-04-01T08:09:10.111213+00:00",
        text="the actual body content",
    )
    rendered = output.render_message(message, full=False)

    assert set(rendered.keys()) == {"id", "from", "ts", "text"}
    assert rendered["id"] == 42
    assert rendered["from"] == 7
    assert rendered["ts"] == "2026-04-01T08:09:10.111213+00:00"
    assert rendered["text"] == "the actual body content"


@pytest.mark.parametrize(
    "forbidden_key",
    [
        "to_member_id",
        "owner_member_id",
        "status_state",
        "created_at",
        "type",
        "metadata",
        "artifacts",
        "history",
        "contextId",
        "fromAgentId",
        "origin_message_id",
    ],
)
def test_render_message__compact_drops_default_state_and_metadata(forbidden_key):
    message = _typed_message(status_state="input_required", origin_message_id=99)
    rendered = output.render_message(message, full=False)
    assert forbidden_key not in rendered


def test_render_message__compact_broadcast_summary_shape_includes_kind_and_origin():
    message = _typed_message(
        type_="broadcast_summary",
        origin_message_id=99,
    )
    rendered = output.render_message(message, full=False)
    assert set(rendered.keys()) == {"id", "from", "ts", "text", "kind", "origin"}
    assert rendered["kind"] == "broadcast_summary"
    assert rendered["origin"] == 99


@pytest.mark.parametrize(
    ("type_", "origin_message_id", "expects_kind", "expects_origin"),
    [
        ("unicast", None, False, False),
        ("unicast", 99, False, True),
        ("broadcast_summary", None, True, False),
        ("broadcast_summary", 99, True, True),
    ],
)
def test_render_message__compact_kind_and_origin_present_only_when_meaningful(
    type_, origin_message_id, expects_kind, expects_origin
):
    message = _typed_message(type_=type_, origin_message_id=origin_message_id)
    rendered = output.render_message(message, full=False)
    assert ("kind" in rendered) is expects_kind
    assert ("origin" in rendered) is expects_origin


@pytest.mark.parametrize(
    "long_form_key",
    ["to_member_id", "owner_member_id", "status_state", "message_id", "text"],
)
def test_render_message__full_preserves_long_form_keys(long_form_key):
    message = _typed_message()
    rendered = output.render_message(message, full=True)
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
        (0, "42"),
        (0, "from:7"),
        (0, "2026-04-01T08:09:10.111213+00:00"),
        (1, "the body of the message"),
    ],
)
def test_format_message__compact_two_lines_with_expected_fields(line, needle):
    message = _typed_message(
        message_id=42,
        from_member_id=7,
        status_timestamp="2026-04-01T08:09:10.111213+00:00",
        text="the body of the message",
    )
    rendered = output.format_message(message, full=False)
    parts = rendered.split("\n")
    assert len(parts) == 2
    if line == 0 and needle == "[":
        assert parts[0].startswith("[")
    elif line == 1 and needle == "the body of the message":
        assert parts[1] == needle
    else:
        assert needle in parts[line]
    # Compact default matches full=False and envelope unwrap works.
    assert output.format_message(message) == rendered
    assert output.format_message({"message": message}, full=False) == rendered


def test_format_message__full_layout_has_more_lines_and_field_labels():
    message = _typed_message(text="message body")
    compact = output.format_message(message, full=False)
    full = output.format_message(message, full=True)
    assert full.count("\n") > compact.count("\n")
    for needle in ("id:", "state:", "from:", "to:", "type:", "text:"):
        assert needle in full


@pytest.fixture
def five_message_fixture() -> list[dict]:
    summary_id = 2001
    return [
        _typed_message(
            message_id=1001,
            from_member_id=11,
            text="Did the API schema change?",
            type_="unicast",
        ),
        _typed_message(
            message_id=1002,
            from_member_id=22,
            text="Yes — see migration 0042.",
            type_="unicast",
        ),
        _typed_message(
            message_id=1003,
            from_member_id=44,
            text="Build failed on main branch.",
            type_="unicast",
            origin_message_id=summary_id,
        ),
        _typed_message(
            message_id=1004,
            from_member_id=44,
            text="Build failed on main branch.",
            type_="unicast",
            origin_message_id=summary_id,
        ),
        _typed_message(
            message_id=summary_id,
            from_member_id=44,
            text="Broadcast sent to 2 recipients",
            type_="broadcast_summary",
            origin_message_id=summary_id,
            status_state="completed",
        ),
    ]


def test_budget__compact_slim_json_smaller_than_compact_full(five_message_fixture):
    """Compact slim (projected ``render_message``) JSON stays materially smaller
    than the compact full (untruncated typed-column) JSON for the same fixture.

    Integer ids are short by construction, so the slim projection saves nothing
    on id length; the savings come from dropping several typed columns and one
    of the two timestamps. The budget is ≤ 55% accordingly."""
    compact_full = output.format_json(five_message_fixture)
    compact_slim = output.format_json(
        [output.render_message(m, full=False) for m in five_message_fixture]
    )
    full_len = len(compact_full)
    slim_len = len(compact_slim)
    ratio = slim_len / full_len
    assert ratio <= 0.55, (
        f"compact slim is {ratio:.0%} of compact full "
        f"(slim={slim_len} chars, full={full_len} chars); budget ≤ 55%."
    )


def test_budget__compact_json_no_whitespace_separators(five_message_fixture):
    compact = output.format_json(
        [output.render_message(m, full=False) for m in five_message_fixture]
    )
    assert "\n" not in compact
    assert ", " not in compact
    assert ": " not in compact


def test_budget__compact_json_round_trips(five_message_fixture):
    compact = output.format_json(
        [output.render_message(m, full=False) for m in five_message_fixture]
    )
    parsed = json.loads(compact)
    assert isinstance(parsed, list)
    assert len(parsed) == 5
