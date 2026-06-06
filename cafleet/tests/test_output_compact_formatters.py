"""Compact list formatters and dropped ``[N]`` index labels."""

from cafleet import output


def _agent(
    *,
    agent_id="abcdef0123456789-tail",
    name="Claude-B",
    description="Reviewer for PR #42",
    status="active",
) -> dict:
    return {
        "agent_id": agent_id,
        "name": name,
        "description": description,
        "status": status,
    }


def _fleet_create_data(
    *,
    fleet_id="550e8400-e29b-41d4-a716-446655440000",
    director_agent_id="7ba91234-5678-90ab-cdef-112233445566",
    label="my-project",
    administrator_agent_id="3c4d5e6f-7890-1234-5678-90abcdef1234",
) -> dict:
    return {
        "fleet_id": fleet_id,
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
    agent_id="abcdef0123456789-tail",
    name="Claude-B",
    pane_id="%7",
    window_id="@3",
    coding_agent="claude",
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


def test_format_agent__compact_shape_and_default_kwarg():
    agent = _agent(
        agent_id="abcdef0123456789-tail",
        name="Claude-B",
        status="active",
        description="A very wordy description we don't want in lists",
    )
    rendered = output.format_agent(agent, full=False)
    assert "\n" not in rendered
    assert "abcdef01" in rendered
    assert "Claude-B" in rendered
    assert "active" in rendered
    # Description is dropped in compact form.
    assert "A very wordy description" not in rendered
    # Default kwarg matches full=False.
    assert output.format_agent(_agent()) == output.format_agent(_agent(), full=False)


def test_format_agent__full_layout_has_labels_and_more_lines():
    full = output.format_agent(_agent(), full=True)
    compact = output.format_agent(_agent(), full=False)
    for needle in ("agent_id:", "name:", "description:", "status:"):
        assert needle in full
    assert full.count("\n") > compact.count("\n")


def test_format_fleet_create__compact_shape_and_default_kwarg():
    rendered = output.format_fleet_create(_fleet_create_data(), full=False)
    assert "\n" not in rendered
    sid = "550e8400-e29b-41d4-a716-446655440000"
    assert sid in rendered or sid[:8] in rendered
    data = _fleet_create_data()
    assert output.format_fleet_create(data) == output.format_fleet_create(
        data, full=False
    )


def test_format_fleet_create__full_layout_has_labels_and_more_lines():
    full = output.format_fleet_create(_fleet_create_data(), full=True)
    compact = output.format_fleet_create(_fleet_create_data(), full=False)
    assert full.count("\n") >= 6
    for needle in ("director_name", "administrator", "pane"):
        assert needle in full
    assert full.count("\n") > compact.count("\n")


def test_format_member__compact_shape_and_default_kwarg():
    rendered = output.format_member(
        _member_data(agent_id="abcdef0123456789-tail", name="Claude-B"),
        full=False,
    )
    assert "\n" not in rendered
    assert "abcdef01" in rendered
    assert "Claude-B" in rendered
    data = _member_data()
    assert output.format_member(data) == output.format_member(data, full=False)


def test_format_member__full_layout_has_labels_and_more_lines():
    full = output.format_member(_member_data(), full=True)
    compact = output.format_member(_member_data(), full=False)
    assert full.count("\n") >= 5
    for needle in ("agent_id:", "name:", "backend:", "pane_id:", "window_id:"):
        assert needle in full
    assert full.count("\n") > compact.count("\n")


def test_format_indexed_list__drops_index_and_blank_line_separated():
    items = ["alpha", "beta", "gamma"]
    rendered = output.format_indexed_list(items, lambda x: x, "empty")
    for index_marker in ("[1]", "[2]", "[3]"):
        assert index_marker not in rendered
    assert rendered == "alpha\n\nbeta\n\ngamma"
    # Single item: no blank separator.
    assert output.format_indexed_list(["alpha"], lambda x: x, "empty") == "alpha"
    # Empty list: empty message verbatim.
    assert (
        output.format_indexed_list([], lambda x: x, "No items found.")
        == "No items found."
    )


def test_format_indexed_list__multiline_formatter_output_preserved():
    items = [{"a": 1}, {"a": 2}]
    rendered = output.format_indexed_list(
        items,
        lambda d: f"line1: {d['a']}\nline2: more",
        "empty",
    )
    assert rendered == "line1: 1\nline2: more\n\nline1: 2\nline2: more"
