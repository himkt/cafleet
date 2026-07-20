"""Monitor/reviewer spawn models come from the selector's policy, not the
legacy fixed overlay choices: a listed `haiku` / `opus` decoy loses to the
policy-selected model."""

import json

import pytest

from tests.cli._model_helpers import run_cli, select_args, write_model_list
from tests.model_selection._helpers import row


@pytest.fixture
def skill_root(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    root = tmp_path / ".claude" / "skills" / "cafleet"
    (root / "reference").mkdir(parents=True)
    return root


def test_monitor_spawn_model_comes_from_selector_not_fixed_haiku(skill_root):
    path = write_model_list(
        skill_root,
        [
            row(model="haiku", rank=10, mon=1, inp="0.1", out="0.2"),
            row(model="budget-model", rank=20, mon=2, inp="0.5", out="1.0"),
        ],
    )
    result = run_cli(select_args(path, "--role", "monitor", "--json"))
    assert result.exit_code == 0, result.output
    selected = json.loads(result.output)["selected"]
    assert selected["model"] == "budget-model"
    assert selected["backend"] == "claude"


def test_reviewer_spawn_model_comes_from_selector_not_fixed_opus(skill_root):
    path = write_model_list(
        skill_root,
        [
            row(model="opus", rank=20, inp="3.0", out="15.0"),
            row(model="frontier-model", rank=30, inp="5.0", out="25.0"),
        ],
    )
    result = run_cli(select_args(path, "--role", "reviewer", "--json"))
    assert result.exit_code == 0, result.output
    selected = json.loads(result.output)["selected"]
    assert selected["model"] == "frontier-model"
    assert selected["backend"] == "claude"
