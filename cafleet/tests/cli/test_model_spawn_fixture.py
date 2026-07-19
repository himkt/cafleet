"""Monitor/reviewer spawn models come from the selector's catalog policy, not
the legacy fixed overlay choices: a catalog `haiku` / `opus` decoy loses to the
policy-selected model."""

import json

import pytest

from tests.cli._model_helpers import fresh, run_cli, select_args, write_catalog
from tests.model_selection._helpers import catalog_with, make_model, uniform_levels


@pytest.fixture
def skill_root(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    root = tmp_path / ".claude" / "skills" / "cafleet"
    (root / "reference").mkdir(parents=True)
    return root


def test_monitor_spawn_model_comes_from_selector_not_fixed_haiku(skill_root):
    haiku_decoy = make_model(
        sku="haiku",
        rank=10,
        levels=uniform_levels(monitor=1),
        input_price=0.1,
        output_price=0.2,
    )
    budget = make_model(
        sku="budget-model",
        rank=20,
        levels=uniform_levels(monitor=2),
        input_price=0.5,
        output_price=1.0,
    )
    catalog_path = write_catalog(skill_root, fresh(catalog_with([haiku_decoy, budget])))
    result = run_cli(select_args(catalog_path, "--role", "monitor", "--json"))
    assert result.exit_code == 0, result.output
    selected = json.loads(result.output)["selected"]
    assert selected["model"] == "budget-model"
    assert selected["backend"] == "claude"


def test_reviewer_spawn_model_comes_from_selector_not_fixed_opus(skill_root):
    opus_decoy = make_model(sku="opus", rank=20, input_price=3.0, output_price=15.0)
    frontier = make_model(
        sku="frontier-model", rank=30, input_price=5.0, output_price=25.0
    )
    catalog_path = write_catalog(
        skill_root, fresh(catalog_with([opus_decoy, frontier]))
    )
    result = run_cli(select_args(catalog_path, "--role", "reviewer", "--json"))
    assert result.exit_code == 0, result.output
    selected = json.loads(result.output)["selected"]
    assert selected["model"] == "frontier-model"
    assert selected["backend"] == "claude"
