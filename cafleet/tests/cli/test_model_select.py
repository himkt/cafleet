"""``cafleet model select`` — CLI boundary: explicit model-list path validation,
JSON success/error envelopes, human output without shell synthesis, override
flags, and the deferred assets guard."""

import json

import pytest

from cafleet import config
from tests.cli._model_helpers import run_cli, select_args, write_model_list
from tests.model_selection._helpers import model_list_text, row


def _default_rows():
    return [
        row(model="cheap-model", rank=10, inp="1.0", out="2.0"),
        row(model="pricey-model", rank=20, cod=5, inp="2.0", out="4.0"),
    ]


@pytest.fixture
def skill_root(tmp_path, monkeypatch):
    """A deployed cafleet skill root under a redirected HOME, with every
    coding-agent binary discoverable."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    root = tmp_path / ".claude" / "skills" / "cafleet"
    (root / "reference").mkdir(parents=True)
    return root


# --------------------------------------------------------------------------- #
# JSON success envelope                                                        #
# --------------------------------------------------------------------------- #


def test_select_json_success_envelope(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--role", "drafter", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "cost_minimized_subject_to_capability"
    assert data["role"] == "drafter"
    assert data["selected"]["key"] == "claude:cheap-model"
    assert data["selected"]["backend"] == "claude"
    assert data["selected"]["model"] == "cheap-model"
    assert data["selected"]["effort"] is None
    assert data["selected"]["estimated_usd"] == pytest.approx(0.024)
    assert data["task_profile"] == {"planning": 3, "research": 2, "review": 1}
    assert data["selection_id"].startswith("sel_")
    assert data["spawn"] == {"state": "pending", "member_id": None, "error": None}
    assert data["model_list_path"] == str(path)
    assert {c["key"] for c in data["candidates"]} == {
        "claude:cheap-model",
        "claude:pricey-model",
    }


def test_select_token_and_requires_flags(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(
        select_args(
            path,
            "--role",
            "drafter",
            "--estimated-input-tokens",
            "1000",
            "--estimated-output-tokens",
            "500",
            "--json",
        )
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["token_estimate"]["input"] == 1000
    assert data["selected"]["estimated_usd"] == pytest.approx(0.002)
    result = run_cli(
        select_args(path, "--role", "drafter", "--requires", "coding=5", "--json")
    )
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:pricey-model"


def test_select_human_output_names_selection_without_shell_synthesis(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--role", "drafter"))
    assert result.exit_code == 0, result.output
    assert "cheap-model" in result.output
    assert "claude" in result.output
    assert "member create" not in result.output


def test_triggered_by_recorded_in_audit(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(
        select_args(
            path,
            "--role",
            "drafter",
            "--triggered-by",
            "cost efficiency mode",
            "--json",
        )
    )
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["triggered_by"] == "cost efficiency mode"


# --------------------------------------------------------------------------- #
# JSON error envelopes                                                         #
# --------------------------------------------------------------------------- #


def test_unknown_role_error_envelope(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--role", "wizard", "--json"))
    assert result.exit_code == 2, result.output
    error = json.loads(result.output)["error"]
    assert error["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    assert error["candidates"] == []


def test_missing_model_list_file_path_unavailable(skill_root):
    path = skill_root / "reference" / "model-list.md"
    result = run_cli(select_args(path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_LIST_PATH_UNAVAILABLE"


def test_relative_model_list_path_rejected(skill_root):
    write_model_list(skill_root, _default_rows())
    result = run_cli(
        select_args("reference/model-list.md", "--role", "drafter", "--json")
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_select_without_model_list_flag_is_missing_option_error(skill_root):
    result = run_cli(["model", "select", "--role", "drafter"])
    assert result.exit_code == 2, result.output
    assert "--model-list" in result.output


def test_invalid_model_list_error(skill_root):
    path = skill_root / "reference" / "model-list.md"
    path.write_text("not a model list at all\n", encoding="utf-8")
    result = run_cli(select_args(path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_LIST_INVALID"


def test_stale_source_error(skill_root):
    path = skill_root / "reference" / "model-list.md"
    path.write_text(
        model_list_text(_default_rows(), retrieved_at="2026-01-01T00:00:00Z"),
        encoding="utf-8",
    )
    result = run_cli(select_args(path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_LIST_STALE"


def test_no_eligible_candidate_error_lists_exclusions(skill_root):
    path = write_model_list(skill_root, [row(model="weak-model", rank=10, cod=3)])
    result = run_cli(select_args(path, "--role", "programmer", "--json"))
    assert result.exit_code == 1, result.output
    error = json.loads(result.output)["error"]
    assert error["code"] == "MODEL_NO_ELIGIBLE_CANDIDATE"
    assert error["candidates"][0]["key"] == "claude:weak-model"
    assert error["candidates"][0]["reason"]


def test_human_mode_error_carries_code(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--role", "wizard"))
    assert result.exit_code == 2, result.output
    assert "MODEL_SELECTION_INVALID_REQUEST" in result.output


# --------------------------------------------------------------------------- #
# Override flags                                                               #
# --------------------------------------------------------------------------- #


def test_model_pin_returns_manual_override(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--model", "cheap-model", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "manual_override"
    assert data["selected"]["backend"] == "claude"


def test_conflicting_model_backend_pair_rejected(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(
        select_args(path, "--model", "cheap-model", "--coding-agent", "codex", "--json")
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_unmapped_model_pin_estimate_unavailable(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--model", "bespoke-model", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "manual_override"
    assert data["estimate_status"] == "unavailable"


def test_effort_is_validated_pass_through_not_a_selector(skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(
        select_args(path, "--role", "drafter", "--effort", "high", "--json")
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["selected"]["key"] == "claude:cheap-model"
    assert data["selected"]["effort"] == "high"
    result = run_cli(
        select_args(path, "--role", "drafter", "--effort", "turbo", "--json")
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


# --------------------------------------------------------------------------- #
# Assets guard timing and help usability                                       #
# --------------------------------------------------------------------------- #


@pytest.fixture
def unseeded_registry(tmp_path, monkeypatch):
    db_path = tmp_path / "empty-registry" / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_path}"
    )
    return db_path


def test_model_help_usable_without_install(unseeded_registry):
    for args in (["model", "--help"], ["model", "select", "--help"]):
        result = run_cli(args)
        assert result.exit_code == 0, result.output
        assert "Usage:" in result.output


def test_select_help_documents_trigger_and_estimate_limits(unseeded_registry):
    result = run_cli(["model", "select", "--help"])
    assert result.exit_code == 0, result.output
    assert "cost efficiency mode" in result.output


def test_model_select_execution_requires_current_assets(unseeded_registry, skill_root):
    path = write_model_list(skill_root, _default_rows())
    result = run_cli(select_args(path, "--role", "drafter"))
    assert result.exit_code == 1, result.output
    assert "no assets install is recorded" in result.output
