"""Tests for hikyaku.output — output formatting functions.

Design doc 0000018 Step 7: format_member() and format_member_list()
include coding_agent (displayed as 'backend').
"""

from hikyaku.output import format_member, format_member_list


# ---------------------------------------------------------------------------
# format_member
# ---------------------------------------------------------------------------


class TestFormatMember:
    def test_includes_backend_line(self):
        """format_member() output includes a 'backend:' line."""
        data = {
            "agent_id": "agent-001",
            "name": "Claude-B",
            "placement": {
                "tmux_pane_id": "%7",
                "tmux_window_id": "@3",
                "coding_agent": "claude",
            },
        }
        result = format_member(data)
        assert "backend:" in result

    def test_backend_shows_codex(self):
        """format_member() shows 'codex' when coding_agent is 'codex'."""
        data = {
            "agent_id": "agent-001",
            "name": "Codex-B",
            "placement": {
                "tmux_pane_id": "%7",
                "tmux_window_id": "@3",
                "coding_agent": "codex",
            },
        }
        result = format_member(data)
        assert "codex" in result

    def test_backend_shows_claude(self):
        """format_member() shows 'claude' when coding_agent is 'claude'."""
        data = {
            "agent_id": "agent-001",
            "name": "Claude-B",
            "placement": {
                "tmux_pane_id": "%7",
                "tmux_window_id": "@3",
                "coding_agent": "claude",
            },
        }
        result = format_member(data)
        assert "claude" in result

    def test_backend_defaults_to_claude_when_missing(self):
        """format_member() defaults to 'claude' when coding_agent key is absent."""
        data = {
            "agent_id": "agent-001",
            "name": "Claude-B",
            "placement": {
                "tmux_pane_id": "%7",
                "tmux_window_id": "@3",
            },
        }
        result = format_member(data)
        assert "claude" in result


# ---------------------------------------------------------------------------
# format_member_list
# ---------------------------------------------------------------------------


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
        data_lines = [l for l in lines if "Codex-B" in l]
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
        data_lines = [l for l in lines if "Claude-B" in l]
        assert len(data_lines) == 1
        assert "claude" in data_lines[0]

    def test_defaults_to_claude_when_coding_agent_missing(self):
        """When placement has no coding_agent key, row defaults to 'claude'."""
        members = [
            {
                "agent_id": "agent-001",
                "name": "Legacy-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": {
                    "director_agent_id": "dir-001",
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "tmux_pane_id": "%7",
                    "created_at": "2026-04-12T10:15:00Z",
                },
            }
        ]
        result = format_member_list(members)
        lines = result.split("\n")
        data_lines = [l for l in lines if "Legacy-B" in l]
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
        claude_line = [l for l in lines if "Claude-M" in l][0]
        codex_line = [l for l in lines if "Codex-M" in l][0]
        assert "claude" in claude_line
        assert "codex" in codex_line

    def test_empty_list_unchanged(self):
        """Empty member list still returns '0 members.'."""
        result = format_member_list([])
        assert "0 members" in result
