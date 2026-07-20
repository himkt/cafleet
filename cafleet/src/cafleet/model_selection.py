"""Pure domain logic for cost-aware model selection.

The model list is a Markdown document whose machine payload is three fixed
tables — ``Metadata``, ``Sources``, and ``Models`` — embedded in the ``cafleet``
skill reference page. :func:`parse_model_list_markdown` extracts, validates,
and types those tables; it performs no I/O and has no fallback source. Role
and token profiles are reviewed code constants here, not model-list data.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime, timedelta
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Mapping

SCHEMA_VERSION = 1

CAPABILITY_DIMENSIONS = ("coding", "planning", "research", "review", "monitor")

APPROVED_SOURCE_URLS = {
    "anthropic": "https://platform.claude.com/docs/en/about-claude/pricing",
    "openai": "https://developers.openai.com/api/docs/pricing",
}

_BACKENDS = frozenset({"claude", "codex", "opencode"})
_TOKEN_COMPONENTS = ("input", "cached_input", "cache_write", "output")

_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_INT_CELL_RE = re.compile(r"^-?[0-9]+$")
_PRICE_CELL_RE = re.compile(r"^[0-9]+(\.[0-9]+)?$")
_SEPARATOR_CELL_RE = re.compile(r"^:?-+:?$")

_NULL_CELL = "—"

_SECTION_HEADERS: dict[str, tuple[str, ...]] = {
    "Metadata": ("Field", "Value"),
    "Sources": ("Source", "URL", "Retrieved at", "Content SHA-256"),
    "Models": (
        "Backend",
        "Model",
        "Aliases",
        "Active",
        "Rank",
        "Cod",
        "Pln",
        "Rsc",
        "Rev",
        "Mon",
        "In",
        "Cached",
        "Write",
        "Out",
        "Max tokens",
    ),
}
_SECTION_ORDER = tuple(_SECTION_HEADERS)
_METADATA_FIELDS = ("schema_version", "generated_at", "freshness_days")


@dataclass(frozen=True)
class TokenProfile:
    input: int
    cached_input: int
    cache_write: int
    output: int


@dataclass(frozen=True)
class RoleProfile:
    task_kind: str
    requires: Mapping[str, int]
    token_profile: str


TOKEN_PROFILES = {
    "small": TokenProfile(input=4000, cached_input=0, cache_write=0, output=1000),
    "standard": TokenProfile(input=12000, cached_input=0, cache_write=0, output=6000),
    "large": TokenProfile(input=24000, cached_input=0, cache_write=0, output=12000),
}

ROLE_PROFILES = {
    "monitor": RoleProfile("monitoring", {"monitor": 2}, "small"),
    "drafter": RoleProfile(
        "design_doc_drafting", {"planning": 3, "research": 2, "review": 1}, "standard"
    ),
    "reviewer": RoleProfile("review", {"review": 4, "planning": 3}, "standard"),
    "analyzer": RoleProfile(
        "requirements_analysis", {"planning": 4, "research": 3, "review": 3}, "standard"
    ),
    "programmer": RoleProfile(
        "implementation", {"coding": 4, "planning": 3, "review": 2}, "large"
    ),
    "tester": RoleProfile(
        "test_design", {"coding": 3, "planning": 3, "review": 3}, "standard"
    ),
    "verifier": RoleProfile(
        "verification", {"coding": 3, "planning": 4, "review": 4}, "standard"
    ),
    "manager": RoleProfile(
        "research_coordination", {"planning": 4, "research": 3}, "standard"
    ),
    "scout": RoleProfile("source_discovery", {"research": 3, "planning": 2}, "small"),
    "researcher": RoleProfile(
        "research_synthesis", {"research": 4, "planning": 3, "review": 2}, "large"
    ),
    "web_researcher": RoleProfile(
        "web_research", {"research": 4, "planning": 3}, "large"
    ),
    "transcript": RoleProfile(
        "research_transcript", {"planning": 3, "research": 2, "review": 2}, "standard"
    ),
    "presentation": RoleProfile(
        "presentation_authoring",
        {"planning": 3, "research": 2, "review": 2},
        "standard",
    ),
    "visual_reviewer": RoleProfile(
        "visual_review", {"review": 4, "planning": 2}, "standard"
    ),
}


class ModelListInvalidError(ValueError):
    """The model-list tables or their cell values violate the schema contract."""


@dataclass(frozen=True)
class SourceRecord:
    url: str
    retrieved_at: datetime
    content_sha256: str


@dataclass(frozen=True)
class ModelRecord:
    key: str
    backend: str
    model: str
    aliases: tuple[str, ...]
    active: bool
    rank: int
    levels: Mapping[str, int]
    prices: Mapping[str, float | None]
    max_total_tokens: int

    @property
    def tokens(self) -> tuple[str, ...]:
        return (self.model, *self.aliases)


@dataclass(frozen=True)
class ModelList:
    schema_version: int
    generated_at: datetime
    freshness_days: int
    sources: Mapping[str, SourceRecord]
    models: tuple[ModelRecord, ...]


def parse_model_list_markdown(text: str) -> ModelList:
    """Parse and validate a model-list Markdown document into a typed model list."""
    tables = _extract_tables(text)
    metadata = _parse_metadata(tables["Metadata"])
    if metadata["schema_version"] != SCHEMA_VERSION:
        raise ModelListInvalidError(
            f"unsupported model list schema version {metadata['schema_version']}"
        )
    return ModelList(
        schema_version=SCHEMA_VERSION,
        generated_at=metadata["generated_at"],
        freshness_days=metadata["freshness_days"],
        sources=_parse_sources(tables["Sources"]),
        models=_parse_models(tables["Models"]),
    )


def _extract_tables(text: str) -> dict[str, list[tuple[str, ...]]]:
    """Split the document into the three fixed sections' table rows.

    Prose is permitted only before the first section heading; each section
    holds exactly one table: the exact expected header row, a separator row,
    then zero or more data rows.
    """
    tables: dict[str, list[tuple[str, ...]]] = {}
    current: str | None = None
    state = "rows"
    for number, raw in enumerate(text.split("\n"), start=1):
        line = raw.strip()
        if line.startswith("## "):
            if current is not None and state != "rows":
                raise ModelListInvalidError(f"section {current!r} has no table")
            title = line[3:]
            expected_index = len(tables)
            if (
                expected_index >= len(_SECTION_ORDER)
                or title != _SECTION_ORDER[expected_index]
            ):
                raise ModelListInvalidError(
                    f"line {number}: unexpected section {title!r};"
                    f" sections must be exactly {', '.join(_SECTION_ORDER)} in order"
                )
            current = title
            tables[title] = []
            state = "header"
            continue
        if not line:
            continue
        if line.startswith("|"):
            if current is None:
                raise ModelListInvalidError(
                    f"line {number}: table row outside a section"
                )
            header = _SECTION_HEADERS[current]
            cells = _split_row(line, number)
            if state == "header":
                if cells != header:
                    raise ModelListInvalidError(
                        f"line {number}: section {current!r} header must be exactly"
                        f" | {' | '.join(header)} |"
                    )
                state = "separator"
            elif state == "separator":
                if len(cells) != len(header) or not all(
                    _SEPARATOR_CELL_RE.match(cell) for cell in cells
                ):
                    raise ModelListInvalidError(
                        f"line {number}: section {current!r} is missing its separator row"
                    )
                state = "rows"
            else:
                if len(cells) != len(header):
                    raise ModelListInvalidError(
                        f"line {number}: expected {len(header)} cells, got {len(cells)}"
                    )
                tables[current].append(cells)
            continue
        if current is not None:
            raise ModelListInvalidError(
                f"line {number}: unexpected content inside section {current!r}"
            )
    if len(tables) != len(_SECTION_ORDER):
        raise ModelListInvalidError(
            f"model list must contain exactly the sections"
            f" {', '.join(_SECTION_ORDER)} in order"
        )
    if state != "rows":
        raise ModelListInvalidError(f"section {current!r} has no table")
    return tables


def _split_row(line: str, number: int) -> tuple[str, ...]:
    if not line.endswith("|") or len(line) < 2:
        raise ModelListInvalidError(
            f"line {number}: table row must start and end with '|'"
        )
    return tuple(cell.strip() for cell in line[1:-1].split("|"))


def _parse_metadata(rows: list[tuple[str, ...]]) -> dict:
    if tuple(row[0] for row in rows) != _METADATA_FIELDS:
        raise ModelListInvalidError(
            "Metadata table must have exactly the rows"
            f" {', '.join(_METADATA_FIELDS)} in order"
        )
    values = {row[0]: row[1] for row in rows}
    return {
        "schema_version": _int_cell(values["schema_version"], "schema_version"),
        "generated_at": _timestamp_cell(values["generated_at"], "generated_at"),
        "freshness_days": _int_cell(
            values["freshness_days"], "freshness_days", minimum=1
        ),
    }


def _parse_sources(rows: list[tuple[str, ...]]) -> dict[str, SourceRecord]:
    sources: dict[str, SourceRecord] = {}
    for name, url, retrieved_at, sha in rows:
        where = f"source {name!r}"
        if name in sources:
            raise ModelListInvalidError(f"duplicate {where}")
        if name not in APPROVED_SOURCE_URLS:
            raise ModelListInvalidError(f"{where} is not an approved source")
        if url != APPROVED_SOURCE_URLS[name]:
            raise ModelListInvalidError(
                f"{where} URL must be the approved URL {APPROVED_SOURCE_URLS[name]!r}"
            )
        if not _SHA256_RE.match(sha):
            raise ModelListInvalidError(
                f"{where} content SHA-256 must be 64 lowercase hex characters"
            )
        sources[name] = SourceRecord(
            url=url,
            retrieved_at=_timestamp_cell(retrieved_at, f"{where} retrieved-at"),
            content_sha256=sha,
        )
    if set(sources) != set(APPROVED_SOURCE_URLS):
        raise ModelListInvalidError(
            "Sources table must define exactly the approved sources"
            " 'anthropic' and 'openai'"
        )
    return sources


def _parse_models(rows: list[tuple[str, ...]]) -> tuple[ModelRecord, ...]:
    if not rows:
        raise ModelListInvalidError("Models table must not be empty")
    models = []
    seen_keys: set[str] = set()
    seen_ranks: set[int] = set()
    seen_tokens: set[tuple[str, str]] = set()
    for position, row in enumerate(rows, start=1):
        backend, model, aliases_cell, active, rank = row[0:5]
        where = f"Models row {position} ({model!r})"
        if backend not in _BACKENDS:
            raise ModelListInvalidError(f"{where}: unknown backend {backend!r}")
        if not model:
            raise ModelListInvalidError(f"Models row {position}: empty Model cell")
        aliases = (
            ()
            if aliases_cell == _NULL_CELL
            else tuple(alias.strip() for alias in aliases_cell.split(","))
        )
        if any(not alias for alias in aliases):
            raise ModelListInvalidError(f"{where}: empty alias")
        record = ModelRecord(
            key=f"{backend}:{model}",
            backend=backend,
            model=model,
            aliases=aliases,
            active=_bool_cell(active, f"{where}: Active"),
            rank=_int_cell(rank, f"{where}: Rank"),
            levels={
                dimension: _int_cell(
                    cell, f"{where}: {dimension}", minimum=0, maximum=5
                )
                for dimension, cell in zip(
                    CAPABILITY_DIMENSIONS, row[5:10], strict=True
                )
            },
            prices={
                component: _price_cell(cell, f"{where}: {component}")
                for component, cell in zip(_TOKEN_COMPONENTS, row[10:14], strict=True)
            },
            max_total_tokens=_int_cell(row[14], f"{where}: Max tokens", minimum=1),
        )
        if record.key in seen_keys:
            raise ModelListInvalidError(f"duplicate model key {record.key!r}")
        if record.rank in seen_ranks:
            raise ModelListInvalidError(
                f"duplicate rank {record.rank} on {record.key!r}"
            )
        seen_keys.add(record.key)
        seen_ranks.add(record.rank)
        for token in record.tokens:
            pair = (backend, token)
            if pair in seen_tokens:
                raise ModelListInvalidError(
                    f"duplicate model token {token!r} for backend {backend!r}"
                )
            seen_tokens.add(pair)
        models.append(record)
    return tuple(models)


def _int_cell(
    cell: str, where: str, *, minimum: int | None = None, maximum: int | None = None
) -> int:
    if not _INT_CELL_RE.match(cell):
        raise ModelListInvalidError(f"{where} must be an integer, got {cell!r}")
    value = int(cell)
    if minimum is not None and value < minimum:
        raise ModelListInvalidError(f"{where} must be >= {minimum}, got {value}")
    if maximum is not None and value > maximum:
        raise ModelListInvalidError(f"{where} must be <= {maximum}, got {value}")
    return value


def _price_cell(cell: str, where: str) -> float | None:
    if cell == _NULL_CELL:
        return None
    if not _PRICE_CELL_RE.match(cell):
        raise ModelListInvalidError(
            f"{where} must be a non-negative USD-per-MTok number or"
            f" {_NULL_CELL!r}, got {cell!r}"
        )
    return float(cell)


def _bool_cell(cell: str, where: str) -> bool:
    if cell == "yes":
        return True
    if cell == "no":
        return False
    raise ModelListInvalidError(f"{where} must be 'yes' or 'no', got {cell!r}")


def _timestamp_cell(cell: str, where: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(cell)
    except ValueError as exc:
        raise ModelListInvalidError(
            f"{where} is not a valid ISO 8601 timestamp: {cell!r}"
        ) from exc
    if parsed.utcoffset() is None:
        raise ModelListInvalidError(f"{where} must be timezone-aware: {cell!r}")
    return parsed


class ModelSelectionError(Exception):
    """A selection request failed; ``code`` is the stable error-contract code.

    ``candidates`` carries the examined candidate records (with exclusion
    reasons) when candidate enumeration occurred before the failure.
    """

    def __init__(
        self,
        code: str,
        message: str,
        *,
        candidates: tuple[CandidateRecord, ...] = (),
    ) -> None:
        super().__init__(message)
        self.code = code
        self.candidates = candidates


@dataclass(frozen=True)
class TokenEstimate:
    input: int
    cached_input: int
    cache_write: int
    output: int


@dataclass(frozen=True)
class CandidateRecord:
    key: str
    eligible: bool
    reason: str | None
    estimated_usd: float | None


@dataclass(frozen=True)
class SelectedModel:
    key: str
    backend: str
    model: str
    effort: str | None
    estimated_usd: float


@dataclass(frozen=True)
class SelectionResult:
    policy: str
    role: str
    task_profile: Mapping[str, int]
    token_estimate: TokenEstimate
    candidates: tuple[CandidateRecord, ...]
    selected: SelectedModel


@dataclass(frozen=True)
class ManualOverrideResult:
    policy: str
    backend: str | None
    model: str
    model_key: str | None
    estimated_usd: float | None
    estimate_status: str


def select_model(
    model_list: ModelList,
    *,
    role: str,
    ready_backends: frozenset[str],
    now: datetime,
    requires: Mapping[str, int] | None = None,
    token_estimate: Mapping[str, int] | None = None,
    backend: str | None = None,
) -> SelectionResult:
    """Deterministically select a model for ``role`` per the selection policies.

    Ordinary roles minimize estimated USD cost among capability-eligible models;
    ``monitor`` selects the least-cost monitor-capable model; ``reviewer`` selects
    the highest rank among reviewer-capable models.
    """
    task_profile = _resolve_task_profile(role, requires)
    estimate = _resolve_token_estimate(
        token_estimate, TOKEN_PROFILES[ROLE_PROFILES[role].token_profile]
    )
    if backend is not None and backend not in _BACKENDS:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown backend override {backend!r}"
        )
    _require_fresh(model_list, now)
    requested = _BACKENDS if backend is None else frozenset({backend})
    candidate_backends = requested & ready_backends
    if not candidate_backends:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    candidates, eligible = _enumerate_candidates(
        model_list, candidate_backends, task_profile, estimate
    )
    if not eligible:
        raise ModelSelectionError(
            "MODEL_NO_ELIGIBLE_CANDIDATE",
            f"no listed candidate meets every constraint for role {role!r}",
            candidates=candidates,
        )
    if role == "reviewer":
        policy = "reviewer_maximum_capability"
        order = _reviewer_order
    elif role == "monitor":
        policy = "monitor_minimum_cost"
        order = _cost_order
    else:
        policy = "cost_minimized_subject_to_capability"
        order = _cost_order
    best_model, best_cost = min(eligible, key=order)
    return SelectionResult(
        policy=policy,
        role=role,
        task_profile=task_profile,
        token_estimate=estimate,
        candidates=candidates,
        selected=_selected(best_model, best_cost),
    )


def select_replacement(
    model_list: ModelList,
    *,
    role: str,
    failed_model_key: str,
    failed_dimensions: list[str] | tuple[str, ...],
    ready_backends: frozenset[str],
    now: datetime,
    requires: Mapping[str, int] | None = None,
    token_estimate: Mapping[str, int] | None = None,
    attempted_model_keys: frozenset[str] = frozenset(),
) -> SelectionResult:
    """Select a strictly stronger replacement after an underpowered-member failure.

    Raises each failed capability floor by one, requires a strictly greater
    rank than the failed model, skips already-attempted models, and minimizes
    cost only within that stronger eligible set.
    """
    task_profile = _resolve_task_profile(role, requires)
    models_by_key = {model.key: model for model in model_list.models}
    if failed_model_key not in models_by_key:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"failed model {failed_model_key!r} is not a listed model",
        )
    failed_rank = models_by_key[failed_model_key].rank
    for dimension in failed_dimensions:
        if dimension not in CAPABILITY_DIMENSIONS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown failed capability dimension {dimension!r}",
            )
        task_profile[dimension] = min(5, task_profile.get(dimension, 0) + 1)
    estimate = _resolve_token_estimate(
        token_estimate, TOKEN_PROFILES[ROLE_PROFILES[role].token_profile]
    )
    _require_fresh(model_list, now)
    candidate_backends = _BACKENDS & ready_backends
    if not candidate_backends:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    candidates, eligible = _enumerate_candidates(
        model_list,
        candidate_backends,
        task_profile,
        estimate,
        min_rank_exclusive=failed_rank,
        excluded_keys=attempted_model_keys | {failed_model_key},
    )
    if not eligible:
        raise ModelSelectionError(
            "MODEL_UPGRADE_UNAVAILABLE",
            f"no strictly stronger eligible replacement for {failed_model_key!r}",
            candidates=candidates,
        )
    best_model, best_cost = min(eligible, key=_cost_order)
    return SelectionResult(
        policy="replacement_upgrade",
        role=role,
        task_profile=task_profile,
        token_estimate=estimate,
        candidates=candidates,
        selected=_selected(best_model, best_cost),
    )


def resolve_manual_override(
    model_list: ModelList,
    *,
    model: str,
    now: datetime,
    backend: str | None = None,
    token_estimate: Mapping[str, int] | None = None,
) -> ManualOverrideResult:
    """Resolve an explicit model pin through the active models' token/alias sets.

    A mapped token fixes the backend and yields an estimate only when the model
    list is fresh, the model is fully priced for the estimate, and a token
    estimate was given; an unmapped token stays permitted for the manual spawn
    path with ``estimate_status: "unavailable"``. A backend pin that conflicts
    with the token's mapped backend is rejected.
    """
    if backend is not None and backend not in _BACKENDS:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown backend override {backend!r}"
        )
    matches = [
        record
        for record in model_list.models
        if record.active and model in record.tokens
    ]
    if backend is not None:
        backend_matches = [record for record in matches if record.backend == backend]
        if not backend_matches and matches:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"model {model!r} is mapped to backend"
                f" {matches[0].backend!r}, not {backend!r}",
            )
        matches = backend_matches
    if len({record.key for record in matches}) > 1:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"model {model!r} is ambiguous across backends; pass an explicit backend",
        )
    if not matches:
        return ManualOverrideResult(
            policy="manual_override",
            backend=backend,
            model=model,
            model_key=None,
            estimated_usd=None,
            estimate_status="unavailable",
        )
    record = matches[0]
    estimated_usd = None
    if token_estimate is not None and _stale_reason(model_list, now) is None:
        estimate = _resolve_token_estimate(token_estimate, None)
        estimated_usd, _ = _model_cost(record, estimate)
    return ManualOverrideResult(
        policy="manual_override",
        backend=record.backend,
        model=record.model,
        model_key=record.key,
        estimated_usd=estimated_usd,
        estimate_status="ok" if estimated_usd is not None else "unavailable",
    )


def _resolve_task_profile(
    role: str, requires: Mapping[str, int] | None
) -> dict[str, int]:
    if role not in ROLE_PROFILES:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown role {role!r}"
        )
    merged = dict(ROLE_PROFILES[role].requires)
    for dimension, level in (requires or {}).items():
        if dimension not in CAPABILITY_DIMENSIONS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown capability dimension {dimension!r}",
            )
        if not isinstance(level, int) or isinstance(level, bool) or not 1 <= level <= 5:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"required level for {dimension!r} must be an integer in 1..5",
            )
        floor = merged.get(dimension, 0)
        if level < floor:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"required level {level} for {dimension!r} is below the"
                f" role-profile floor {floor}",
            )
        merged[dimension] = level
    return merged


def _resolve_token_estimate(
    token_estimate: Mapping[str, int] | None, profile: TokenProfile | None
) -> TokenEstimate:
    components = (
        {name: getattr(profile, name) for name in _TOKEN_COMPONENTS}
        if profile is not None
        else dict.fromkeys(_TOKEN_COMPONENTS, 0)
    )
    for name, value in (token_estimate or {}).items():
        if name not in _TOKEN_COMPONENTS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown token-estimate component {name!r}",
            )
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"token estimate for {name!r} must be a non-negative integer",
            )
        components[name] = value
    return TokenEstimate(**components)


def _stale_reason(model_list: ModelList, now: datetime) -> str | None:
    for name, source in model_list.sources.items():
        age = now - source.retrieved_at
        if age > timedelta(days=model_list.freshness_days):
            return (
                f"source {name!r} was retrieved more than"
                f" {model_list.freshness_days} days before selection time"
            )
        if -age > timedelta(minutes=5):
            return f"source {name!r} was retrieved more than five minutes after selection time"
    return None


def _require_fresh(model_list: ModelList, now: datetime) -> None:
    reason = _stale_reason(model_list, now)
    if reason is not None:
        raise ModelSelectionError("MODEL_LIST_STALE", reason)


def _enumerate_candidates(
    model_list: ModelList,
    candidate_backends: frozenset[str],
    task_profile: Mapping[str, int],
    estimate: TokenEstimate,
    *,
    min_rank_exclusive: int | None = None,
    excluded_keys: frozenset[str] = frozenset(),
) -> tuple[tuple[CandidateRecord, ...], list[tuple[ModelRecord, float]]]:
    records = []
    eligible = []
    for model in model_list.models:
        cost = None
        if not model.active:
            reason = "inactive model"
        elif model.backend not in candidate_backends:
            reason = f"backend {model.backend!r} is not a ready requested backend"
        elif model.key in excluded_keys:
            reason = "model already attempted for this task"
        elif min_rank_exclusive is not None and model.rank <= min_rank_exclusive:
            reason = (
                f"rank {model.rank} is not strictly"
                f" greater than the failed model's {min_rank_exclusive}"
            )
        else:
            reason = _capability_shortfall(model, task_profile)
            if reason is None:
                cost, reason = _model_cost(model, estimate)
        records.append(
            CandidateRecord(
                key=model.key,
                eligible=reason is None,
                reason=reason,
                estimated_usd=cost,
            )
        )
        if reason is None and cost is not None:
            eligible.append((model, cost))
    return tuple(records), eligible


def _capability_shortfall(
    model: ModelRecord, task_profile: Mapping[str, int]
) -> str | None:
    for dimension in CAPABILITY_DIMENSIONS:
        required = task_profile.get(dimension)
        if required is None:
            continue
        level = model.levels[dimension]
        if level < required:
            return f"{dimension} capability {level} < {required}"
    return None


def _model_cost(
    model: ModelRecord, estimate: TokenEstimate
) -> tuple[float | None, str | None]:
    if all(price is None for price in model.prices.values()):
        return None, "model has no approved prices"
    total = (
        estimate.input + estimate.cached_input + estimate.cache_write + estimate.output
    )
    if total > model.max_total_tokens:
        return None, "token estimate exceeds the model's max total tokens"
    cost = 0.0
    for name in _TOKEN_COMPONENTS:
        tokens = getattr(estimate, name)
        price = model.prices[name]
        if price is None:
            if tokens:
                return None, f"{name} tokens are requested but unpriced"
            continue
        cost += tokens / 1_000_000 * price
    return cost, None


def _cost_order(candidate: tuple[ModelRecord, float]) -> tuple[float, int, str]:
    model, cost = candidate
    return (cost, -model.rank, model.key)


def _reviewer_order(candidate: tuple[ModelRecord, float]) -> tuple[int, float, str]:
    model, cost = candidate
    return (-model.rank, cost, model.key)


def _selected(model: ModelRecord, cost: float) -> SelectedModel:
    return SelectedModel(
        key=model.key,
        backend=model.backend,
        model=model.model,
        effort=None,
        estimated_usd=cost,
    )
