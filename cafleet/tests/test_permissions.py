"""Unit tests for ``cafleet.permissions`` (design doc 0000037)."""

import json
from pathlib import Path

import pytest

from cafleet import permissions

# ---------------------------------------------------------------------------
# discover_settings_paths
# ---------------------------------------------------------------------------


class TestDiscoverSettingsPaths:
    def test_returns_three_paths_in_matcher_precedence_order(
        self, tmp_path, monkeypatch
    ):
        monkeypatch.chdir(tmp_path)
        monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)

        paths = permissions.discover_settings_paths()

        assert len(paths) == 3
        assert paths[0] == tmp_path / ".claude" / "settings.local.json"
        assert paths[1] == tmp_path / ".claude" / "settings.json"
        assert paths[2] == Path("~/.claude/settings.json").expanduser()

    def test_paths_are_path_objects(self, tmp_path, monkeypatch):
        monkeypatch.chdir(tmp_path)
        monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)

        paths = permissions.discover_settings_paths()

        for p in paths:
            assert isinstance(p, Path)

    def test_claude_config_dir_set_overrides_user_path(self, tmp_path, monkeypatch):
        custom = tmp_path / "custom-claude-dir"
        monkeypatch.chdir(tmp_path)
        monkeypatch.setenv("CLAUDE_CONFIG_DIR", str(custom))

        paths = permissions.discover_settings_paths()

        assert paths[2] == custom / "settings.json"

    def test_claude_config_dir_unset_falls_back_to_home(self, tmp_path, monkeypatch):
        monkeypatch.chdir(tmp_path)
        monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)

        paths = permissions.discover_settings_paths()

        # ``~`` must be expanded — never a literal ``~`` segment
        assert "~" not in str(paths[2])
        assert paths[2] == Path("~/.claude/settings.json").expanduser()

    def test_paths_returned_even_when_files_missing(self, tmp_path, monkeypatch):
        monkeypatch.chdir(tmp_path)
        monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)

        paths = permissions.discover_settings_paths()

        assert not paths[0].exists()
        assert not paths[1].exists()

    def test_cwd_resolved_at_call_time(self, tmp_path, monkeypatch):
        """No caching: re-invoking after chdir reflects the new CWD."""
        monkeypatch.chdir(tmp_path)
        monkeypatch.delenv("CLAUDE_CONFIG_DIR", raising=False)

        paths_a = permissions.discover_settings_paths()

        other = tmp_path / "other"
        other.mkdir()
        monkeypatch.chdir(other)

        paths_b = permissions.discover_settings_paths()

        assert paths_a[0] != paths_b[0]
        assert paths_b[0] == other / ".claude" / "settings.local.json"


# ---------------------------------------------------------------------------
# load_bash_patterns
# ---------------------------------------------------------------------------


def _write_settings(path: Path, *, allow=None, deny=None, ask=None, extra=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    perms: dict = {}
    if allow is not None:
        perms["allow"] = allow
    if deny is not None:
        perms["deny"] = deny
    if ask is not None:
        perms["ask"] = ask
    doc: dict = {}
    if perms:
        doc["permissions"] = perms
    if extra:
        doc.update(extra)
    path.write_text(json.dumps(doc))


class TestLoadBashPatterns:
    def test_missing_file_treated_as_empty(self, tmp_path):
        missing = tmp_path / "settings.json"
        # File is not written

        allow, deny = permissions.load_bash_patterns([missing])

        assert allow == []
        assert deny == []

    def test_valid_json_without_permissions_key_treated_as_empty(self, tmp_path):
        path = tmp_path / "settings.json"
        path.write_text(json.dumps({"theme": "dark"}))

        allow, deny = permissions.load_bash_patterns([path])

        assert allow == []
        assert deny == []

    def test_basic_allow_and_deny_parsed(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(
            path,
            allow=["Bash(git status)"],
            deny=["Bash(rm -rf *)"],
        )

        allow, deny = permissions.load_bash_patterns([path])

        assert len(allow) == 1
        assert allow[0].raw == "Bash(git status)"
        assert allow[0].body == "git status"
        assert allow[0].source_file == path

        assert len(deny) == 1
        assert deny[0].raw == "Bash(rm -rf *)"
        assert deny[0].body == "rm -rf *"
        assert deny[0].source_file == path

    def test_malformed_json_raises_with_path(self, tmp_path):
        path = tmp_path / "settings.json"
        path.write_text("{ not valid json")

        with pytest.raises(Exception, match="failed to parse") as exc_info:
            permissions.load_bash_patterns([path])

        assert str(path) in str(exc_info.value)

    def test_non_bash_entries_silently_filtered(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(
            path,
            allow=[
                "Bash(git status)",
                "Read(src/**)",
                "WebFetch(https://example.com)",
                "bare-string",
            ],
            deny=[
                "Bash(rm -rf *)",
                "Read(/etc/secrets)",
            ],
        )

        allow, deny = permissions.load_bash_patterns([path])

        assert [p.body for p in allow] == ["git status"]
        assert [p.body for p in deny] == ["rm -rf *"]

    def test_permissions_ask_ignored_entirely(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(path, ask=["Bash(git push:*)"], allow=[], deny=[])

        allow, deny = permissions.load_bash_patterns([path])

        assert allow == []
        assert deny == []

    def test_allow_lists_unioned_across_layers(self, tmp_path):
        a = tmp_path / "a.json"
        b = tmp_path / "b.json"
        c = tmp_path / "c.json"
        _write_settings(a, allow=["Bash(a)"])
        _write_settings(b, allow=["Bash(b)"])
        _write_settings(c, allow=["Bash(c)"])

        allow, deny = permissions.load_bash_patterns([a, b, c])

        bodies = sorted(p.body for p in allow)
        assert bodies == ["a", "b", "c"]
        assert deny == []

    def test_deny_lists_unioned_across_layers(self, tmp_path):
        a = tmp_path / "a.json"
        b = tmp_path / "b.json"
        _write_settings(a, deny=["Bash(rm)"])
        _write_settings(b, deny=["Bash(dd)"])

        allow, deny = permissions.load_bash_patterns([a, b])

        bodies = sorted(p.body for p in deny)
        assert bodies == ["dd", "rm"]
        assert allow == []

    def test_pattern_source_file_tracks_original_layer(self, tmp_path):
        a = tmp_path / "a.json"
        b = tmp_path / "b.json"
        _write_settings(a, allow=["Bash(from-a)"])
        _write_settings(b, allow=["Bash(from-b)"])

        allow, _ = permissions.load_bash_patterns([a, b])

        by_body = {p.body: p.source_file for p in allow}
        assert by_body["from-a"] == a
        assert by_body["from-b"] == b


# ---------------------------------------------------------------------------
# Glob matcher
# ---------------------------------------------------------------------------


def _pat(body: str, source_file: Path | None = None) -> permissions.Pattern:
    return permissions.Pattern(
        raw=f"Bash({body})",
        body=body,
        source_file=source_file or Path("/tmp/x.json"),
    )


class TestGlobMatcher:
    def test_star_matches_anything(self):
        pat = _pat("*")

        assert permissions.match(pat, "git status")
        assert permissions.match(pat, "anything goes here")
        assert permissions.match(pat, "")

    def test_exact_match_no_wildcard(self):
        pat = _pat("git status")

        assert permissions.match(pat, "git status")
        assert not permissions.match(pat, "git status --short")
        assert not permissions.match(pat, "gitstatus")

    def test_word_boundary_colon_star(self):
        pat = _pat("git status:*")

        assert permissions.match(pat, "git status")
        assert permissions.match(pat, "git status --short")
        assert not permissions.match(pat, "gitstatus")

    def test_word_boundary_space_star(self):
        pat = _pat("git status *")

        assert permissions.match(pat, "git status")
        assert permissions.match(pat, "git status --short")
        assert not permissions.match(pat, "gitstatus")

    def test_no_boundary_trailing_star(self):
        pat = _pat("gitstatus*")

        assert permissions.match(pat, "gitstatus")
        assert permissions.match(pat, "gitstatus123")
        assert permissions.match(pat, "gitstatus --short")
        assert not permissions.match(pat, "git status")

    def test_interior_wildcard_no_trailing_star(self):
        """``Bash(* install)`` matches `npm install` but NOT `npm install --save-dev`."""
        pat = _pat("* install")

        assert permissions.match(pat, "npm install")
        assert permissions.match(pat, "yarn install")
        assert not permissions.match(pat, "npm install --save-dev")
        assert not permissions.match(pat, "install")

    def test_multi_position_wildcard_anchored(self):
        """``Bash(git * main)`` is fullmatch-anchored: trailing args do NOT match."""
        pat = _pat("git * main")

        assert permissions.match(pat, "git checkout main")
        assert permissions.match(pat, "git push origin main")
        assert not permissions.match(pat, "git status")
        assert not permissions.match(pat, "git checkout main --force")


# ---------------------------------------------------------------------------
# decide
# ---------------------------------------------------------------------------


class TestDecide:
    def test_allow_when_only_allow_matches(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(path, allow=["Bash(git status:*)"])

        decision = permissions.decide("git status --short", [path])

        assert decision.outcome == "allow"
        assert decision.matched_pattern == "Bash(git status:*)"
        assert decision.matched_file == path
        assert decision.offending_substring == "git status --short"
        assert decision.searched_files == [path]

    def test_deny_when_only_deny_matches(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(path, deny=["Bash(rm -rf *)"])

        decision = permissions.decide("rm -rf /tmp", [path])

        assert decision.outcome == "deny"
        assert decision.matched_pattern == "Bash(rm -rf *)"
        assert decision.matched_file == path
        assert decision.offending_substring == "rm -rf /tmp"
        assert decision.searched_files == [path]

    def test_ask_when_no_pattern_matches(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(path, allow=["Bash(npm test:*)"])

        decision = permissions.decide("git status", [path])

        assert decision.outcome == "ask"
        assert decision.matched_pattern is None
        assert decision.matched_file is None
        assert decision.offending_substring is None
        assert decision.searched_files == [path]

    def test_deny_wins_over_allow_on_conflict(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(
            path,
            allow=["Bash(git push:*)"],
            deny=["Bash(git push:*)"],
        )

        decision = permissions.decide("git push origin main", [path])

        assert decision.outcome == "deny"
        assert decision.matched_pattern == "Bash(git push:*)"
        assert decision.matched_file == path

    def test_deny_in_user_layer_blocks_allow_in_project_layer(self, tmp_path):
        project = tmp_path / "project.json"
        user = tmp_path / "user.json"
        _write_settings(project, allow=["Bash(git push:*)"])
        _write_settings(user, deny=["Bash(git push:*)"])

        decision = permissions.decide("git push origin main", [project, user])

        assert decision.outcome == "deny"
        assert decision.matched_file == user

    def test_searched_files_always_populated_with_resolved_paths(self, tmp_path):
        a = tmp_path / "a.json"
        b = tmp_path / "b.json"
        c = tmp_path / "c.json"
        _write_settings(a, allow=["Bash(npm test:*)"])
        # b and c left missing on disk

        decision = permissions.decide("git status", [a, b, c])

        assert decision.outcome == "ask"
        assert decision.searched_files == [a, b, c]

    def test_no_caching_reload_after_disk_mutation(self, tmp_path):
        path = tmp_path / "settings.json"
        _write_settings(path, allow=["Bash(git status:*)"])

        first = permissions.decide("git status", [path])
        assert first.outcome == "allow"

        # Mutate file on disk
        _write_settings(path, allow=["Bash(npm test:*)"])

        second = permissions.decide("git status", [path])
        assert second.outcome == "ask"
