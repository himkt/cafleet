"""Tests for ``_resolve_prompt`` in ``cafleet.cli``.

Design doc 0000024 task 2.2 — pin the contract that both the default
prompt template AND a user-supplied ``prompt_argv`` get the same
``{session_id}`` / ``{agent_id}`` / ``{director_name}`` /
``{director_agent_id}`` substitutions via ``str.format``.

Four cases:

  (a) default path (empty ``prompt_argv``) substitutes all UUIDs into
      the CLAUDE template — existing behaviour, must keep passing
  (b) custom prompt containing ``{agent_id}`` gets substituted — new
      behaviour pinned here, regression for R1 (task 2.1)
  (c) custom prompt with no placeholders passes through unchanged
  (d) custom prompt with doubled ``{{...}}`` braces collapses to single
      literal braces and does NOT attempt placeholder substitution on
      the inner tokens — risk-row mitigation for literal-brace JSON
      snippets in custom prompts

Cases (b) and (d) fail against pre-fix code because today's
``_resolve_prompt`` returns early for non-empty ``prompt_argv`` without
calling ``.format``. That failure is expected and drives the TDD loop.
"""

import uuid

import click
import pytest

from cafleet import broker
from cafleet.cli import _resolve_prompt
from cafleet.coding_agent import CLAUDE


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


# ===========================================================================
# (a) default path substitutes all template placeholders
# ===========================================================================


class TestDefaultPromptSubstitution:
    """Design doc 2.2(a): empty prompt_argv → CLAUDE template filled in."""

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
        assert session_id in result, (
            f"default prompt must contain session_id={session_id!r}. got: {result!r}"
        )
        assert new_agent_id in result, (
            f"default prompt must contain agent_id={new_agent_id!r}. got: {result!r}"
        )
        assert director_agent_id in result, (
            f"default prompt must contain director_agent_id={director_agent_id!r}. "
            f"got: {result!r}"
        )
        assert "Director-X" in result, (
            f"default prompt must contain director_name 'Director-X'. got: {result!r}"
        )
        # Template placeholders must not leak through unsubstituted.
        assert "{session_id}" not in result, (
            f"{{session_id}} placeholder must be substituted. got: {result!r}"
        )
        assert "{agent_id}" not in result, (
            f"{{agent_id}} placeholder must be substituted. got: {result!r}"
        )
        assert "{director_name}" not in result, (
            f"{{director_name}} placeholder must be substituted. got: {result!r}"
        )
        assert "{director_agent_id}" not in result, (
            f"{{director_agent_id}} placeholder must be substituted. got: {result!r}"
        )


# ===========================================================================
# (b) custom prompt with {agent_id} placeholder gets substituted
# ===========================================================================


class TestCustomPromptPlaceholderSubstitution:
    """Design doc 2.2(b): custom prompt uses the same format kwargs as default.

    Against pre-fix code this test FAILS because ``_resolve_prompt`` returns
    ``" ".join(prompt_argv)`` before reaching ``.format``. That is expected
    (TDD) — the Programmer's task 2.1 fix makes it pass.
    """

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
        assert result == f"message for {new_agent_id}", (
            f"custom prompt must substitute {{agent_id}} with {new_agent_id!r}. "
            f"got: {result!r}"
        )


# ===========================================================================
# (c) custom prompt with no placeholders passes through unchanged
# ===========================================================================


class TestCustomPromptNoPlaceholderPassThrough:
    """Design doc 2.2(c): .format on a literal string is a no-op."""

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
        assert result == "no placeholders here", (
            f"custom prompt with no placeholders must pass through unchanged. "
            f"got: {result!r}"
        )


# ===========================================================================
# (d) doubled-brace escape collapses to single literal braces
# ===========================================================================


class TestCustomPromptDoubledBraceEscape:
    """Design doc 2.2(d): ``{{...}}`` collapses to ``{...}`` without substitution.

    The risk-row mitigation for literal-brace JSON snippets: callers who
    need a literal ``{`` / ``}`` in a custom prompt must double them, and
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
        assert result == "data is {not a placeholder} closed", (
            f"doubled braces must collapse to single braces and NOT substitute "
            f"the inner tokens. got: {result!r}"
        )
        # Defensive: no UUID should leak in via accidental substitution.
        assert new_agent_id not in result, (
            f"doubled-brace escape must not substitute agent_id. got: {result!r}"
        )
        assert director_agent_id not in result, (
            f"doubled-brace escape must not substitute director_agent_id. "
            f"got: {result!r}"
        )


# ===========================================================================
# (e) malformed custom prompts surface as click.UsageError, not KeyError /
#     ValueError, so member_create's rollback path runs.
# ===========================================================================


class TestCustomPromptMalformedRaisesUsageError:
    """PR #25 review feedback: ``str.format`` failures must convert to
    ``click.UsageError`` so ``member_create``'s rollback path (which only
    catches ``UsageError``) reliably runs and the just-registered agent
    does not get orphaned in the registry.
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
        assert "foo" in message, (
            f"error message must name the unknown placeholder. got: {message!r}"
        )
        assert "{session_id}" in message and "{agent_id}" in message, (
            f"error message must list supported placeholders. got: {message!r}"
        )

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
        assert "{{" in message and "}}" in message, (
            f"error message must hint at brace doubling. got: {message!r}"
        )

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
        assert "{{" in message and "}}" in message, (
            f"error message must hint at brace doubling. got: {message!r}"
        )
