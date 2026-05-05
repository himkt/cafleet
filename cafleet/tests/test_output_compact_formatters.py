"""Surfaces 3 + 17 — compact list formatters and dropped ``[N]`` index labels
(design 0000049 Step 6).

Pure-function tests for the slimmed text-mode renders of:

- ``format_agent(agent, *, full=False)`` — default 1-line, ``full=True`` restores
  the legacy 4-line layout.
- ``format_session_create(data, *, full=False)`` — default 1-line, ``full=True``
  restores the legacy 7-line layout.
- ``format_member(data, *, full=False)`` — default 1-line, ``full=True`` restores
  the legacy 6-line layout.
- ``format_indexed_list(items, formatter, empty_msg)`` — drop ``[N]`` index
  labels; items are separated by a single blank line.

The CLI integration (broadcast echo + ``--quiet`` flag wiring) lives in
``test_cli_compact_echo.py``.
"""

from cafleet import output


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def _agent(
    *,
    agent_id: str = "abcdef0123456789-tail",
    name: str = "Claude-B",
    description: str = "Reviewer for PR #42",
    status: str = "active",
) -> dict:
    return {
        "agent_id": agent_id,
        "name": name,
        "description": description,
        "status": status,
    }


def _session_create_data(
    *,
    session_id: str = "550e8400-e29b-41d4-a716-446655440000",
    director_agent_id: str = "7ba91234-5678-90ab-cdef-112233445566",
    label: str | None = "my-project",
    administrator_agent_id: str = "3c4d5e6f-7890-1234-5678-90abcdef1234",
) -> dict:
    return {
        "session_id": session_id,
        "label": label,
        "created_at": "2026-04-16T08:50:00+00:00",
        "administrator_agent_id": administrator_agent_id,
        "director": {
            "agent_id": director_agent_id,
            "name": "Director",
            "placement": {
                "tmux_session": "main",
                "tmux_window_id": "@3",
                "tmux_pane_id": "%0",
                "coding_agent": "claude",
            },
        },
    }


def _member_data(
    *,
    agent_id: str = "abcdef0123456789-tail",
    name: str = "Claude-B",
    pane_id: str = "%7",
    window_id: str = "@3",
    coding_agent: str = "claude",
) -> dict:
    return {
        "agent_id": agent_id,
        "name": name,
        "placement": {
            "tmux_pane_id": pane_id,
            "tmux_window_id": window_id,
            "coding_agent": coding_agent,
        },
    }


# ---------------------------------------------------------------------------
# 1. format_agent — 1-line default, --full restores 4-line
# ---------------------------------------------------------------------------


def test_format_agent__compact_renders_single_line():
    rendered = output.format_agent(_agent(), full=False)
    assert "\n" not in rendered, (
        f"compact format_agent must be a single line; got: {rendered!r}"
    )


def test_format_agent__compact_contains_id8():
    rendered = output.format_agent(
        _agent(agent_id="abcdef0123456789-tail"), full=False
    )
    assert "abcdef01" in rendered


def test_format_agent__compact_contains_name():
    rendered = output.format_agent(_agent(name="Claude-B"), full=False)
    assert "Claude-B" in rendered


def test_format_agent__compact_contains_status():
    rendered = output.format_agent(_agent(status="active"), full=False)
    assert "active" in rendered


def test_format_agent__compact_omits_description_by_default():
    """``description`` is verbose; it lives behind ``--full``."""
    rendered = output.format_agent(
        _agent(description="A very wordy description we don't want in lists"),
        full=False,
    )
    assert "A very wordy description" not in rendered


def test_format_agent__full_true_restores_legacy_4_line_layout():
    rendered = output.format_agent(_agent(), full=True)
    # Legacy layout used dedicated ``agent_id:``, ``name:``, ``description:``,
    # and ``status:`` lines.
    for needle in ("agent_id:", "name:", "description:", "status:"):
        assert needle in rendered, (
            f"legacy field label {needle!r} missing from full layout; got:\n{rendered}"
        )


def test_format_agent__full_true_has_more_lines_than_compact():
    full = output.format_agent(_agent(), full=True)
    compact = output.format_agent(_agent(), full=False)
    assert full.count("\n") > compact.count("\n")


def test_format_agent__default_kwarg_is_compact():
    """Calling without an explicit ``full`` kwarg matches ``full=False``."""
    assert output.format_agent(_agent()) == output.format_agent(_agent(), full=False)


# ---------------------------------------------------------------------------
# 2. format_session_create — 1-line default, --full restores 7-line
# ---------------------------------------------------------------------------


def test_format_session_create__compact_renders_single_line():
    rendered = output.format_session_create(_session_create_data(), full=False)
    assert "\n" not in rendered, (
        f"compact format_session_create must be a single line; got: {rendered!r}"
    )


def test_format_session_create__compact_contains_session_identifier():
    """At minimum, the compact line must surface enough of the session_id to
    identify it (full string or the 8-char prefix)."""
    sid = "550e8400-e29b-41d4-a716-446655440000"
    rendered = output.format_session_create(
        _session_create_data(session_id=sid), full=False
    )
    assert sid in rendered or sid[:8] in rendered, (
        f"compact line must surface session_id (full or 8-char prefix); got: {rendered!r}"
    )


def test_format_session_create__full_true_restores_legacy_7_line_layout():
    rendered = output.format_session_create(_session_create_data(), full=True)
    assert rendered.count("\n") >= 6, (
        f"full layout should be 7 lines (≥6 newlines); got: {rendered!r}"
    )


def test_format_session_create__full_true_contains_director_name_label():
    rendered = output.format_session_create(_session_create_data(), full=True)
    assert "director_name" in rendered


def test_format_session_create__full_true_contains_administrator_label():
    rendered = output.format_session_create(_session_create_data(), full=True)
    assert "administrator" in rendered


def test_format_session_create__full_true_contains_pane_label():
    rendered = output.format_session_create(_session_create_data(), full=True)
    assert "pane" in rendered


def test_format_session_create__full_true_has_more_lines_than_compact():
    full = output.format_session_create(_session_create_data(), full=True)
    compact = output.format_session_create(_session_create_data(), full=False)
    assert full.count("\n") > compact.count("\n")


def test_format_session_create__default_kwarg_is_compact():
    data = _session_create_data()
    assert output.format_session_create(data) == output.format_session_create(
        data, full=False
    )


# ---------------------------------------------------------------------------
# 3. format_member — 1-line default, --full restores 6-line
# ---------------------------------------------------------------------------


def test_format_member__compact_renders_single_line():
    rendered = output.format_member(_member_data(), full=False)
    assert "\n" not in rendered, (
        f"compact format_member must be a single line; got: {rendered!r}"
    )


def test_format_member__compact_contains_id8():
    rendered = output.format_member(
        _member_data(agent_id="abcdef0123456789-tail"), full=False
    )
    assert "abcdef01" in rendered


def test_format_member__compact_contains_name():
    rendered = output.format_member(_member_data(name="Claude-B"), full=False)
    assert "Claude-B" in rendered


def test_format_member__full_true_restores_legacy_6_line_layout():
    rendered = output.format_member(_member_data(), full=True)
    # Legacy layout used dedicated ``agent_id:``, ``name:``, ``backend:``,
    # ``pane_id:``, and ``window_id:`` lines plus the headline. ≥5 newlines.
    assert rendered.count("\n") >= 5, (
        f"full layout should be 6 lines (≥5 newlines); got: {rendered!r}"
    )
    for needle in ("agent_id:", "name:", "backend:", "pane_id:", "window_id:"):
        assert needle in rendered, (
            f"legacy field label {needle!r} missing from full layout; got:\n{rendered}"
        )


def test_format_member__full_true_has_more_lines_than_compact():
    full = output.format_member(_member_data(), full=True)
    compact = output.format_member(_member_data(), full=False)
    assert full.count("\n") > compact.count("\n")


def test_format_member__default_kwarg_is_compact():
    data = _member_data()
    assert output.format_member(data) == output.format_member(data, full=False)


# ---------------------------------------------------------------------------
# 4. format_indexed_list — drop [N]; items separated by single blank line
# ---------------------------------------------------------------------------


def test_format_indexed_list__drops_bracket_index_labels():
    items = ["alpha", "beta", "gamma"]
    rendered = output.format_indexed_list(items, lambda x: x, "empty")
    # Surface 17: no ``[1]``, ``[2]``, ``[3]`` prefixes.
    for legacy in ("[1]", "[2]", "[3]"):
        assert legacy not in rendered, (
            f"legacy index marker {legacy!r} must be dropped; got:\n{rendered}"
        )


def test_format_indexed_list__items_separated_by_single_blank_line():
    items = ["alpha", "beta", "gamma"]
    rendered = output.format_indexed_list(items, lambda x: x, "empty")
    # ``\n\n`` separates each adjacent pair.
    assert rendered == "alpha\n\nbeta\n\ngamma", (
        f"Surface 17 expects items joined by '\\n\\n'; got: {rendered!r}"
    )


def test_format_indexed_list__single_item_has_no_blank_separator():
    items = ["alpha"]
    rendered = output.format_indexed_list(items, lambda x: x, "empty")
    assert rendered == "alpha"


def test_format_indexed_list__empty_returns_empty_msg_unchanged():
    rendered = output.format_indexed_list([], lambda x: x, "No items found.")
    assert rendered == "No items found."


def test_format_indexed_list__multiline_formatter_output_preserved():
    """A formatter that itself returns multi-line text — items still join with a single
    blank line between them; per-item internal newlines pass through."""
    items = [{"a": 1}, {"a": 2}]
    rendered = output.format_indexed_list(
        items, lambda d: f"line1: {d['a']}\nline2: more", "empty"
    )
    assert rendered == "line1: 1\nline2: more\n\nline1: 2\nline2: more"
