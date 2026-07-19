"""Per-candidate backend eligibility: a backend needs a current ``asset_installs``
row and an installed skill replica whose manifest and catalog fingerprints match
the Director replica, or it is excluded before ranking."""

import json
import sqlite3

import pytest

from tests.cli._model_helpers import (
    RUNTIME_VERSION,
    fresh,
    run_cli,
    select_args,
    write_catalog,
    write_manifest,
    write_replica,
)
from tests.model_selection._helpers import catalog_with, make_model


@pytest.fixture
def registry_db(_cli_registry):
    return _cli_registry


@pytest.fixture
def homes(tmp_path, monkeypatch):
    """Redirected HOME with a Director-side claude replica root and a (not yet
    written) codex replica root; every coding-agent binary is discoverable."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("shutil.which", lambda name: f"/usr/bin/{name}")
    claude_root = tmp_path / ".claude" / "skills" / "cafleet"
    codex_root = tmp_path / ".codex" / "skills" / "cafleet"
    (claude_root / "reference").mkdir(parents=True)
    return claude_root, codex_root


def _seed_codex_install(db_path, version=RUNTIME_VERSION):
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT OR REPLACE INTO asset_installs"
            " (coding_agent, cafleet_version, installed_at) VALUES (?, ?, ?)",
            ("codex", version, "2026-07-19T00:00:00+00:00"),
        )
        conn.commit()
    finally:
        conn.close()


def _mixed_payload():
    cheap_codex = make_model(
        backend="codex",
        sku="cheap-codex-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
    )
    claude = make_model(sku="claude-model", rank=20, input_price=2.0, output_price=4.0)
    return fresh(catalog_with([cheap_codex, claude]))


def _select(catalog_path):
    return run_cli(select_args(catalog_path, "--role", "drafter", "--json"))


def test_current_codex_replica_is_eligible(homes, registry_db):
    claude_root, codex_root = homes
    catalog_path = write_catalog(claude_root, _mixed_payload())
    write_replica(codex_root, catalog_path.read_text(encoding="utf-8"))
    _seed_codex_install(registry_db)
    result = _select(catalog_path)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "codex:cheap-codex-model"


def test_missing_codex_replica_excludes_backend(homes, registry_db):
    claude_root, codex_root = homes
    catalog_path = write_catalog(claude_root, _mixed_payload())
    _seed_codex_install(registry_db)
    result = _select(catalog_path)
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["selected"]["key"] == "claude:claude-model"
    excluded = next(
        c for c in data["candidates"] if c["key"] == "codex:cheap-codex-model"
    )
    assert excluded["eligible"] is False
    assert excluded["reason"]


def test_unrecorded_codex_install_excludes_backend(homes):
    claude_root, codex_root = homes
    catalog_path = write_catalog(claude_root, _mixed_payload())
    write_replica(codex_root, catalog_path.read_text(encoding="utf-8"))
    result = _select(catalog_path)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:claude-model"


def test_stale_codex_install_excludes_backend(homes, registry_db):
    claude_root, codex_root = homes
    catalog_path = write_catalog(claude_root, _mixed_payload())
    write_replica(codex_root, catalog_path.read_text(encoding="utf-8"))
    _seed_codex_install(registry_db, version="0.0.1")
    result = _select(catalog_path)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:claude-model"


def test_mismatched_codex_replica_fingerprint_excludes_backend(homes, registry_db):
    claude_root, codex_root = homes
    catalog_path = write_catalog(claude_root, _mixed_payload())
    write_replica(codex_root, catalog_path.read_text(encoding="utf-8"))
    write_manifest(codex_root, "divergent replica bytes\n")
    _seed_codex_install(registry_db)
    result = _select(catalog_path)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["selected"]["key"] == "claude:claude-model"


def test_every_backend_without_replica_is_candidate_asset_unavailable(
    homes, registry_db
):
    claude_root, codex_root = homes
    codex_only = make_model(
        backend="codex",
        sku="only-codex-model",
        rank=10,
        input_price=0.5,
        output_price=1.0,
    )
    catalog_path = write_catalog(claude_root, fresh(catalog_with([codex_only])))
    _seed_codex_install(registry_db)
    result = _select(catalog_path)
    assert result.exit_code == 1, result.output
    assert (
        json.loads(result.output)["error"]["code"]
        == "MODEL_CANDIDATE_ASSET_UNAVAILABLE"
    )


def test_select_without_catalog_flag_is_missing_option_error(homes):
    result = run_cli(["model", "select", "--role", "drafter"])
    assert result.exit_code == 2, result.output
    assert "--catalog" in result.output
