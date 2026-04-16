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
        """format_member_list() table header includes 'backend' column."""
        members = [
            {
                "agent_id": "agent-001",
                "name": "Claude-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%7",
                    "coding_agent": "claude",
                    "created_at": "2026-04-12T10:15:00Z",
                },
            }
        ]
        result = format_member_list(members)
        assert "backend" in result.lower()

    def test_row_shows_codex_backend(self):
        """A member with coding_agent='codex' shows 'codex' in its row."""
        members = [
            {
                "agent_id": "agent-001",
                "name": "Codex-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%7",
                    "coding_agent": "codex",
                    "created_at": "2026-04-12T10:15:00Z",
                },
            }
        ]
        result = format_member_list(members)
        lines = result.split("\n")
        # Find the data row (not header/separator)
        data_lines = [line for line in lines if "Codex-B" in line]
        assert len(data_lines) == 1
        assert "codex" in data_lines[0]

    def test_row_shows_claude_backend(self):
        """A member with coding_agent='claude' shows 'claude' in its row."""
        members = [
            {
                "agent_id": "agent-001",
                "name": "Claude-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%7",
                    "coding_agent": "claude",
                    "created_at": "2026-04-12T10:15:00Z",
                },
            }
        ]
        result = format_member_list(members)
        lines = result.split("\n")
        data_lines = [line for line in lines if "Claude-B" in line]
        assert len(data_lines) == 1
        assert "claude" in data_lines[0]

    def test_mixed_backends(self):
        """A list with both claude and codex members shows correct backends."""
        members = [
            {
                "agent_id": "agent-001",
                "name": "Claude-M",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%7",
                    "coding_agent": "claude",
                    "created_at": "2026-04-12T10:15:00Z",
                },
            },
            {
                "agent_id": "agent-002",
                "name": "Codex-M",
                "status": "active",
                "registered_at": "2026-04-12T10:16:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%8",
                    "coding_agent": "codex",
                    "created_at": "2026-04-12T10:16:00Z",
                },
            },
        ]
        result = format_member_list(members)
        lines = result.split("\n")
        claude_line = [line for line in lines if "Claude-M" in line][0]
        codex_line = [line for line in lines if "Codex-M" in line][0]
        assert "claude" in claude_line
        assert "codex" in codex_line

    def test_empty_list_unchanged(self):
        """Empty member list still returns '0 members.'."""
        result = format_member_list([])
        assert "0 members" in result
