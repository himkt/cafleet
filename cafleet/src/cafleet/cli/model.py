"""``cafleet model`` — cost-aware model selection against the deployed catalog."""

import uuid
from datetime import UTC, datetime
from pathlib import Path

import click

from cafleet import output
from cafleet.cli._helpers import ensure_assets_current, json_flag
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
    ensure_assets_current()
    now = datetime.now(UTC)
    token_estimate = _token_estimate_from_flags(
        input_tokens, cached_input_tokens, cache_write_tokens, output_tokens
    )
    try:
        catalog_path = _validated_catalog_path(catalog_arg)
        try:
            catalog = parse_catalog_markdown(catalog_path.read_bytes().decode("utf-8"))
        except (CatalogInvalidError, UnicodeDecodeError) as exc:
            raise ModelSelectionError("MODEL_CATALOG_INVALID", str(exc)) from exc
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
            payload = _manual_payload(
                result, effort, triggered_by, str(catalog_path), now
            )
        else:
            if role is None:
                raise ModelSelectionError(
                    "MODEL_SELECTION_INVALID_REQUEST",
                    "either --role or --model is required",
                )
            requires = _parse_requires(requires_args)
            result = select_model(
                catalog,
                role=role,
                ready_backends=_ready_backends(coding_agent),
                now=now,
                requires=requires,
                token_estimate=token_estimate,
                backend=coding_agent,
            )
            _validate_effort(result.selected.backend, effort)
            payload = _selection_payload(
                result,
                catalog,
                effort,
                triggered_by,
                token_estimate is not None,
                str(catalog_path),
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


def _ready_backends(coding_agent: str | None) -> frozenset[str]:
    """Resolve the runtime-ready candidate backend set via each backend's
    existing readiness contract."""
    requested = (
        frozenset(CODING_AGENTS) if coding_agent is None else frozenset({coding_agent})
    )
    ready = set()
    for backend in sorted(requested):
        try:
            CODING_AGENTS[backend].ensure_available()
        except RuntimeError:
            continue
        ready.add(backend)
    if not ready:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    return frozenset(ready)


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
    catalog_path: str,
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
        "catalog_path": catalog_path,
        "spawn": {"state": "pending", "member_id": None, "error": None},
    }


def _manual_payload(
    result: ManualOverrideResult,
    effort: str | None,
    triggered_by: str | None,
    catalog_path: str,
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
        "catalog_path": catalog_path,
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
