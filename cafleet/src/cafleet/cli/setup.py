"""``cafleet setup`` — onboarding: migrate the DB + install the coding-agent assets."""

import importlib.metadata
import json
import shutil
import tempfile
import urllib.error
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

import click

from cafleet.broker.asset_installs import (
    asset_installs_table_exists,
    record_asset_install,
)
from cafleet.db.init import run_db_init

GITHUB_REPO = "himkt/cafleet"
SKILL_DIRS = (
    "cafleet",
    "cafleet-design-doc",
    "cafleet-research",
    "cafleet-model-catalog-refresh",
)

CAFLEET_ASSET_MEMBERS = (
    "cafleet/asset-manifest.json",
    "cafleet/reference/model-catalog.md",
)
HTTP_TIMEOUT = 30  # seconds; applied to every urlopen (release lookup + download)

AGENT_SKILLS_DIRS = {
    "claude": Path("~/.claude/skills"),
    "codex": Path("~/.codex/skills"),
    "opencode": Path("~/.config/opencode/skills"),
}

AGENT_PRESET_TARGETS = {
    "codex": Path("~/.codex/rules/cafleet.rules"),
    "opencode": Path("~/.opencode/agents/cafleet.md"),
}

PRESET_ARCHIVE_SOURCES = {
    "codex": "presets/codex/cafleet.rules",
    "opencode": "presets/opencode/cafleet.md",
}

SCHEMA_PREFLIGHT_ERROR = (
    "the database schema is missing or outdated; run 'cafleet setup' first"
)


def _http_get(req: urllib.request.Request, *, on_404: str | None = None) -> bytes:
    """Fetch ``req`` with the shared timeout and GitHub-API error mapping.

    ``HTTPError`` is caught before its ``URLError`` parent; when ``on_404`` is
    given, a 404 raises it instead of the generic unreachable message.
    """
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
            return resp.read()
    except urllib.error.HTTPError as exc:
        if on_404 is not None and exc.code == 404:
            raise click.ClickException(on_404) from exc
        raise click.ClickException(
            f"could not reach the GitHub API ({exc.reason})"
        ) from exc
    except (urllib.error.URLError, TimeoutError) as exc:
        reason = getattr(exc, "reason", exc)
        raise click.ClickException(
            f"could not reach the GitHub API ({reason})"
        ) from exc


def _resolve_download_url(cli_version: str) -> str:
    """Look up the release for ``cli_version`` and return its assets-archive URL."""
    api_url = f"https://api.github.com/repos/{GITHUB_REPO}/releases/tags/{cli_version}"
    req = urllib.request.Request(
        api_url,
        headers={"Accept": "application/vnd.github+json", "User-Agent": "cafleet"},
    )
    body = _http_get(req, on_404=f"no release found for version {cli_version}")

    asset_name = f"cafleet-assets-v{cli_version}.zip"
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

    Returns the extracted archive root holding ``skills/`` and ``presets/``.
    Raises ``click.ClickException`` on any network or archive failure, before
    the caller removes any target.
    """
    archive_path = dest_root / "assets.zip"
    req = urllib.request.Request(download_url, headers={"User-Agent": "cafleet"})
    archive_path.write_bytes(_http_get(req))

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
    for member in CAFLEET_ASSET_MEMBERS:
        if not (skills_root / member).is_file():
            raise click.ClickException("release asset is malformed")
    for source in PRESET_ARCHIVE_SOURCES.values():
        if not (extract_root / source).is_file():
            raise click.ClickException("release asset is malformed")
    return extract_root


def _install_preset(agent: str, extract_root: Path) -> None:
    """Install ``agent``'s bundled preset, overwriting whatever exists.

    The symlink check runs before the directory check because ``is_dir()``
    follows symlinks and ``shutil.rmtree`` refuses them.
    """
    target = AGENT_PRESET_TARGETS[agent].expanduser()
    try:
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.is_symlink():
            target.unlink()
        elif target.is_dir():
            shutil.rmtree(target)
        elif target.exists():
            target.unlink()
        shutil.copyfile(extract_root / PRESET_ARCHIVE_SOURCES[agent], target)
    except OSError as exc:
        raise click.ClickException(
            f"failed to install preset into {target}: {exc}"
        ) from exc


def _install_assets(targets: list[str], cli_version: str) -> None:
    """Run the full assets half: resolve, download, validate, and install."""
    download_url = _resolve_download_url(cli_version)

    with tempfile.TemporaryDirectory() as tmpdir:
        extract_root = _download_and_extract(download_url, Path(tmpdir))
        skills_root = extract_root / "skills"

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
            has_preset = agent in AGENT_PRESET_TARGETS
            if has_preset:
                _install_preset(agent, extract_root)
            record_asset_install(agent, cli_version)
            click.echo(
                f"{agent}: installed {', '.join(SKILL_DIRS)} "
                f"(v{cli_version}) -> {AGENT_SKILLS_DIRS[agent]}"
            )
            if has_preset:
                click.echo(
                    f"{agent}: installed preset "
                    f"(v{cli_version}) -> {AGENT_PRESET_TARGETS[agent]}"
                )


def _run_assets_half(targets: list[str]) -> None:
    """Pre-flight the schema, then install and record versions per target."""
    if not asset_installs_table_exists():
        raise click.ClickException(SCHEMA_PREFLIGHT_ERROR)
    cli_version = importlib.metadata.version("cafleet")
    _install_assets(targets, cli_version)


@click.command("setup")
@click.option(
    "--skip",
    "skip",
    multiple=True,
    type=click.Choice(["claude", "codex", "opencode"]),
    help="Skip the named agent's assets install (repeatable).",
)
def setup(skip: tuple[str, ...]) -> None:
    """Migrate the database schema and install the coding-agent assets
    (skills and presets).
    """
    skipped = set(skip)
    targets = [agent for agent in AGENT_SKILLS_DIRS if agent not in skipped]

    failures: list[str] = []

    try:
        run_db_init()
    except click.ClickException as exc:
        click.echo(f"db half failed: {exc.format_message()}")
        failures.append("db")

    if not targets:
        click.echo("assets half skipped (all agents skipped)")
    else:
        try:
            _run_assets_half(targets)
        except click.ClickException as exc:
            click.echo(f"assets half failed: {exc.format_message()}")
            failures.append("assets")

    if failures:
        raise click.ClickException(f"{' and '.join(failures)} half failed")
