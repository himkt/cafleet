"""Configurable text truncation."""

import pytest

from cafleet import output
from cafleet.config import Settings


@pytest.mark.parametrize(
    ("scenario", "env_value", "expected_value_or_raises"),
    [
        ("default_is_200", None, 200),
        ("override_to_350", "350", 350),
        ("invalid_non_integer_raises", "not-a-number", "raises"),
    ],
)
def test_settings__max_text_len_env_handling(
    monkeypatch, scenario, env_value, expected_value_or_raises
):
    if env_value is None:
        monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    else:
        monkeypatch.setenv("CAFLEET_MAX_TEXT_LEN", env_value)
    if expected_value_or_raises == "raises":
        with pytest.raises(ValueError, match="CAFLEET_MAX_TEXT_LEN"):
            Settings()
    else:
        s = Settings()
        assert s.max_text_len == expected_value_or_raises

    # Non-prefixed alias is NOT picked up.
    monkeypatch.delenv("CAFLEET_MAX_TEXT_LEN", raising=False)
    monkeypatch.setenv("MAX_TEXT_LEN", "999")
    assert Settings().max_text_len == 200


@pytest.mark.parametrize(
    (
        "scenario",
        "settings_max",
        "body_len",
        "explicit_limit",
        "full",
        "expected_len",
        "starts_with",
    ),
    [
        (
            "default_settings_200_truncates_250_body",
            200,
            250,
            None,
            False,
            201,
            "a" * 200,
        ),
        ("default_settings_200_passes_150_body", 200, 150, None, False, 150, "a" * 150),
        ("settings_50_truncates_100_body", 50, 100, None, False, 51, "a" * 50),
        ("explicit_limit_overrides_settings", 200, 100, 10, False, 11, "a" * 10),
        ("full_true_bypasses_settings", 10, 500, None, True, 500, "a" * 500),
    ],
)
def test_truncate_text__settings_default_and_explicit_limit(
    monkeypatch,
    scenario,
    settings_max,
    body_len,
    explicit_limit,
    full,
    expected_len,
    starts_with,
):
    monkeypatch.setattr("cafleet.config.settings.max_text_len", settings_max)
    body = "a" * body_len
    kwargs = {"full": full}
    if explicit_limit is not None:
        kwargs["limit"] = explicit_limit
    result = output.truncate_text(body, **kwargs)
    assert result is not None
    assert len(result) == expected_len
    assert result.startswith(starts_with)


def test_truncate_text__suffix_is_horizontal_ellipsis_u2026_and_multibyte_safe():
    body = "a" * 100
    result = output.truncate_text(body, full=False, limit=10)
    assert result is not None
    assert result.endswith("…")
    suffix = result[10:]
    assert len(suffix) == 1
    assert result[-1] == "…"
    assert not result.endswith("...")

    # Multibyte body truncated by codepoint.
    body_mb = "あいうえおかきくけこさしすせそ"  # 15 codepoints
    result_mb = output.truncate_text(body_mb, full=False, limit=10)
    assert result_mb == "あいうえおかきくけこ…"
    assert len(result_mb) == 11


@pytest.mark.parametrize(
    (
        "scenario",
        "description",
        "expected_present_in_render",
        "expected_absent_in_render",
    ),
    [
        ("long_truncated_to_60_plus_ellipsis", "x" * 200, "x" * 60 + "…", "x" * 200),
        ("short_passes_through", "y" * 30, "y" * 30, "…"),
        ("exactly_60_passes_through", "z" * 60, "z" * 60, "…"),
        ("61_truncated", "w" * 61, "w" * 60 + "…", "w" * 61),
    ],
)
def test_format_member_detail_full__description_truncated_at_60(
    scenario, description, expected_present_in_render, expected_absent_in_render
):
    member = {
        "member_id": 123,
        "name": "Claude-B",
        "description": description,
        "status": "active",
        "kind": "member",
        "skills": [],
        "placement": None,
    }
    rendered = output.format_member_detail(member, full=True)
    assert expected_present_in_render in rendered
    if expected_absent_in_render:
        assert expected_absent_in_render not in rendered


def test_truncate_task_text__full_true_bypasses_and_default_uses_settings(monkeypatch):
    # full=True bypasses regardless of settings.
    monkeypatch.setattr("cafleet.config.settings.max_text_len", 10)
    task = {
        "task_id": 1,
        "context_id": 20,
        "from_member_id": 10,
        "to_member_id": 20,
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "a" * 500,
    }
    output.truncate_task_text(task, full=True)
    assert task["text"] == "a" * 500

    # Default uses settings.max_text_len.
    monkeypatch.setattr("cafleet.config.settings.max_text_len", 20)
    task2 = dict(task)
    task2["text"] = "a" * 100
    output.truncate_task_text(task2, full=False)
    assert task2["text"] == "a" * 20 + "…"
