"""Tests for ``cafleet.output`` formatting helpers."""

from cafleet.output import format_member, format_member_list


def _member(**placement_overrides) -> dict:
    placement = {
        "tmux_pane_id": "%7",
        "tmux_window_id": "@3",
        "coding_agent": "claude",
    }
    placement.update(placement_overrides)
    return {
        "agent_id": "agent-001",
        "name": "Claude-B",
        "placement": placement,
    }


def _list_entry(*, agent_id: str, name: str, coding_agent: str, pane_id: str) -> dict:
    return {
        "agent_id": agent_id,
        "name": name,
        "status": "active",
        "registered_at": "2026-04-12T10:15:00Z",
        "placement": {
            "director_agent_id": "dir-001",
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": coding_agent,
            "created_at": "2026-04-12T10:15:00Z",
        },
    }


class TestFormatMember:
    def test_includes_backend_line(self):
        assert "backend:" in format_member(_member())

    def test_backend_shows_codex(self):
        result = format_member(_member(coding_agent="codex"))
        assert "codex" in result

    def test_backend_shows_claude(self):
        result = format_member(_member())
        assert "claude" in result


class TestFormatMemberList:
    def test_table_header_includes_backend(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Claude-B",
                    coding_agent="claude",
                    pane_id="%7",
                )
            ]
        )
        assert "backend" in result.lower()

    def test_row_shows_codex_backend(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Codex-B",
                    coding_agent="codex",
                    pane_id="%7",
                )
            ]
        )
        data_lines = [line for line in result.split("\n") if "Codex-B" in line]
        assert len(data_lines) == 1
        assert "codex" in data_lines[0]

    def test_row_shows_claude_backend(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Claude-B",
                    coding_agent="claude",
                    pane_id="%7",
                )
            ]
        )
        data_lines = [line for line in result.split("\n") if "Claude-B" in line]
        assert len(data_lines) == 1
        assert "claude" in data_lines[0]

    def test_mixed_backends(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Claude-M",
                    coding_agent="claude",
                    pane_id="%7",
                ),
                _list_entry(
                    agent_id="agent-002",
                    name="Codex-M",
                    coding_agent="codex",
                    pane_id="%8",
                ),
            ]
        )
        lines = result.split("\n")
        claude_line = next(line for line in lines if "Claude-M" in line)
        codex_line = next(line for line in lines if "Codex-M" in line)
        assert "claude" in claude_line
        assert "codex" in codex_line

    def test_empty_list_unchanged(self):
        assert "0 members" in format_member_list([])
