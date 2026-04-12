"""Tests for hikyaku.models — Pydantic model changes for multi-runner support.

Design doc 0000018 Step 5: PlacementCreate gains coding_agent with default
"claude"; PlacementView gains coding_agent field; PlacementPatch is unchanged.
"""

from hikyaku.models import PlacementCreate, PlacementPatch, PlacementView


# ---------------------------------------------------------------------------
# PlacementCreate
# ---------------------------------------------------------------------------


class TestPlacementCreate:
    def test_coding_agent_defaults_to_claude(self):
        """coding_agent defaults to 'claude' when not provided."""
        p = PlacementCreate(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
        )
        assert p.coding_agent == "claude"

    def test_coding_agent_accepts_codex(self):
        """coding_agent can be set to 'codex'."""
        p = PlacementCreate(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            coding_agent="codex",
        )
        assert p.coding_agent == "codex"

    def test_coding_agent_accepts_claude_explicitly(self):
        """coding_agent can be set explicitly to 'claude'."""
        p = PlacementCreate(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            coding_agent="claude",
        )
        assert p.coding_agent == "claude"

    def test_default_fields_unchanged(self):
        """Existing fields still work as before."""
        p = PlacementCreate(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id="%7",
        )
        assert p.director_agent_id == "dir-1"
        assert p.tmux_session == "main"
        assert p.tmux_window_id == "@3"
        assert p.tmux_pane_id == "%7"

    def test_tmux_pane_id_still_defaults_to_none(self):
        """tmux_pane_id still defaults to None (pending placement)."""
        p = PlacementCreate(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
        )
        assert p.tmux_pane_id is None


# ---------------------------------------------------------------------------
# PlacementView
# ---------------------------------------------------------------------------


class TestPlacementView:
    def test_includes_coding_agent(self):
        """PlacementView includes coding_agent field."""
        v = PlacementView(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id="%7",
            coding_agent="codex",
            created_at="2026-04-12T10:00:00Z",
        )
        assert v.coding_agent == "codex"

    def test_coding_agent_claude(self):
        """PlacementView accepts coding_agent='claude'."""
        v = PlacementView(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
            coding_agent="claude",
            created_at="2026-04-12T10:00:00Z",
        )
        assert v.coding_agent == "claude"

    def test_model_dump_includes_coding_agent(self):
        """model_dump() output includes coding_agent key."""
        v = PlacementView(
            director_agent_id="dir-1",
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id="%7",
            coding_agent="codex",
            created_at="2026-04-12T10:00:00Z",
        )
        d = v.model_dump()
        assert "coding_agent" in d
        assert d["coding_agent"] == "codex"


# ---------------------------------------------------------------------------
# PlacementPatch — unchanged
# ---------------------------------------------------------------------------


class TestPlacementPatch:
    def test_no_coding_agent_field(self):
        """PlacementPatch does NOT have a coding_agent field (patches tmux_pane_id only)."""
        p = PlacementPatch(tmux_pane_id="%7")
        assert not hasattr(p, "coding_agent") or "coding_agent" not in p.model_fields

    def test_still_has_tmux_pane_id(self):
        """PlacementPatch still only patches tmux_pane_id."""
        p = PlacementPatch(tmux_pane_id="%9")
        assert p.tmux_pane_id == "%9"
