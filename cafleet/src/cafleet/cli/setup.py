"""``cafleet setup`` — onboarding: create the DB schema + install the skills."""

import importlib.metadata
import json
import shutil
import tempfile
import urllib.error
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

import click

from cafleet.broker.skill_installs import (
    record_skill_install,
    skill_installs_table_exists,
)
from cafleet.db.schema import create_schema

GITHUB_REPO = "himkt/cafleet"
SKILL_DIRS = ("cafleet", "cafleet-design-doc", "cafleet-research")
HTTP_TIMEOUT = 30  # seconds; applied to every urlopen (release lookup + download)

AGENT_SKILLS_DIRS = {
    "claude": Path("~/.claude/skills"),
    "codex": Path("~/.codex/skills"),
    "opencode": Path("~/.config/opencode/skills"),
}

SCHEMA_PREFLIGHT_ERROR = (
    "the database schema is missing or outdated; "
    "run 'cafleet setup' or 'cafleet setup db' first"
)


def _resolve_targets(agents: tuple[str, ...]) -> list[str]:
    """Resolve the skills targets from ``--agent`` or by auto-detection."""
    if agents:
        return list(dict.fromkeys(agents))

    detected = [
        agent
        for agent, skills_dir in AGENT_SKILLS_DIRS.items()
        if skills_dir.expanduser().parent.exists()
    ]
    if not detected:
        raise click.ClickException(
            "no coding-agent homes detected (looked for ~/.claude, ~/.codex, "
            "~/.config/opencode); install a coding agent first, or pass --agent"
        )
    return detected


def _resolve_download_url(cli_version: str) -> str:
    """Look up the release for ``cli_version`` and return its skills asset URL."""
    api_url = f"https://api.github.com/repos/{GITHUB_REPO}/releases/tags/{cli_version}"
    req = urllib.request.Request(
        api_url,
        headers={"Accept": "application/vnd.github+json", "User-Agent": "cafleet"},
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
            body = resp.read()
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            raise click.ClickException(
                f"no release found for version {cli_version}"
            ) from exc
        raise click.ClickException(
            f"could not reach the GitHub API ({exc.reason})"
        ) from exc
    except (urllib.error.URLError, TimeoutError) as exc:
        reason = getattr(exc, "reason", exc)
        raise click.ClickException(
            f"could not reach the GitHub API ({reason})"
        ) from exc

    asset_name = f"cafleet-skills-v{cli_version}.zip"
    try:
        assets = json.loads(body)["assets"]
        for asset in assets:
            if asset["name"] == asset_name:
                return asset["browser_download_url"]
    except (json.JSONDecodeError, KeyError, TypeError) as exc:
        raise click.ClickException("could not parse the GitHub API response") from exc
    raise click.ClickException(f"asset {asset_name} not found in release {cli_version}")


def _download_and_extract(download_url: str, dest_root: Path) -> Path:
    """Download the asset, reject unsafe members, extract, and validate layout.

    Returns the extracted ``skills/`` directory. Raises ``click.ClickException``
    on any network or archive failure, before the caller removes any target.
    """
    archive_path = dest_root / "skills.zip"
    req = urllib.request.Request(download_url, headers={"User-Agent": "cafleet"})
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
            archive_path.write_bytes(resp.read())
    except urllib.error.HTTPError as exc:
        raise click.ClickException(
            f"could not reach the GitHub API ({exc.reason})"
        ) from exc
    except (urllib.error.URLError, TimeoutError) as exc:
        reason = getattr(exc, "reason", exc)
        raise click.ClickException(
            f"could not reach the GitHub API ({reason})"
        ) from exc

    extract_root = dest_root / "extracted"
    try:
        with zipfile.ZipFile(archive_path) as zf:
            for member in zf.namelist():
                parts = PurePosixPath(member)
                if parts.is_absolute() or ".." in parts.parts:
                    raise click.ClickException(
                        f"archive member '{member}' has an unsafe path; "
                        "rejecting the archive"
                    )
            zf.extractall(extract_root)
    except (zipfile.BadZipFile, OSError) as exc:
        raise click.ClickException("release asset is malformed") from exc

    skills_root = extract_root / "skills"
    if not skills_root.is_dir():
        raise click.ClickException("release asset is malformed")
    entries = list(skills_root.iterdir())
    if {entry.name for entry in entries} != set(SKILL_DIRS) or not all(
        entry.is_dir() for entry in entries
    ):
        raise click.ClickException("release asset is malformed")
    return skills_root


def _install_skills(targets: list[str], cli_version: str) -> None:
    """Run the full skills half: resolve, download, validate, and install."""
    download_url = _resolve_download_url(cli_version)

    with tempfile.TemporaryDirectory() as tmpdir:
        skills_root = _download_and_extract(download_url, Path(tmpdir))

        for agent in targets:
            skills_dir = AGENT_SKILLS_DIRS[agent].expanduser()
            try:
                for name in SKILL_DIRS:
                    dest = skills_dir / name
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    if dest.exists():
                        shutil.rmtree(dest)
                    shutil.copytree(skills_root / name, dest)
            except OSError as exc:
                raise click.ClickException(
                    f"failed to install skills into {skills_dir}: {exc}"
                ) from exc
            record_skill_install(agent, cli_version)
            click.echo(
                f"{agent}: installed {', '.join(SKILL_DIRS)} "
                f"(v{cli_version}) -> {AGENT_SKILLS_DIRS[agent]}"
            )


def _run_skills_half(agents: tuple[str, ...]) -> None:
    """Pre-flight the schema, resolve targets, install, and record versions."""
    if not skill_installs_table_exists():
        raise click.ClickException(SCHEMA_PREFLIGHT_ERROR)
    cli_version = importlib.metadata.version("cafleet")
    targets = _resolve_targets(agents)
    _install_skills(targets, cli_version)


@click.group("setup", invoke_without_command=True)
@click.pass_context
def setup(ctx: click.Context) -> None:
    """Create the database schema and install the coding-agent skills."""
    if ctx.invoked_subcommand is not None:
        return

    failures: list[str] = []

    try:
        create_schema()
    except click.ClickException as exc:
        click.echo(f"db half failed: {exc.format_message()}")
        failures.append("db")

    try:
        _run_skills_half(())
    except click.ClickException as exc:
        click.echo(f"skills half failed: {exc.format_message()}")
        failures.append("skills")

    if failures:
        raise click.ClickException(f"{' and '.join(failures)} half failed")


@setup.command("db")
def setup_db() -> None:
    """Create the database schema (idempotent); touches nothing else."""
    db_file = create_schema()
    click.echo(f"schema ready at {db_file}")


@setup.command("skill")
@click.option(
    "--agent",
    "agents",
    type=click.Choice(["claude", "codex", "opencode"]),
    multiple=True,
    help="Scope the skills install to the named agent(s); repeatable. "
    "Omit to auto-detect every coding-agent home that exists.",
)
def setup_skill(agents: tuple[str, ...]) -> None:
    """Install the coding-agent skills and record the installed version."""
    _run_skills_half(agents)
