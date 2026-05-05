"""Surface 5 — configurable text truncation (design 0000049 Step 8).

Asserts the four Step 8 contracts (per the Director's clarification):

(a) ``Settings.max_text_len`` defaults to ``200`` and is overridable via the
    ``CAFLEET_MAX_TEXT_LEN`` environment variable.
(b) ``output.truncate_text`` reads ``settings.max_text_len`` when the caller
    passes no explicit ``limit``; an explicit ``limit`` still wins.
(c) The truncation suffix is the single-codepoint ``"…"`` (U+2026), not the
    legacy three-character ``"..."`` form.
(d) ``agent.description`` truncates at 60 codepoints in ``--full`` mode (the
    compact 1-line ``format_agent`` already drops description entirely, so
    truncation only matters in the verbose layout).

The metadata-string truncation at limit 80 is scope-reduced because there
is no clean target post-Surface-14 — the typed-column shape no longer has
free-form ``metadata`` strings to truncate. Flagged in the report rather
than tested.

Tests use ``monkeypatch.setenv`` + a fresh ``Settings()`` instance to
exercise the env-var-driven default without polluting the process-wide
``cafleet.config.settings`` singleton.
"""

import importlib

import pytest

from cafleet import output
from cafleet.config import Settings


# ---------------------------------------------------------------------------
# (a) Settings.max_text_len env-var-driven default is 200
# ---------------------------------------------------------------------------


def test_settings__max_text_len_default_is_200(monkeypatch):
    monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    s = Settings()
    assert s.max_text_len == 200


def test_settings__max_text_len_overridable_via_env_var(monkeypatch):
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "350")
    s = Settings()
    assert s.max_text_len == 350


def test_settings__max_text_len_env_var_must_be_integer(monkeypatch):
    """Pydantic-Settings should reject a non-integer env value (so callers
    fail loudly rather than silently degrading)."""
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "not-a-number")
    with pytest.raises(Exception):
        Settings()


def test_settings__max_text_len_field_uses_validation_alias(monkeypatch):
    """Other aliases (e.g. just ``max_text_len`` without the ``CAFLEET_``
    prefix) must NOT be picked up — the project's env-var contract uses
    a uniform ``CAFLEET_*`` prefix."""
    monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    monkeypatch.setenv("MAX_TEXT_LEN", "999")
    s = Settings()
    # Bare ``MAX_TEXT_LEN`` is not the alias — the default 200 still wins.
    assert s.max_text_len == 200


# ---------------------------------------------------------------------------
# (b) truncate_text default limit reads from settings.max_text_len
# ---------------------------------------------------------------------------


def test_truncate_text__no_limit_reads_settings_max_text_len(monkeypatch):
    """A 250-char body is truncated to 200 codepoints + suffix when no
    explicit ``limit`` is supplied."""
    # Ensure default settings is loaded with default 200.
    monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    body = "a" * 250
    result = output.truncate_text(body, full=False)
    assert result is not None
    # 200 chars of body + 1-char ellipsis = 201 codepoints.
    assert len(result) == 201
    assert result.startswith("a" * 200)


def test_truncate_text__no_limit_under_settings_default_passes_through(monkeypatch):
    monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    body = "a" * 150
    result = output.truncate_text(body, full=False)
    assert result == body


def test_truncate_text__env_var_override_changes_default_limit(monkeypatch):
    """When ``CAFLEET_MAX_TEXT_LEN=50`` is set, ``truncate_text`` (no
    explicit limit) truncates at 50 codepoints."""
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "50")
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    body = "a" * 100
    result = output.truncate_text(body, full=False)
    assert result is not None
    # 50 chars + 1-char ellipsis.
    assert len(result) == 51
    assert result.startswith("a" * 50)


def test_truncate_text__explicit_limit_overrides_settings(monkeypatch):
    """An explicit ``limit`` argument wins over the env-driven default."""
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "200")
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert len(result) == 11
    assert result.startswith("a" * 10)


def test_truncate_text__full_true_bypasses_settings_limit(monkeypatch):
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "10")
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    body = "a" * 500
    assert output.truncate_text(body, full=True) == body


# ---------------------------------------------------------------------------
# (c) Suffix is "…" (single codepoint)
# ---------------------------------------------------------------------------


def test_truncate_text__suffix_is_single_codepoint_ellipsis():
    """Surface 5 replaces the 3-char ``"..."`` with the 1-char ``"…"``."""
    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert result.endswith("…"), f"suffix should be '…' (U+2026); got: {result!r}"


def test_truncate_text__suffix_is_exactly_one_codepoint():
    """The suffix must be exactly one Python ``str`` codepoint long — proof
    that the new ``"…"`` form is in use."""
    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    suffix = result[10:]
    assert len(suffix) == 1, (
        f"truncation suffix must be exactly 1 codepoint; got {suffix!r} "
        f"(len={len(suffix)})"
    )


def test_truncate_text__no_legacy_3_dot_suffix():
    """Defensive guard against the legacy ``"..."`` (three full stops) form."""
    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert not result.endswith("...")


def test_truncate_text__suffix_is_u_2026_horizontal_ellipsis():
    """Pin the exact codepoint: U+2026 HORIZONTAL ELLIPSIS."""
    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert result[-1] == "…"


def test_truncate_text__multibyte_body_with_ellipsis_is_well_formed():
    """A multibyte body truncated by codepoint joined with the single-codepoint
    ellipsis still produces a well-formed string of length limit + 1."""
    body = "あいうえおかきくけこさしすせそ"  # 15 codepoints
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert len(result) == 11  # 10 codepoints + "…"
    assert result == "あいうえおかきくけこ…"


# ---------------------------------------------------------------------------
# (d) agent.description truncation at 60 codepoints in --full mode
# ---------------------------------------------------------------------------


def test_format_agent_full__truncates_description_at_60_codepoints():
    """In ``--full`` mode the agent layout includes a ``description:`` line
    whose value is truncated to 60 codepoints + ``"…"``."""
    long_desc = "x" * 200
    agent = {
        "agent_id": "abcdef0123456789-tail",
        "name": "Claude-B",
        "description": long_desc,
        "status": "active",
    }
    rendered = output.format_agent(agent, full=True)
    # The description field must NOT contain the full 200-character body.
    assert long_desc not in rendered, (
        f"description should be truncated in --full mode; got:\n{rendered}"
    )
    # The truncation suffix must appear after exactly 60 codepoints of body.
    assert "x" * 60 + "…" in rendered, (
        f"description should be truncated to 60 codepoints + '…'; got:\n{rendered}"
    )


def test_format_agent_full__short_description_passes_through():
    """A description ≤ 60 codepoints must pass through unchanged."""
    short_desc = "y" * 30
    agent = {
        "agent_id": "abcdef0123456789-tail",
        "name": "Claude-B",
        "description": short_desc,
        "status": "active",
    }
    rendered = output.format_agent(agent, full=True)
    assert short_desc in rendered
    # No ellipsis since nothing was truncated.
    assert "…" not in rendered


def test_format_agent_full__exactly_60_char_description_passes_through():
    desc_60 = "z" * 60
    agent = {
        "agent_id": "abcdef0123456789-tail",
        "name": "Claude-B",
        "description": desc_60,
        "status": "active",
    }
    rendered = output.format_agent(agent, full=True)
    assert desc_60 in rendered
    assert "…" not in rendered


def test_format_agent_full__61_char_description_is_truncated_to_60_plus_ellipsis():
    desc_61 = "w" * 61
    agent = {
        "agent_id": "abcdef0123456789-tail",
        "name": "Claude-B",
        "description": desc_61,
        "status": "active",
    }
    rendered = output.format_agent(agent, full=True)
    assert "w" * 60 + "…" in rendered
    # The full 61-char form must NOT survive verbatim.
    assert desc_61 not in rendered


# ---------------------------------------------------------------------------
# truncate_task_text: --full bypass remains, default uses settings
# ---------------------------------------------------------------------------


def test_truncate_task_text__full_true_bypasses_settings(monkeypatch):
    """``truncate_task_text(..., full=True)`` returns the input unchanged
    regardless of settings.max_text_len."""
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "10")
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    task = {
        "task_id": "tid",
        "context_id": "ctx",
        "from_agent_id": "fid",
        "to_agent_id": "tid",
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "a" * 500,
    }
    output.truncate_task_text(task, full=True)
    assert task["text"] == "a" * 500


def test_truncate_task_text__default_limit_truncates_to_settings_max(monkeypatch):
    """Without an explicit ``limit``, ``truncate_task_text`` honours the
    env-driven default and applies the new ``"…"`` suffix."""
    monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", "20")
    from cafleet import config

    importlib.reload(config)
    importlib.reload(output)

    task = {
        "task_id": "tid",
        "context_id": "ctx",
        "from_agent_id": "fid",
        "to_agent_id": "tid",
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "a" * 100,
    }
    output.truncate_task_text(task, full=False)
    assert task["text"] == "a" * 20 + "…"
