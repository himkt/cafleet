"""Tests for ``_resolve_prompt`` and ``cafleet member create`` (design doc 0000024)."""

import json
import uuid

import click
import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import _resolve_prompt, cli
from cafleet.coding_agent import CLAUDE
from cafleet.db import engine as engine_mod
from cafleet.tmux import DirectorContext


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def director_agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def new_agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def ctx(session_id):
    """Minimal ``click.Context`` with ``ctx.obj['session_id']`` populated."""
    command = click.Command("member-create")
    context = click.Context(command)
    context.obj = {"session_id": session_id, "json_output": False}
    return context


@pytest.fixture
def mock_get_agent(monkeypatch):
    """Make ``broker.get_agent`` return a fake director unconditionally.

    Covers both the pre-fix default-only lookup AND the post-fix custom-path
    lookup (task 2.1 adds the same director lookup to the custom branch so
    ``director_name`` and ``director_agent_id`` are available as kwargs for
    every ``.format`` call).
    """

    def fake_get_agent(agent_id, session_id):
        return {"agent_id": agent_id, "name": "Director-X"}

    monkeypatch.setattr(broker, "get_agent", fake_get_agent)
    return fake_get_agent


class TestDefaultPromptSubstitution:
    def test_default_path_substitutes_all_placeholders(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
        session_id,
    ):
        result = _resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=(),
            coding_agent_config=CLAUDE,
        )
        assert session_id in result
        assert new_agent_id in result
        assert director_agent_id in result
        assert "Director-X" in result
        assert "{session_id}" not in result
        assert "{agent_id}" not in result
        assert "{director_name}" not in result
        assert "{director_agent_id}" not in result


class TestCustomPromptPlaceholderSubstitution:
    def test_custom_prompt_with_agent_id_placeholder_substitutes(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        result = _resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=("message", "for", "{agent_id}"),
            coding_agent_config=CLAUDE,
        )
        assert result == f"message for {new_agent_id}"


class TestCustomPromptNoPlaceholderPassThrough:
    def test_custom_prompt_without_placeholders_unchanged(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        result = _resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=("no", "placeholders", "here"),
            coding_agent_config=CLAUDE,
        )
        assert result == "no placeholders here"


class TestCustomPromptDoubledBraceEscape:
    """Design doc 2.2(d): ``{{...}}`` collapses to ``{...}`` without substitution.

    Callers embedding literal JSON snippets must double their braces, and
    ``.format`` then collapses each pair to a single literal brace.
    No placeholder substitution is attempted on the inner tokens.

    Against pre-fix code this test FAILS because the custom path never
    calls ``.format`` and returns the raw doubled-brace string.
    """

    def test_custom_prompt_with_doubled_braces_collapses_to_single(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        result = _resolve_prompt(
            ctx,
            director_agent_id=director_agent_id,
            new_agent_id=new_agent_id,
            prompt_argv=("data", "is", "{{not", "a", "placeholder}}", "closed"),
            coding_agent_config=CLAUDE,
        )
        assert result == "data is {not a placeholder} closed"
        assert new_agent_id not in result
        assert director_agent_id not in result


class TestCustomPromptMalformedRaisesUsageError:
    """``str.format`` errors must convert to ``click.UsageError``.

    ``member_create``'s rollback path only catches ``UsageError``, so a raw
    ``KeyError`` / ``ValueError`` would orphan the just-registered agent.
    """

    def test_unknown_placeholder_raises_usage_error(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        with pytest.raises(click.UsageError) as exc_info:
            _resolve_prompt(
                ctx,
                director_agent_id=director_agent_id,
                new_agent_id=new_agent_id,
                prompt_argv=("hello", "{foo}"),
                coding_agent_config=CLAUDE,
            )
        message = str(exc_info.value)
        assert "foo" in message
        assert "{session_id}" in message
        assert "{agent_id}" in message

    def test_unmatched_brace_raises_usage_error(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        with pytest.raises(click.UsageError) as exc_info:
            _resolve_prompt(
                ctx,
                director_agent_id=director_agent_id,
                new_agent_id=new_agent_id,
                prompt_argv=("hello", "{unclosed"),
                coding_agent_config=CLAUDE,
            )
        message = str(exc_info.value)
        assert "{{" in message
        assert "}}" in message

    def test_attribute_access_raises_usage_error(
        self,
        ctx,
        director_agent_id,
        new_agent_id,
        mock_get_agent,
    ):
        # PR #25 3rd review: ``{agent_id.foo}`` triggers str.format attribute
        # access on the substituted string; ``str`` has no ``.foo`` so Python
        # raises ``AttributeError``. Must be caught and converted to
        # ``UsageError`` so the rollback path in ``member_create`` still runs.
        with pytest.raises(click.UsageError) as exc_info:
            _resolve_prompt(
                ctx,
                director_agent_id=director_agent_id,
                new_agent_id=new_agent_id,
                prompt_argv=("hello", "{agent_id.foo}"),
                coding_agent_config=CLAUDE,
            )
        message = str(exc_info.value)
        assert "{{" in message
        assert "}}" in message


_CLI_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def _reset_engine():
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None
    yield
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None


@pytest.fixture
def bootstrapped_session(tmp_path, monkeypatch, _reset_engine):
    """Create a real session and return ``(session_id, director_agent_id, runner)``.

    Spins up a fresh SQLite DB, runs ``cafleet db init`` + ``cafleet session
    create --json``, and returns the three values every ``member create``
    invocation needs. ``tmux.ensure_tmux_available`` / ``director_context``
    are stubbed so ``session create`` (which demands a tmux context) succeeds.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _CLI_FAKE_DIRECTOR_CTX)

    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    create = runner.invoke(cli, ["session", "create", "--json"])
    assert create.exit_code == 0, create.output
    data = json.loads(create.output)
    return data["session_id"], data["director"]["agent_id"], runner


@pytest.fixture
def split_window_recorder(monkeypatch):
    """Monkeypatch ``tmux.split_window`` to capture its kwargs and return a pane."""
    calls: list[dict] = []

    def fake_split_window(**kwargs):
        calls.append(kwargs)
        return "%42"

    monkeypatch.setattr("cafleet.tmux.split_window", fake_split_window)
    # ``select_layout`` / ``send_exit`` are best-effort; stub them too so
    # nothing reaches a real tmux.
    monkeypatch.setattr("cafleet.tmux.select_layout", lambda **_: None)
    monkeypatch.setattr("cafleet.tmux.send_exit", lambda **_: None, raising=False)
    return calls


@pytest.fixture
def stub_coding_agent_binaries(monkeypatch):
    """Pretend every coding-agent binary is on PATH.

    ``coding_agent_config.ensure_available()`` calls ``shutil.which(self.binary)``;
    patching it module-wide is the narrowest monkeypatch that keeps both
    ``claude`` and ``codex`` spawns alive without a real binary.
    """
    monkeypatch.setattr("cafleet.coding_agent.shutil.which", lambda _: "/usr/bin/stub")


class TestMemberCreatePassesDisplayName:
    """Design doc 0000029 Step 4(f),(g): ``cli.py`` threads ``--name`` as
    ``display_name`` into ``coding_agent_config.build_command()``, which
    means the ``command`` kwarg handed to ``tmux.split_window`` contains
    ``"--name"`` + the member name for ``claude`` spawns and does NOT
    contain ``"--name"`` for ``codex`` spawns.
    """

    def test_member_create_passes_member_name_as_display_name(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        session_id, director_id, runner = bootstrapped_session
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "claude",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 0, result.output
        assert len(split_window_recorder) == 1
        command = split_window_recorder[0]["command"]
        assert isinstance(command, list)
        assert "--name" in command
        name_index = command.index("--name")
        assert command[name_index + 1] == "Drafter"
        assert command[name_index + 2] == "hello"
        assert command[0] == "claude"

    def test_member_create_codex_does_not_pass_name_flag(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        """Codex regression: ``codex`` has no ``--name`` equivalent today, so
        ``display_name_args=()`` must elide the flag even when ``--name``
        is supplied to ``cafleet member create`` (it is always required
        at the CLI level — ``click.Option --name required=True``).
        """
        session_id, director_id, runner = bootstrapped_session
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "codex",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 0, result.output
        assert len(split_window_recorder) == 1
        command = split_window_recorder[0]["command"]
        assert "--name" not in command
        assert command[0] == "codex"
        assert "--approval-mode" in command
        assert "auto-edit" in command


class TestNoBashFlag:
    """Step 4 task 1+2 (round-5c-era): ``--no-bash`` / ``--allow-bash`` flag pair.

    Per-coding-agent default resolution:
    - claude unset → ``--no-bash`` (deny_bash=True), argv gains
      ``--disallowedTools Bash``.
    - claude ``--allow-bash`` → deny_bash=False, argv stays clean.
    - codex ``--no-bash`` → CLI rejects with verbatim error, exits 1, no
      broker rows are created.
    - codex unset → ``--allow-bash`` (deny_bash=False), argv stays clean
      (codex's empty ``disallow_tools_args`` makes deny_bash a no-op anyway).

    Step 14 task 5 (round 6) prunes the codex sub-cases. Until then the four
    sub-cases must coexist.
    """

    def test_claude_default_appends_disallowed_tools_bash(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        session_id, director_id, runner = bootstrapped_session
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "claude",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 0, result.output
        assert len(split_window_recorder) == 1
        command = split_window_recorder[0]["command"]
        assert "--disallowedTools" in command
        deny_index = command.index("--disallowedTools")
        assert command[deny_index + 1] == "Bash"
        assert command[0] == "claude"
        # Pinned argv ordering: deny tokens before name args.
        name_index = command.index("--name")
        assert deny_index < name_index, (
            f"--disallowedTools must precede --name; got {command!r}"
        )

    def test_claude_explicit_allow_bash_omits_disallowed_tools(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        session_id, director_id, runner = bootstrapped_session
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "claude",
                "--allow-bash",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 0, result.output
        assert len(split_window_recorder) == 1
        command = split_window_recorder[0]["command"]
        assert "--disallowedTools" not in command
        assert "Bash" not in command
        assert command[0] == "claude"

    def test_codex_no_bash_rejected_with_verbatim_error_no_broker_rows(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        session_id, director_id, runner = bootstrapped_session
        before = broker.list_agents(session_id)
        before_count = len(before)
        # Snapshot only — the bootstrap fixture creates the Director plus an
        # Administrator agent; the rejection must not grow that count.

        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "codex",
                "--no-bash",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 1, result.output
        # Verbatim error per design doc §6 round-5c text.
        assert (
            "--no-bash with --coding-agent codex is not supported" in result.output
        )
        assert "Codex has no --disallowedTools-equivalent flag" in result.output
        assert "Use --allow-bash, or pick claude" in result.output
        # No broker rows created — rejection happens before register_agent.
        after = broker.list_agents(session_id)
        assert len(after) == before_count, (
            "Rejection must short-circuit before register_agent runs; "
            f"agent count grew from {before_count} to {len(after)}"
        )
        # No tmux pane spawned either.
        assert split_window_recorder == [], (
            "Rejection must short-circuit before tmux.split_window runs"
        )

    def test_codex_default_omits_disallowed_tools(
        self,
        bootstrapped_session,
        split_window_recorder,
        stub_coding_agent_binaries,
    ):
        session_id, director_id, runner = bootstrapped_session
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "member",
                "create",
                "--agent-id",
                director_id,
                "--name",
                "Drafter",
                "--description",
                "Drafter for PR #42",
                "--coding-agent",
                "codex",
                "--",
                "hello",
            ],
        )
        assert result.exit_code == 0, result.output
        assert len(split_window_recorder) == 1
        command = split_window_recorder[0]["command"]
        assert "--disallowedTools" not in command
        assert command[0] == "codex"
