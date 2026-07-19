"""``cafleet model`` — cost-aware model selection against the deployed catalog."""

import hashlib
import importlib.metadata
import json
import uuid
from datetime import UTC, datetime
from pathlib import Path

import click

from cafleet import output
from cafleet.broker.asset_installs import (
    asset_installs_table_exists,
    list_asset_installs,
)
from cafleet.cli._helpers import json_flag
from cafleet.cli.setup import AGENT_SKILLS_DIRS
from cafleet.coding_agent import CODING_AGENTS
from cafleet.model_selection import (
    CandidateRecord,
    Catalog,
    CatalogInvalidError,
    ManualOverrideResult,
    ModelSelectionError,
    SelectionResult,
    parse_catalog_markdown,
    resolve_manual_override,
    select_model,
)


@click.group("model")
def model_group() -> None:
    """Cost-aware model selection commands.

    The assets guard runs at command execution, not here, so ``--help`` stays
    usable before ``cafleet setup``.
    """


@model_group.command("select")
@click.option(
    "--catalog",
    "catalog_arg",
    required=True,
    help="Absolute path to the loaded skill replica's reference/model-catalog.md.",
)
@click.option(
    "--role",
    default=None,
    help="Catalog role-profile key (e.g. monitor, reviewer, programmer).",
)
@click.option(
    "--coding-agent",
    "coding_agent",
    default=None,
    help="Backend override: restrict candidates to claude, codex, or opencode.",
)
@click.option(
    "--model",
    "model_pin",
    default=None,
    help="Explicit model pin (manual override); resolves through the catalog token map.",
)
@click.option(
    "--effort",
    default=None,
    help="Reasoning-effort pass-through; validated against the selected backend, never ranked.",
)
@click.option(
    "--requires",
    "requires_args",
    multiple=True,
    help="dimension=level (repeatable); may only raise a role-profile floor.",
)
@click.option("--estimated-input-tokens", "input_tokens", type=int, default=None)
@click.option(
    "--estimated-cached-input-tokens", "cached_input_tokens", type=int, default=None
)
@click.option(
    "--estimated-cache-write-tokens", "cache_write_tokens", type=int, default=None
)
@click.option("--estimated-output-tokens", "output_tokens", type=int, default=None)
@click.option(
    "--triggered-by",
    "triggered_by",
    default=None,
    help="Recorded activation phrase for the audit trail.",
)
@json_flag
def model_select(
    catalog_arg,
    role,
    coding_agent,
    model_pin,
    effort,
    requires_args,
    input_tokens,
    cached_input_tokens,
    cache_write_tokens,
    output_tokens,
    triggered_by,
    json_output,
):
    """Select a backend and model from the local model catalog.

    \b
    Trigger: automatic cost-minimized selection for an ordinary member applies
    only when the user request contains the exact phrase 'cost efficiency mode';
    monitor and reviewer policies apply on every team spawn.

    \b
    Overrides: an explicit --model pin is a manual override — it resolves
    through the catalog token map, fixes the backend, and is recorded rather
    than replaced. --coding-agent restricts candidates to one backend;
    --effort is validated pass-through and never selects or ranks a model.

    \b
    Estimates use standard direct-provider USD token prices from the catalog's
    approved sources. They are planning estimates only — not a subscription,
    marketplace, regional, or negotiated invoice guarantee.
    """
    _ensure_any_assets_install()
    now = datetime.now(UTC)
    token_estimate = _token_estimate_from_flags(
        input_tokens, cached_input_tokens, cache_write_tokens, output_tokens
    )
    try:
        catalog_path = _validated_catalog_path(catalog_arg)
        catalog_bytes, catalog_sha, manifest_sha = _validated_catalog_asset(
            catalog_path
        )
        try:
            catalog = parse_catalog_markdown(catalog_bytes.decode("utf-8"))
        except (CatalogInvalidError, UnicodeDecodeError) as exc:
            raise ModelSelectionError("MODEL_CATALOG_INVALID", str(exc)) from exc
        catalog_asset = {
            "path": str(catalog_path),
            "cafleet_version": importlib.metadata.version("cafleet"),
            "manifest_sha256": manifest_sha,
            "catalog_sha256": catalog_sha,
        }
        if coding_agent is not None and coding_agent not in CODING_AGENTS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown coding agent {coding_agent!r}",
            )
        if model_pin is not None:
            result = resolve_manual_override(
                catalog,
                model=model_pin,
                backend=coding_agent,
                now=now,
                token_estimate=token_estimate,
            )
            _validate_effort(result.backend, effort)
            payload = _manual_payload(result, effort, triggered_by, catalog_asset, now)
        else:
            if role is None:
                raise ModelSelectionError(
                    "MODEL_SELECTION_INVALID_REQUEST",
                    "either --role or --model is required",
                )
            requires = _parse_requires(requires_args)
            ready_backends, asset_dropped = _ready_backends(coding_agent, manifest_sha)
            try:
                result = select_model(
                    catalog,
                    role=role,
                    ready_backends=ready_backends,
                    now=now,
                    requires=requires,
                    token_estimate=token_estimate,
                    backend=coding_agent,
                )
            except ModelSelectionError as exc:
                raise _remap_asset_dropped(exc, asset_dropped) from exc
            _validate_effort(result.selected.backend, effort)
            payload = _selection_payload(
                result,
                catalog,
                effort,
                triggered_by,
                token_estimate is not None,
                catalog_asset,
                now,
            )
    except ModelSelectionError as exc:
        _emit_error(exc, json_output)
        return
    if json_output:
        click.echo(output.format_json(payload))
    else:
        _echo_human(payload)


def _emit_error(exc: ModelSelectionError, json_output: bool) -> None:
    if json_output:
        envelope = {
            "error": {
                "code": exc.code,
                "message": str(exc),
                "details": {},
                "candidates": [_candidate_dict(c) for c in exc.candidates],
            }
        }
        click.echo(output.format_json(envelope))
    else:
        click.echo(f"Error: {exc.code}: {exc}", err=True)
    raise SystemExit(2 if exc.code == "MODEL_SELECTION_INVALID_REQUEST" else 1)


def _validated_catalog_path(catalog_arg: str) -> Path:
    path = Path(catalog_arg)
    if not path.is_absolute():
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"--catalog must be an absolute path, got {catalog_arg!r}",
        )
    if not path.is_file():
        raise ModelSelectionError(
            "MODEL_CATALOG_PATH_UNAVAILABLE",
            f"catalog path {catalog_arg!r} is absent or not a regular file",
        )
    return path


def _validated_catalog_asset(catalog_path: Path) -> tuple[bytes, str, str]:
    """Enforce the loaded-skill-root layout and the release asset fingerprint."""
    skill_root = catalog_path.parent.parent
    if (
        catalog_path.name != "model-catalog.md"
        or catalog_path.parent.name != "reference"
        or skill_root.name != "cafleet"
    ):
        raise ModelSelectionError(
            "MODEL_CATALOG_ASSET_MISMATCH",
            f"catalog path {str(catalog_path)!r} is not under a deployed"
            " cafleet skill root (<root>/cafleet/reference/model-catalog.md)",
        )
    manifest_path = skill_root / "asset-manifest.json"
    if not manifest_path.is_file():
        raise ModelSelectionError(
            "MODEL_CATALOG_ASSET_MISMATCH",
            f"no asset manifest at {str(manifest_path)!r}",
        )
    manifest_bytes = manifest_path.read_bytes()
    try:
        manifest = json.loads(manifest_bytes)
        manifest_version = manifest["cafleet_version"]
        manifest_catalog_sha = manifest["catalog_sha256"]
    except (json.JSONDecodeError, TypeError, KeyError) as exc:
        raise ModelSelectionError(
            "MODEL_CATALOG_ASSET_MISMATCH",
            f"asset manifest {str(manifest_path)!r} is malformed",
        ) from exc
    runtime_version = importlib.metadata.version("cafleet")
    if manifest_version != runtime_version:
        raise ModelSelectionError(
            "MODEL_CATALOG_ASSET_MISMATCH",
            f"asset manifest version {manifest_version!r} does not match"
            f" the installed CLI version {runtime_version!r}",
        )
    catalog_bytes = catalog_path.read_bytes()
    catalog_sha = hashlib.sha256(catalog_bytes).hexdigest()
    if catalog_sha != manifest_catalog_sha:
        raise ModelSelectionError(
            "MODEL_CATALOG_ASSET_MISMATCH",
            "catalog file hash does not match the release manifest fingerprint",
        )
    manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()
    return catalog_bytes, catalog_sha, manifest_sha


def _ensure_any_assets_install() -> None:
    """Require a recorded assets install; per-backend staleness is a candidate
    exclusion in ``_ready_backends``, not a command-level failure."""
    rows = list_asset_installs() if asset_installs_table_exists() else []
    if not rows:
        raise click.ClickException(
            "no assets install is recorded; run 'cafleet setup' first"
        )


def _ready_backends(
    coding_agent: str | None, manifest_sha: str
) -> tuple[frozenset[str], frozenset[str]]:
    """Resolve the runtime-ready, asset-matched candidate backend set.

    Returns ``(ready, asset_dropped)`` where ``asset_dropped`` holds the
    binary-ready backends excluded for lacking a current matching skill replica.
    """
    requested = (
        frozenset(CODING_AGENTS) if coding_agent is None else frozenset({coding_agent})
    )
    installs = {
        row["coding_agent"]: row["cafleet_version"] for row in list_asset_installs()
    }
    runtime_version = importlib.metadata.version("cafleet")
    binary_ready = set()
    ready = set()
    for backend in sorted(requested):
        try:
            CODING_AGENTS[backend].ensure_available()
        except RuntimeError:
            continue
        binary_ready.add(backend)
        if installs.get(backend) != runtime_version:
            continue
        replica_root = AGENT_SKILLS_DIRS[backend].expanduser() / "cafleet"
        if _replica_matches(replica_root, manifest_sha):
            ready.add(backend)
    if not binary_ready:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    if not ready:
        raise ModelSelectionError(
            "MODEL_CANDIDATE_ASSET_UNAVAILABLE",
            "every otherwise eligible backend lacks a current matching"
            " CAFleet skill replica",
        )
    return frozenset(ready), frozenset(binary_ready - ready)


def _remap_asset_dropped(
    exc: ModelSelectionError, asset_dropped: frozenset[str]
) -> ModelSelectionError:
    """A no-eligible-candidate failure whose every candidate was excluded for an
    asset-dropped backend is the candidate-asset error, not a capability one."""
    if (
        exc.code != "MODEL_NO_ELIGIBLE_CANDIDATE"
        or not exc.candidates
        or not all(
            candidate.key.split(":", 1)[0] in asset_dropped
            for candidate in exc.candidates
        )
    ):
        return exc
    return ModelSelectionError(
        "MODEL_CANDIDATE_ASSET_UNAVAILABLE",
        "every otherwise eligible backend lacks a current matching"
        " CAFleet skill replica",
        candidates=exc.candidates,
    )


def _replica_matches(replica_root: Path, director_manifest_sha: str) -> bool:
    """A candidate replica matches when its full manifest hash equals the
    Director replica's and its catalog hashes to its own manifest value."""
    manifest_path = replica_root / "asset-manifest.json"
    catalog_path = replica_root / "reference" / "model-catalog.md"
    if not manifest_path.is_file() or not catalog_path.is_file():
        return False
    manifest_bytes = manifest_path.read_bytes()
    if hashlib.sha256(manifest_bytes).hexdigest() != director_manifest_sha:
        return False
    try:
        manifest_catalog_sha = json.loads(manifest_bytes)["catalog_sha256"]
    except (json.JSONDecodeError, TypeError, KeyError):
        return False
    catalog_sha = hashlib.sha256(catalog_path.read_bytes()).hexdigest()
    return catalog_sha == manifest_catalog_sha


def _validate_effort(backend: str | None, effort: str | None) -> None:
    if effort is None or backend is None:
        return
    try:
        CODING_AGENTS[backend].validate_effort(effort)
    except ValueError as exc:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"invalid effort {effort!r} for backend {backend!r}: {exc}",
        ) from exc


def _parse_requires(requires_args: tuple[str, ...]) -> dict[str, int] | None:
    if not requires_args:
        return None
    requires = {}
    for raw in requires_args:
        dimension, separator, level = raw.partition("=")
        if not separator or not dimension or not level:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"--requires must be dimension=level, got {raw!r}",
            )
        try:
            requires[dimension] = int(level)
        except ValueError as exc:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"--requires level must be an integer, got {raw!r}",
            ) from exc
    return requires


def _token_estimate_from_flags(
    input_tokens, cached_input_tokens, cache_write_tokens, output_tokens
) -> dict[str, int] | None:
    values = {
        "input": input_tokens,
        "cached_input": cached_input_tokens,
        "cache_write": cache_write_tokens,
        "output": output_tokens,
    }
    estimate = {name: value for name, value in values.items() if value is not None}
    return estimate or None


def _selection_id(now: datetime) -> str:
    return f"sel_{now.strftime('%Y%m%dT%H%M%SZ')}_{uuid.uuid4().hex[:6]}"


def _candidate_dict(candidate: CandidateRecord) -> dict:
    record: dict[str, object] = {"key": candidate.key, "eligible": candidate.eligible}
    if candidate.reason is not None:
        record["reason"] = candidate.reason
    if candidate.estimated_usd is not None:
        record["estimated_usd"] = candidate.estimated_usd
    return record


def _snapshot_model(catalog: Catalog, key: str) -> dict:
    model = next(m for m in catalog.models if m.key == key)
    return {
        "key": model.key,
        "backend": model.backend,
        "provider_sku": model.provider_sku,
        "provider": model.provider,
        "global_rank": model.capability.global_rank,
        "levels": dict(model.capability.levels),
        "rate_cards": [
            {
                "id": card.id,
                "status": card.status,
                "max_total_tokens": card.max_total_tokens,
                "components": {
                    name: {
                        "mode": component.mode,
                        "usd_per_mtok": component.usd_per_mtok,
                    }
                    for name, component in card.components.items()
                },
                "effective_from": card.effective_from.isoformat(),
                "effective_until": (
                    card.effective_until.isoformat()
                    if card.effective_until is not None
                    else None
                ),
                "pricing_source": card.pricing_source,
            }
            for card in model.rate_cards
        ],
    }


def _selection_payload(
    result: SelectionResult,
    catalog: Catalog,
    effort: str | None,
    triggered_by: str | None,
    explicit_estimate: bool,
    catalog_asset: dict,
    now: datetime,
) -> dict:
    return {
        "policy": result.policy,
        "role": result.role,
        "triggered_by": triggered_by,
        "task_profile": dict(result.task_profile),
        "token_estimate": {
            "input": result.token_estimate.input,
            "cached_input": result.token_estimate.cached_input,
            "cache_write": result.token_estimate.cache_write,
            "output": result.token_estimate.output,
            "source": "director" if explicit_estimate else "role_profile",
        },
        "candidates": [_candidate_dict(c) for c in result.candidates],
        "selection_id": _selection_id(now),
        "selected": {
            "key": result.selected.key,
            "backend": result.selected.backend,
            "model": result.selected.model,
            "canonical_token": result.selected.canonical_token,
            "effort": effort,
            "estimated_usd": result.selected.estimated_usd,
        },
        "catalog": {
            "schema_version": catalog.schema_version,
            "generated_at": catalog.generated_at.isoformat(),
            "source_hashes": {
                name: source.content_sha256 for name, source in catalog.sources.items()
            },
            "snapshot": {
                "eligible_models": [
                    _snapshot_model(catalog, candidate.key)
                    for candidate in result.candidates
                    if candidate.eligible
                ]
            },
        },
        "catalog_asset": catalog_asset,
        "spawn": {"state": "pending", "member_id": None, "error": None},
    }


def _manual_payload(
    result: ManualOverrideResult,
    effort: str | None,
    triggered_by: str | None,
    catalog_asset: dict,
    now: datetime,
) -> dict:
    return {
        "policy": result.policy,
        "triggered_by": triggered_by,
        "estimate_status": result.estimate_status,
        "selection_id": _selection_id(now),
        "selected": {
            "key": result.model_key,
            "backend": result.backend,
            "model": result.model,
            "canonical_token": result.canonical_token,
            "effort": effort,
            "estimated_usd": result.estimated_usd,
        },
        "catalog_asset": catalog_asset,
        "spawn": {"state": "pending", "member_id": None, "error": None},
    }


def _echo_human(payload: dict) -> None:
    selected = payload["selected"]
    click.echo(f"policy: {payload['policy']}")
    click.echo(
        f"selected: {selected['model']}"
        f" (backend {selected['backend']}, key {selected['key']})"
    )
    if selected["effort"] is not None:
        click.echo(f"effort: {selected['effort']}")
    if selected["estimated_usd"] is not None:
        click.echo(f"estimated_usd: {selected['estimated_usd']}")
    click.echo(f"selection_id: {payload['selection_id']}")
