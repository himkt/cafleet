"""Shared builders for the ``cafleet model select`` CLI test files: deployed
skill-replica writers with matching asset manifests, and invocation shorthands."""

import hashlib
import importlib.metadata
import json
from datetime import UTC, datetime, timedelta

from click.testing import CliRunner

from cafleet.cli import cli
from tests.model_selection._helpers import catalog_markdown

RUNTIME_VERSION = importlib.metadata.version("cafleet")


def stamp(delta=timedelta()):
    return (datetime.now(UTC) + delta).strftime("%Y-%m-%dT%H:%M:%SZ")


def fresh(payload):
    payload["generated_at"] = stamp()
    for source in payload["sources"].values():
        source["retrieved_at"] = stamp()
    return payload


def write_manifest(root, catalog_text, *, version=RUNTIME_VERSION):
    manifest = {
        "cafleet_version": version,
        "catalog_sha256": hashlib.sha256(catalog_text.encode("utf-8")).hexdigest(),
    }
    (root / "asset-manifest.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )


def write_replica(root, catalog_text, *, version=RUNTIME_VERSION):
    """Materialize a deployed cafleet skill replica: catalog + matching manifest."""
    (root / "reference").mkdir(parents=True, exist_ok=True)
    catalog_path = root / "reference" / "model-catalog.md"
    catalog_path.write_text(catalog_text, encoding="utf-8")
    write_manifest(root, catalog_text, version=version)
    return catalog_path


def write_catalog(root, payload):
    return write_replica(root, catalog_markdown(payload))


def run_cli(args):
    return CliRunner().invoke(cli, args)


def select_args(catalog_path, *extra):
    return ["model", "select", "--catalog", str(catalog_path), *extra]
