"""The wheel ships parser/selection code but no model list: the installed
package carries no copy, the build config includes none, and the CLI runs only
against an explicit deployed skill-asset path."""

from pathlib import Path

import cafleet

REPO_ROOT = Path(__file__).resolve().parents[3]


def test_installed_package_carries_no_model_list_copy():
    package_dir = Path(cafleet.__file__).resolve().parent
    assert [path for path in package_dir.rglob("*") if "model-list" in path.name] == []


def test_wheel_build_config_excludes_model_list_and_skills():
    pyproject = (REPO_ROOT / "cafleet" / "pyproject.toml").read_text(encoding="utf-8")
    assert "model-list" not in pyproject
    assert "skills/cafleet" not in pyproject
