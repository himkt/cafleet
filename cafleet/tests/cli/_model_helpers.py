"""Shared builders for the ``cafleet model select`` CLI test files: deployed
catalog writers and invocation shorthands."""

from datetime import UTC, datetime, timedelta

from click.testing import CliRunner

from cafleet.cli import cli
from tests.model_selection._helpers import catalog_markdown


def stamp(delta=timedelta()):
    return (datetime.now(UTC) + delta).strftime("%Y-%m-%dT%H:%M:%SZ")


def fresh(payload):
    payload["generated_at"] = stamp()
    for source in payload["sources"].values():
        source["retrieved_at"] = stamp()
    return payload


def write_catalog(root, payload):
    (root / "reference").mkdir(parents=True, exist_ok=True)
    catalog_path = root / "reference" / "model-catalog.md"
    catalog_path.write_text(catalog_markdown(payload), encoding="utf-8")
    return catalog_path


def run_cli(args):
    return CliRunner().invoke(cli, args)


def select_args(catalog_path, *extra):
    return ["model", "select", "--catalog", str(catalog_path), *extra]
