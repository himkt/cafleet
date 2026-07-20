"""Shared builders for the ``cafleet model select`` CLI tests: deployed
model-list writers and invocation shorthands."""

from datetime import UTC, datetime

from click.testing import CliRunner

from cafleet.cli import cli
from tests.model_selection._helpers import model_list_text


def write_model_list(root, model_rows):
    (root / "reference").mkdir(parents=True, exist_ok=True)
    path = root / "reference" / "model-list.md"
    retrieved_at = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    path.write_text(
        model_list_text(model_rows, retrieved_at=retrieved_at), encoding="utf-8"
    )
    return path


def run_cli(args):
    return CliRunner().invoke(cli, args)


def select_args(model_list_path, *extra):
    return ["model", "select", "--model-list", str(model_list_path), *extra]
