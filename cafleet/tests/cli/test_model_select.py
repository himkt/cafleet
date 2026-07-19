"""``cafleet model select`` — CLI boundary: explicit catalog path validation,
JSON success/error envelopes, human output without shell synthesis, override
flags, and the deferred assets guard."""

import json
from datetime import UTC, datetime, timedelta

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from tests.model_selection._helpers import (
    catalog_markdown,
    catalog_with,
    make_model,
    uniform_levels,
)


def _stamp(delta=timedelta()):
    return (datetime.now(UTC) + delta).strftime("%Y-%m-%dT%H:%M:%SZ")


def _fresh(payload):
    payload["generated_at"] = _stamp()
    for source in payload["sources"].values():
        source["retrieved_at"] = _stamp()
    return payload


def _default_payload():
    cheap = make_model(sku="cheap-model", rank=10, input_price=1.0, output_price=2.0)
    pricey = make_model(
        sku="pricey-model",
        rank=20,
        levels=uniform_levels(coding=5),
        input_price=2.0,
        output_price=4.0,
    )
    return _fresh(catalog_with([cheap, pricey]))


@pytest.fixture
def skill_root(tmp_path, monkeypatch):
    """A deployed cafleet skill root under a redirected HOME, with every
    coding-agent binary discoverable."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    root = tmp_path / ".claude" / "skills" / "cafleet"
    (root / "reference").mkdir(parents=True)
    return root


def _write_catalog(root, payload):
    catalog_path = root / "reference" / "model-catalog.md"
    catalog_path.write_text(catalog_markdown(payload), encoding="utf-8")
    return catalog_path


def _run(args):
    return CliRunner().invoke(cli, args)


def _select_args(catalog_path, *extra):
    return ["model", "select", "--catalog", str(catalog_path), *extra]


# --------------------------------------------------------------------------- #
# JSON success envelope                                                        #
# --------------------------------------------------------------------------- #


def test_select_json_success_envelope(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "cost_minimized_subject_to_capability"
    assert data["role"] == "drafter"
    assert data["selected"]["key"] == "claude:cheap-model"
    assert data["selected"]["backend"] == "claude"
    assert data["selected"]["model"] == "cheap-model"
    assert data["selected"]["canonical_token"] == "cheap-model"
    assert data["selected"]["effort"] is None
    assert data["selected"]["estimated_usd"] == pytest.approx(0.024)
    assert data["task_profile"] == {"planning": 3, "research": 2, "review": 1}
    assert data["selection_id"].startswith("sel_")
    assert data["spawn"] == {"state": "pending", "member_id": None, "error": None}
    assert {c["key"] for c in data["candidates"]} == {
        "claude:cheap-model",
        "claude:pricey-model",
    }


def test_select_json_records_catalog_path(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["catalog_path"] == str(catalog_path)


def test_select_token_flags_replace_profile(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(
            catalog_path,
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
    assert data["token_estimate"]["output"] == 500
    assert data["selected"]["estimated_usd"] == pytest.approx(0.002)


def test_select_requires_flag_raises_floor(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(
            catalog_path, "--role", "drafter", "--requires", "coding=5", "--json"
        )
    )
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:pricey-model"


def test_select_monitor_role_uses_small_profile(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "monitor", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["selected"]["key"] == "claude:cheap-model"
    assert data["token_estimate"]["input"] == 4000
    assert data["token_estimate"]["output"] == 1000


def test_select_human_output_names_selection_without_shell_synthesis(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "drafter"))
    assert result.exit_code == 0, result.output
    assert "cheap-model" in result.output
    assert "claude" in result.output
    assert "member create" not in result.output


# --------------------------------------------------------------------------- #
# JSON error envelopes                                                         #
# --------------------------------------------------------------------------- #


def test_unknown_role_error_envelope(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "wizard", "--json"))
    assert result.exit_code == 2, result.output
    error = json.loads(result.output)["error"]
    assert error["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    assert error["candidates"] == []


def test_malformed_requires_error_envelope(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(
            catalog_path, "--role", "drafter", "--requires", "coding", "--json"
        )
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_missing_catalog_file_path_unavailable(skill_root):
    catalog_path = skill_root / "reference" / "model-catalog.md"
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_CATALOG_PATH_UNAVAILABLE"
    )


def test_relative_catalog_path_rejected(skill_root):
    _write_catalog(skill_root, _default_payload())
    result = _run(
        [
            "model",
            "select",
            "--catalog",
            "reference/model-catalog.md",
            "--role",
            "drafter",
            "--json",
        ]
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_catalog_outside_skill_root_is_still_a_valid_source(skill_root, tmp_path):
    rogue_root = tmp_path / "random"
    rogue_root.mkdir()
    catalog_path = rogue_root / "model-catalog.md"
    catalog_path.write_text(catalog_markdown(_default_payload()), encoding="utf-8")
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:cheap-model"


def test_select_without_catalog_flag_is_missing_option_error(skill_root):
    result = _run(["model", "select", "--role", "drafter"])
    assert result.exit_code == 2, result.output
    assert "--catalog" in result.output


def test_stale_source_error(skill_root):
    payload = _default_payload()
    payload["sources"]["anthropic"]["retrieved_at"] = _stamp(timedelta(days=-60))
    catalog_path = _write_catalog(skill_root, payload)
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_CATALOG_STALE"


def test_invalid_catalog_error(skill_root):
    catalog_path = skill_root / "reference" / "model-catalog.md"
    catalog_path.write_text("not a catalog at all\n", encoding="utf-8")
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_CATALOG_INVALID"


def test_schema_invalid_role_profile_is_catalog_invalid(skill_root):
    payload = _default_payload()
    payload["role_profiles"]["wizard"] = {
        "task_kind": "monitoring",
        "requires": {"monitor": 2},
        "token_profile": "small",
    }
    catalog_path = _write_catalog(skill_root, payload)
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 1, result.output
    assert json.loads(result.output)["error"]["code"] == "MODEL_CATALOG_INVALID"


def test_triggered_by_recorded_in_audit(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(
            catalog_path,
            "--role",
            "drafter",
            "--triggered-by",
            "cost efficiency mode",
            "--json",
        )
    )
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["triggered_by"] == "cost efficiency mode"


def test_absent_trigger_recorded_as_null(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "drafter", "--json"))
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["triggered_by"] is None


def test_no_eligible_candidate_error_lists_exclusions(skill_root):
    weak = make_model(sku="weak-model", rank=10, levels=uniform_levels(coding=3))
    catalog_path = _write_catalog(skill_root, _fresh(catalog_with([weak])))
    result = _run(_select_args(catalog_path, "--role", "programmer", "--json"))
    assert result.exit_code == 1, result.output
    error = json.loads(result.output)["error"]
    assert error["code"] == "MODEL_NO_ELIGIBLE_CANDIDATE"
    assert error["candidates"][0]["key"] == "claude:weak-model"
    assert error["candidates"][0]["reason"]


def test_human_mode_error_carries_code(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "wizard"))
    assert result.exit_code == 2, result.output
    assert "MODEL_SELECTION_INVALID_REQUEST" in result.output


# --------------------------------------------------------------------------- #
# Override flags                                                               #
# --------------------------------------------------------------------------- #


def test_model_pin_returns_manual_override(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--model", "cheap-model", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "manual_override"
    assert data["selected"]["backend"] == "claude"


def test_conflicting_model_backend_pair_rejected(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(
            catalog_path, "--model", "cheap-model", "--coding-agent", "codex", "--json"
        )
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_unmapped_model_pin_estimate_unavailable(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--model", "bespoke-model", "--json"))
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["policy"] == "manual_override"
    assert data["estimate_status"] == "unavailable"


def test_invalid_effort_rejected(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(catalog_path, "--role", "drafter", "--effort", "turbo", "--json")
    )
    assert result.exit_code == 2, result.output
    assert (
        json.loads(result.output)["error"]["code"] == "MODEL_SELECTION_INVALID_REQUEST"
    )


def test_valid_effort_recorded_not_selected(skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(
        _select_args(catalog_path, "--role", "drafter", "--effort", "high", "--json")
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["selected"]["key"] == "claude:cheap-model"
    assert data["selected"]["effort"] == "high"


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


def test_model_group_help_usable_without_install(unseeded_registry):
    result = _run(["model", "--help"])
    assert result.exit_code == 0, result.output
    assert "Usage:" in result.output


def test_model_select_help_usable_without_install(unseeded_registry):
    result = _run(["model", "select", "--help"])
    assert result.exit_code == 0, result.output
    assert "Usage:" in result.output


def test_select_help_documents_trigger_and_estimate_limits(unseeded_registry):
    result = _run(["model", "select", "--help"])
    assert result.exit_code == 0, result.output
    assert "cost efficiency mode" in result.output


def test_model_select_execution_requires_current_assets(unseeded_registry, skill_root):
    catalog_path = _write_catalog(skill_root, _default_payload())
    result = _run(_select_args(catalog_path, "--role", "drafter"))
    assert result.exit_code == 1, result.output
    assert "no assets install is recorded" in result.output
