"""Unit tests for ``cafleet.coding_agent.opencode_preset``.

Tests the dataclass-backed opencode agent preset: ``PermissionRuleset``,
``OpencodeAgentDefinition``, the ``CAFLEET_AGENT`` constant, and the
``materialize_cafleet_agent`` helper. Verifies the markdown rendering
contract (JSON-as-YAML frontmatter), the catch-all + deny ordering
discipline (last-match-wins per ``permission/evaluate.ts:9-15``), and the
skip-if-exists semantics of the materialization helper.
"""

import json
from dataclasses import FrozenInstanceError

import pytest

from cafleet.coding_agent.opencode_preset import (
    CAFLEET_AGENT,
    OpencodeAgentDefinition,
    PermissionRuleset,
    materialize_cafleet_agent,
)

# ---------------------------------------------------------------------------
# PermissionRuleset dataclass shape
# ---------------------------------------------------------------------------


def test_permission_ruleset_is_frozen():
    """PermissionRuleset is a frozen dataclass — immutable.

    Asserts the specific ``dataclasses.FrozenInstanceError`` rather than a
    broad ``(AttributeError, Exception)`` catch, so an unrelated AttributeError
    (e.g. typo in a field name) does NOT silently satisfy the test."""
    ruleset = PermissionRuleset()
    with pytest.raises(FrozenInstanceError):
        ruleset.webfetch = "allow"  # type: ignore[misc]


def test_permission_ruleset_default_action_shorthand_keys_are_deny():
    """Per §3.2: external_directory / webfetch / websearch / repo_clone /
    question / plan_enter / plan_exit all default to ``deny``."""
    ruleset = PermissionRuleset()
    assert ruleset.external_directory == "deny"
    assert ruleset.webfetch == "deny"
    assert ruleset.websearch == "deny"
    assert ruleset.repo_clone == "deny"
    assert ruleset.question == "deny"
    assert ruleset.plan_enter == "deny"
    assert ruleset.plan_exit == "deny"


def test_permission_ruleset_object_valued_keys_default_to_empty_dict():
    """bash / read / edit default to empty dicts — call sites supply the
    rules in insertion order to control rule precedence."""
    ruleset = PermissionRuleset()
    assert ruleset.bash == {}
    assert ruleset.read == {}
    assert ruleset.edit == {}


# ---------------------------------------------------------------------------
# OpencodeAgentDefinition.to_markdown() rendering
# ---------------------------------------------------------------------------


def _parse_rendered(markdown: str) -> tuple[dict, str]:
    """Split the rendered markdown on its ``---`` fence and JSON-decode
    the frontmatter. Returns ``(frontmatter_dict, body_str)``."""
    assert markdown.startswith("---\n"), "frontmatter must open with ---"
    rest = markdown[len("---\n") :]
    fence_idx = rest.index("\n---\n")
    fm_text = rest[:fence_idx]
    body = rest[fence_idx + len("\n---\n") :]
    return json.loads(fm_text), body


def test_to_markdown_starts_and_ends_with_frontmatter_fences():
    """The rendered output opens with ``---\\n`` and contains a closing
    ``\\n---\\n`` fence before the body."""
    rendered = CAFLEET_AGENT.to_markdown()
    assert rendered.startswith("---\n")
    assert "\n---\n\n" in rendered


def test_to_markdown_frontmatter_is_valid_json():
    """Per §3.1 ``to_markdown`` emits JSON inside the ``---`` block —
    JSON is a strict subset of YAML 1.2 so opencode's parser reads it."""
    rendered = CAFLEET_AGENT.to_markdown()
    frontmatter, _ = _parse_rendered(rendered)
    assert isinstance(frontmatter, dict)


def test_to_markdown_frontmatter_has_required_top_level_keys():
    """Frontmatter dict has ``description``, ``mode``, ``permission`` —
    these are the keys opencode's ``config/permission.ts:16-35`` recognizes."""
    frontmatter, _ = _parse_rendered(CAFLEET_AGENT.to_markdown())
    assert set(frontmatter.keys()) == {"description", "mode", "permission"}


def test_to_markdown_renders_mode_primary():
    """Per §3.2 the preset's ``mode`` is ``"primary"`` (general-purpose
    conversational agent with a permission ruleset)."""
    frontmatter, _ = _parse_rendered(CAFLEET_AGENT.to_markdown())
    assert frontmatter["mode"] == "primary"


def test_to_markdown_body_contains_cafleet_marker():
    """The body section follows the closing ``---`` fence and starts with
    the ``# CAFleet member agent`` header per §3.2."""
    _, body = _parse_rendered(CAFLEET_AGENT.to_markdown())
    assert body.lstrip().startswith("# CAFleet member agent")


def test_to_markdown_round_trips_through_json():
    """Round-trip the rendered frontmatter through ``json.loads`` to
    confirm parseability (integration test per Step 5: JSON is a valid
    YAML 1.2 subset so this stands in for the opencode parser)."""
    rendered = CAFLEET_AGENT.to_markdown()
    frontmatter, body = _parse_rendered(rendered)
    # Round-trip survives a re-encode.
    re_encoded = json.dumps(frontmatter)
    re_parsed = json.loads(re_encoded)
    assert re_parsed == frontmatter
    assert body.strip()


def test_to_markdown_ascii_safe_unicode_passes_through():
    """``json.dumps(..., ensure_ascii=False)`` preserves non-ASCII text
    so multi-byte characters in the description / body survive rendering."""
    definition = OpencodeAgentDefinition(
        description="ünïcödé description",
        mode="primary",
        permission=PermissionRuleset(),
        body="日本語 body content\n",
    )
    rendered = definition.to_markdown()
    assert "ünïcödé" in rendered
    assert "日本語" in rendered


def test_to_markdown_preserves_dict_insertion_order_for_bash_rules():
    """Critical per §3.3: the catch-all ``"*": "allow"`` MUST appear FIRST
    in the rendered output so that ``findLast`` selects specific deny
    patterns over the catch-all. Python dict order + ``json.dumps``
    preserves insertion order — assert directly on the rendered text."""
    rendered = CAFLEET_AGENT.to_markdown()
    bash_block_start = rendered.index('"bash"')
    bash_section = rendered[bash_block_start:]
    star_allow_idx = bash_section.index('"*": "allow"')
    # Pick a sentinel specific-deny pattern that MUST appear after the catch-all.
    git_push_idx = bash_section.index('"git push*"')
    assert star_allow_idx < git_push_idx, (
        "catch-all '*': 'allow' must be listed BEFORE specific denies"
    )


# ---------------------------------------------------------------------------
# CAFLEET_AGENT constant — structural invariants
# ---------------------------------------------------------------------------


def test_cafleet_agent_is_primary_mode():
    assert CAFLEET_AGENT.mode == "primary"


def test_cafleet_agent_description_mentions_dontask_posture():
    """The description should make the safety floor explicit so opencode
    users reading the agent file know what posture they get."""
    desc = CAFLEET_AGENT.description.lower()
    assert "cafleet" in desc
    assert "dontask" in desc or "claude code" in desc or "permission" in desc


def test_cafleet_agent_bash_catch_all_is_first_key():
    """Per §3.3 the catch-all ``"*": "allow"`` MUST be the FIRST key in the
    bash deny-list dict (insertion order = rule precedence for findLast)."""
    bash = CAFLEET_AGENT.permission.bash
    first_key = next(iter(bash))
    assert first_key == "*"
    assert bash["*"] == "allow"


def test_cafleet_agent_read_catch_all_is_first_key():
    read = CAFLEET_AGENT.permission.read
    first_key = next(iter(read))
    assert first_key == "*"
    assert read["*"] == "allow"


def test_cafleet_agent_edit_catch_all_is_first_key():
    edit = CAFLEET_AGENT.permission.edit
    first_key = next(iter(edit))
    assert first_key == "*"
    assert edit["*"] == "allow"


@pytest.mark.parametrize(
    "wrapper_pattern",
    [
        "bash -c*",
        "sh -c*",
        "zsh -c*",
        "python -c*",
        "python3 -c*",
        "perl -e*",
        "node -e*",
        "node --eval*",
        "ruby -e*",
        "eval*",
        "exec*",
        "osascript*",
    ],
)
def test_cafleet_agent_bash_denies_shell_indirection_wrapper(wrapper_pattern):
    """Per §3.4 every shell-indirection wrapper in §3.2 must be denied.
    Without these, ``bash -c '<smuggled>'`` slips through tree-sitter as a
    single node and the deny-list never sees the inner command."""
    assert CAFLEET_AGENT.permission.bash[wrapper_pattern] == "deny"


@pytest.mark.parametrize(
    "destructive_pattern",
    [
        "rm -rf*",
        "sudo*",
        "git push*",
        "git reset --hard*",
        "chmod*",
        "chown*",
    ],
)
def test_cafleet_agent_bash_denies_destructive_operations(destructive_pattern):
    """Destructive shell ops from §3.2 are explicit denies."""
    assert CAFLEET_AGENT.permission.bash[destructive_pattern] == "deny"


@pytest.mark.parametrize(
    "egress_pattern", ["curl*", "wget*", "nc*", "ssh*", "scp*", "rsync*"]
)
def test_cafleet_agent_bash_denies_network_egress(egress_pattern):
    """Network egress utilities from §3.2 are explicit denies."""
    assert CAFLEET_AGENT.permission.bash[egress_pattern] == "deny"


def test_cafleet_agent_read_denies_dotenv_files():
    """Per §3.2: ``**/.env`` and ``**/.env.*`` are explicit read denies
    so credentials in dotenv files cannot be exfiltrated."""
    assert CAFLEET_AGENT.permission.read["**/.env"] == "deny"
    assert CAFLEET_AGENT.permission.read["**/.env.*"] == "deny"


def test_cafleet_agent_edit_denies_dotenv_files():
    """Per §3.2: ``**/.env`` and ``**/.env.*`` are explicit edit denies."""
    assert CAFLEET_AGENT.permission.edit["**/.env"] == "deny"
    assert CAFLEET_AGENT.permission.edit["**/.env.*"] == "deny"


@pytest.mark.parametrize(
    "shorthand_attr",
    [
        "external_directory",
        "webfetch",
        "websearch",
        "repo_clone",
        "question",
        "plan_enter",
        "plan_exit",
    ],
)
def test_cafleet_agent_action_shorthand_keys_resolve_to_deny(shorthand_attr):
    """Per §3.2 + §3.4: every Action-shorthand key is set to ``deny`` so
    no permission check resolves to ``ask`` (no human in the TUI loop)."""
    assert getattr(CAFLEET_AGENT.permission, shorthand_attr) == "deny"


def test_cafleet_agent_body_mentions_cafleet_member_role():
    """The markdown body explains the agent's role to opencode at runtime."""
    body = CAFLEET_AGENT.body.lower()
    assert "cafleet" in body
    assert "director" in body or "member" in body


# ---------------------------------------------------------------------------
# materialize_cafleet_agent skip-if-exists semantics
# ---------------------------------------------------------------------------


def test_materialize_writes_file_when_target_does_not_exist(tmp_path, monkeypatch):
    """Per §4.2: when ``~/.opencode/agents/cafleet.md`` is missing,
    ``materialize_cafleet_agent`` writes the rendered markdown and creates
    parent directories. Verifies (a) of the Step-5 task."""
    monkeypatch.setenv("HOME", str(tmp_path))

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    assert not target.exists()
    assert not target.parent.exists()

    materialize_cafleet_agent(CAFLEET_AGENT)

    assert target.exists()
    assert target.read_text(encoding="utf-8") == CAFLEET_AGENT.to_markdown()


def test_materialize_creates_parent_directories(tmp_path, monkeypatch):
    """The ``agents/`` parent dir is created with parents=True so first-spawn
    materialization works even when ``~/.opencode/`` is absent."""
    monkeypatch.setenv("HOME", str(tmp_path))

    materialize_cafleet_agent(CAFLEET_AGENT)

    assert (tmp_path / ".opencode").is_dir()
    assert (tmp_path / ".opencode" / "agents").is_dir()


def test_materialize_is_no_op_when_target_exists(tmp_path, monkeypatch):
    """Per §4.2: skip-if-exists. Once the target exists with ANY content,
    materialize MUST NOT overwrite it. Verifies (b) of the Step-5 task —
    operator customizations survive upgrades."""
    monkeypatch.setenv("HOME", str(tmp_path))

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    target.parent.mkdir(parents=True)
    custom_content = "# operator's hand-edited version\nfoo: bar\n"
    target.write_text(custom_content, encoding="utf-8")

    materialize_cafleet_agent(CAFLEET_AGENT)

    assert target.read_text(encoding="utf-8") == custom_content


def test_materialize_preserves_empty_target_file(tmp_path, monkeypatch):
    """Edge case: an empty pre-existing file still triggers skip-if-exists.
    The presence of the path — not its content — is the signal."""
    monkeypatch.setenv("HOME", str(tmp_path))

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    target.parent.mkdir(parents=True)
    target.write_text("", encoding="utf-8")

    materialize_cafleet_agent(CAFLEET_AGENT)

    assert target.read_text(encoding="utf-8") == ""


def test_materialize_does_not_modify_mtime_on_existing_file(tmp_path, monkeypatch):
    """Skip-if-exists must not touch the file at all — including mtime.
    The Step-6 manual smoke test relies on ``stat`` mtime to verify the
    second spawn does not modify the preset; this is the unit-test analog."""
    monkeypatch.setenv("HOME", str(tmp_path))

    target = tmp_path / ".opencode" / "agents" / "cafleet.md"
    target.parent.mkdir(parents=True)
    target.write_text("existing\n", encoding="utf-8")
    mtime_before = target.stat().st_mtime_ns

    materialize_cafleet_agent(CAFLEET_AGENT)

    mtime_after = target.stat().st_mtime_ns
    assert mtime_before == mtime_after


def test_materialize_propagates_oserror_when_home_unwritable(tmp_path, monkeypatch):
    """Per §4.3: when the parent dir cannot be created (or write fails),
    ``materialize_cafleet_agent`` wraps the underlying ``OSError`` as a
    ``RuntimeError`` (chained via ``raise ... from exc``) so the spawn
    aborts cleanly with a single exception type for ``cli.member_create``
    to surface. The test simulates the failure by pointing HOME at a path
    that exists as a regular file (so ``mkdir(parents=True)`` fails)."""
    not_a_dir = tmp_path / "home-but-its-a-file"
    not_a_dir.write_text("not a directory", encoding="utf-8")
    monkeypatch.setenv("HOME", str(not_a_dir))

    with pytest.raises(RuntimeError) as excinfo:
        materialize_cafleet_agent(CAFLEET_AGENT)

    # The original OSError is chained via ``raise ... from exc`` so
    # operators / cli.member_create can inspect the underlying cause.
    assert isinstance(excinfo.value.__cause__, OSError)
