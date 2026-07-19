"""Pure domain logic for cost-aware model selection.

The model catalog is a Markdown document whose sole machine payload is one
canonical-JSON fenced block introduced by the ``<!-- cafleet-model-catalog: v1 -->``
sentinel line. :func:`parse_catalog_markdown` extracts, validates, and types that
payload; it performs no I/O and has no fallback catalog source.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping
    from typing import TypeGuard

SCHEMA_VERSION = 1

CAPABILITY_DIMENSIONS = ("coding", "planning", "research", "review", "monitor")

APPROVED_SOURCE_URLS = {
    "anthropic": "https://platform.claude.com/docs/en/about-claude/pricing",
    "openai": "https://developers.openai.com/api/docs/pricing",
}

ROLE_TASK_KINDS = {
    "monitor": "monitoring",
    "drafter": "design_doc_drafting",
    "reviewer": "review",
    "analyzer": "requirements_analysis",
    "programmer": "implementation",
    "tester": "test_design",
    "verifier": "verification",
    "manager": "research_coordination",
    "scout": "source_discovery",
    "researcher": "research_synthesis",
    "web_researcher": "web_research",
    "transcript": "research_transcript",
    "presentation": "presentation_authoring",
    "visual_reviewer": "visual_review",
}

_BACKENDS = frozenset({"claude", "codex", "opencode"})
_PROVIDERS = frozenset({"anthropic", "openai"})
_TOKEN_PROFILE_NAMES = frozenset({"small", "standard", "large"})
_TOKEN_COMPONENTS = ("input", "cached_input", "cache_write", "output")
_RATE_CARD_STATUSES = frozenset({"known", "unknown", "not-applicable"})
_MAINTAINER_JUDGMENT = "maintainer_judgment"

_MARKER_RE = re.compile(r"^<!-- cafleet-model-catalog: (?P<version>\S+) -->$")
_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_PAYLOAD_FENCE = "```json"

_MIGRATIONS: dict[int, Callable[[dict], dict]] = {}


class CatalogInvalidError(ValueError):
    """The catalog Markdown envelope or its JSON payload violates the schema contract."""


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


@dataclass(frozen=True)
class SourceRecord:
    url: str
    retrieved_at: datetime
    content_sha256: str


@dataclass(frozen=True)
class CapabilityProvenance:
    type: str
    rationale: str
    reviewed_at: datetime


@dataclass(frozen=True)
class Capability:
    global_rank: int
    levels: Mapping[str, int]
    provenance: CapabilityProvenance


@dataclass(frozen=True)
class RateCardComponent:
    mode: str
    usd_per_mtok: float | None


@dataclass(frozen=True)
class RateCard:
    id: str
    status: str
    max_total_tokens: int
    components: Mapping[str, RateCardComponent]
    effective_from: date
    effective_until: date | None
    pricing_source: str


@dataclass(frozen=True)
class ModelAvailability:
    requires_backend: str


@dataclass(frozen=True)
class CatalogModel:
    key: str
    backend: str
    provider_sku: str
    provider: str
    active: bool
    capability: Capability
    rate_cards: tuple[RateCard, ...]
    availability: ModelAvailability


@dataclass(frozen=True)
class ModelTokenEntry:
    backend: str
    token: str
    model_key: str
    primary: bool


@dataclass(frozen=True)
class Catalog:
    schema_version: int
    generated_at: datetime
    freshness_days: int
    currency: str
    token_profiles: Mapping[str, TokenProfile]
    role_profiles: Mapping[str, RoleProfile]
    sources: Mapping[str, SourceRecord]
    models: tuple[CatalogModel, ...]
    model_tokens: tuple[ModelTokenEntry, ...]


def parse_catalog_markdown(text: str) -> Catalog:
    """Parse and validate a model-catalog Markdown document into a typed catalog."""
    payload_text = _extract_payload(text)
    try:
        data = json.loads(payload_text)
    except json.JSONDecodeError as exc:
        raise CatalogInvalidError(f"catalog payload is not valid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise CatalogInvalidError("catalog payload must be a JSON object")
    canonical = json.dumps(data, sort_keys=True, indent=2, ensure_ascii=False) + "\n"
    if payload_text != canonical:
        raise CatalogInvalidError(
            "catalog payload is not canonical JSON"
            " (sorted keys, two-space indentation, terminating newline)"
        )
    return _validate_catalog(_migrate(data))


def _extract_payload(text: str) -> str:
    lines = text.split("\n")
    markers = [
        (index, match.group("version"))
        for index, line in enumerate(lines)
        if (match := _MARKER_RE.match(line))
    ]
    if len(markers) != 1:
        raise CatalogInvalidError(
            f"catalog must contain exactly one sentinel marker line, found {len(markers)}"
        )
    marker_index, marker_version = markers[0]
    if marker_version != f"v{SCHEMA_VERSION}":
        raise CatalogInvalidError(
            f"unsupported catalog marker version {marker_version!r}"
        )
    payload_fences = [
        index for index, line in enumerate(lines) if line == _PAYLOAD_FENCE
    ]
    if len(payload_fences) != 1:
        raise CatalogInvalidError(
            f"catalog must contain exactly one ```json payload block, found {len(payload_fences)}"
        )
    if payload_fences[0] != marker_index + 1:
        raise CatalogInvalidError(
            "the sentinel marker must be immediately followed by the ```json payload block"
        )
    closing = next(
        (
            index
            for index in range(marker_index + 2, len(lines))
            if lines[index] == "```"
        ),
        None,
    )
    if closing is None:
        raise CatalogInvalidError("the ```json payload block is not closed")
    return "\n".join(lines[marker_index + 2 : closing]) + "\n"


def _migrate(data: dict) -> dict:
    version = data.get("schema_version")
    if not _is_int(version):
        raise CatalogInvalidError("schema_version must be an integer")
    while version != SCHEMA_VERSION:
        migration = _MIGRATIONS.get(version)
        if migration is None:
            raise CatalogInvalidError(f"unsupported catalog schema version {version}")
        data = migration(data)
        version = data["schema_version"]
    return data


def _validate_catalog(data: dict) -> Catalog:
    _require_keys(
        data,
        {
            "schema_version",
            "generated_at",
            "freshness_days",
            "currency",
            "token_profiles",
            "role_profiles",
            "sources",
            "models",
            "model_tokens",
        },
        "catalog",
    )
    generated_at = _parse_timestamp(data["generated_at"], "generated_at")
    freshness_days = _validate_int(data["freshness_days"], "freshness_days", minimum=1)
    if data["currency"] != "USD":
        raise CatalogInvalidError("currency must be 'USD'")
    token_profiles = _validate_token_profiles(data["token_profiles"])
    role_profiles = _validate_role_profiles(data["role_profiles"], token_profiles)
    sources = _validate_sources(data["sources"])
    models = _validate_models(data["models"], frozenset(sources))
    model_tokens = _validate_model_tokens(data["model_tokens"], models)
    return Catalog(
        schema_version=SCHEMA_VERSION,
        generated_at=generated_at,
        freshness_days=freshness_days,
        currency="USD",
        token_profiles=token_profiles,
        role_profiles=role_profiles,
        sources=sources,
        models=models,
        model_tokens=model_tokens,
    )


def _validate_token_profiles(raw: object) -> dict[str, TokenProfile]:
    profiles = _require_dict(raw, "token_profiles")
    if set(profiles) != _TOKEN_PROFILE_NAMES:
        raise CatalogInvalidError(
            "token_profiles must define exactly the profiles 'small', 'standard', and 'large'"
        )
    validated = {}
    for name, profile in profiles.items():
        where = f"token_profiles.{name}"
        components = _require_dict(profile, where)
        _require_keys(components, set(_TOKEN_COMPONENTS), where)
        validated[name] = TokenProfile(
            **{
                component: _validate_int(
                    components[component], f"{where}.{component}", minimum=0
                )
                for component in _TOKEN_COMPONENTS
            }
        )
    return validated


def _validate_role_profiles(
    raw: object, token_profiles: Mapping[str, TokenProfile]
) -> dict[str, RoleProfile]:
    profiles = _require_dict(raw, "role_profiles")
    if not profiles:
        raise CatalogInvalidError("role_profiles must not be empty")
    validated = {}
    for role, profile in profiles.items():
        where = f"role_profiles.{role}"
        if role not in ROLE_TASK_KINDS:
            raise CatalogInvalidError(f"unknown role-profile key {role!r}")
        fields = _require_dict(profile, where)
        _require_keys(fields, {"task_kind", "requires", "token_profile"}, where)
        if fields["task_kind"] != ROLE_TASK_KINDS[role]:
            raise CatalogInvalidError(
                f"{where}.task_kind must be {ROLE_TASK_KINDS[role]!r},"
                f" got {fields['task_kind']!r}"
            )
        requires = _require_dict(fields["requires"], f"{where}.requires")
        if not requires:
            raise CatalogInvalidError(f"{where}.requires must not be empty")
        for dimension, level in requires.items():
            if dimension not in CAPABILITY_DIMENSIONS:
                raise CatalogInvalidError(
                    f"{where}.requires has unknown capability dimension {dimension!r}"
                )
            _validate_int(level, f"{where}.requires.{dimension}", minimum=1, maximum=5)
        if fields["token_profile"] not in token_profiles:
            raise CatalogInvalidError(
                f"{where}.token_profile {fields['token_profile']!r} is not a defined token profile"
            )
        validated[role] = RoleProfile(
            task_kind=fields["task_kind"],
            requires=dict(requires),
            token_profile=fields["token_profile"],
        )
    return validated


def _validate_sources(raw: object) -> dict[str, SourceRecord]:
    sources = _require_dict(raw, "sources")
    if set(sources) != set(APPROVED_SOURCE_URLS):
        raise CatalogInvalidError(
            "sources must define exactly the approved sources 'anthropic' and 'openai'"
        )
    validated = {}
    for name, source in sources.items():
        where = f"sources.{name}"
        fields = _require_dict(source, where)
        _require_keys(fields, {"url", "retrieved_at", "content_sha256"}, where)
        if fields["url"] != APPROVED_SOURCE_URLS[name]:
            raise CatalogInvalidError(
                f"{where}.url must be the approved URL {APPROVED_SOURCE_URLS[name]!r}"
            )
        sha = fields["content_sha256"]
        if not isinstance(sha, str) or not _SHA256_RE.match(sha):
            raise CatalogInvalidError(
                f"{where}.content_sha256 must be 64 lowercase hex characters"
            )
        validated[name] = SourceRecord(
            url=fields["url"],
            retrieved_at=_parse_timestamp(
                fields["retrieved_at"], f"{where}.retrieved_at"
            ),
            content_sha256=sha,
        )
    return validated


def _validate_models(
    raw: object, source_names: frozenset[str]
) -> tuple[CatalogModel, ...]:
    entries = _require_list(raw, "models")
    if not entries:
        raise CatalogInvalidError("models must not be empty")
    models = []
    seen_keys: set[str] = set()
    seen_skus: set[str] = set()
    seen_ranks: set[int] = set()
    for position, entry in enumerate(entries):
        model = _validate_model(entry, source_names, f"models[{position}]")
        if model.key in seen_keys:
            raise CatalogInvalidError(f"duplicate model key {model.key!r}")
        if model.provider_sku in seen_skus:
            raise CatalogInvalidError(f"duplicate provider SKU {model.provider_sku!r}")
        if model.capability.global_rank in seen_ranks:
            raise CatalogInvalidError(
                f"duplicate global_rank {model.capability.global_rank} on {model.key!r}"
            )
        seen_keys.add(model.key)
        seen_skus.add(model.provider_sku)
        seen_ranks.add(model.capability.global_rank)
        models.append(model)
    return tuple(models)


def _validate_model(
    raw: object, source_names: frozenset[str], where: str
) -> CatalogModel:
    fields = _require_dict(raw, where)
    _require_keys(
        fields,
        {
            "key",
            "backend",
            "provider_sku",
            "provider",
            "active",
            "capability",
            "rate_cards",
            "availability",
        },
        where,
    )
    backend = fields["backend"]
    if backend not in _BACKENDS:
        raise CatalogInvalidError(f"{where}.backend {backend!r} is not a known backend")
    if fields["provider"] not in _PROVIDERS:
        raise CatalogInvalidError(
            f"{where}.provider {fields['provider']!r} is not a known provider"
        )
    provider_sku = _require_str(fields["provider_sku"], f"{where}.provider_sku")
    key = _require_str(fields["key"], f"{where}.key")
    if key != f"{backend}:{provider_sku}":
        raise CatalogInvalidError(
            f"{where}.key must be '<backend>:<provider_sku>' ({backend}:{provider_sku}), got {key!r}"
        )
    if not isinstance(fields["active"], bool):
        raise CatalogInvalidError(f"{where}.active must be a boolean")
    availability = _require_dict(fields["availability"], f"{where}.availability")
    _require_keys(availability, {"requires_backend"}, f"{where}.availability")
    if availability["requires_backend"] != backend:
        raise CatalogInvalidError(
            f"{where}.availability.requires_backend must equal the model backend {backend!r}"
        )
    return CatalogModel(
        key=key,
        backend=backend,
        provider_sku=provider_sku,
        provider=fields["provider"],
        active=fields["active"],
        capability=_validate_capability(fields["capability"], f"{where}.capability"),
        rate_cards=_validate_rate_cards(
            fields["rate_cards"], source_names, f"{where}.rate_cards"
        ),
        availability=ModelAvailability(
            requires_backend=availability["requires_backend"]
        ),
    )


def _validate_capability(raw: object, where: str) -> Capability:
    fields = _require_dict(raw, where)
    _require_keys(fields, {"global_rank", "levels", "provenance"}, where)
    levels = _require_dict(fields["levels"], f"{where}.levels")
    _require_keys(levels, set(CAPABILITY_DIMENSIONS), f"{where}.levels")
    for dimension in CAPABILITY_DIMENSIONS:
        _validate_int(
            levels[dimension], f"{where}.levels.{dimension}", minimum=0, maximum=5
        )
    provenance = _require_dict(fields["provenance"], f"{where}.provenance")
    _require_keys(
        provenance, {"type", "rationale", "reviewed_at"}, f"{where}.provenance"
    )
    if provenance["type"] != _MAINTAINER_JUDGMENT:
        raise CatalogInvalidError(
            f"{where}.provenance.type must be {_MAINTAINER_JUDGMENT!r},"
            f" got {provenance['type']!r}"
        )
    return Capability(
        global_rank=_validate_int(fields["global_rank"], f"{where}.global_rank"),
        levels=dict(levels),
        provenance=CapabilityProvenance(
            type=provenance["type"],
            rationale=_require_str(
                provenance["rationale"], f"{where}.provenance.rationale"
            ),
            reviewed_at=_parse_timestamp(
                provenance["reviewed_at"], f"{where}.provenance.reviewed_at"
            ),
        ),
    )


def _validate_rate_cards(
    raw: object, source_names: frozenset[str], where: str
) -> tuple[RateCard, ...]:
    entries = _require_list(raw, where)
    cards = []
    seen_ids: set[str] = set()
    for position, entry in enumerate(entries):
        card = _validate_rate_card(entry, source_names, f"{where}[{position}]")
        if card.id in seen_ids:
            raise CatalogInvalidError(f"duplicate rate-card id {card.id!r} in {where}")
        seen_ids.add(card.id)
        cards.append(card)
    return tuple(cards)


def _validate_rate_card(
    raw: object, source_names: frozenset[str], where: str
) -> RateCard:
    fields = _require_dict(raw, where)
    _require_keys(
        fields,
        {
            "id",
            "status",
            "max_total_tokens",
            "components",
            "effective_from",
            "effective_until",
            "pricing_source",
        },
        where,
    )
    if fields["status"] not in _RATE_CARD_STATUSES:
        raise CatalogInvalidError(
            f"{where}.status {fields['status']!r} is not a known status"
        )
    if fields["pricing_source"] not in source_names:
        raise CatalogInvalidError(
            f"{where}.pricing_source {fields['pricing_source']!r} is not a defined source"
        )
    components_raw = _require_dict(fields["components"], f"{where}.components")
    _require_keys(components_raw, set(_TOKEN_COMPONENTS), f"{where}.components")
    components = {
        component: _validate_component(
            components_raw[component], f"{where}.components.{component}"
        )
        for component in _TOKEN_COMPONENTS
    }
    effective_until = (
        None
        if fields["effective_until"] is None
        else _parse_date(fields["effective_until"], f"{where}.effective_until")
    )
    return RateCard(
        id=_require_str(fields["id"], f"{where}.id"),
        status=fields["status"],
        max_total_tokens=_validate_int(
            fields["max_total_tokens"], f"{where}.max_total_tokens", minimum=1
        ),
        components=components,
        effective_from=_parse_date(fields["effective_from"], f"{where}.effective_from"),
        effective_until=effective_until,
        pricing_source=fields["pricing_source"],
    )


def _validate_component(raw: object, where: str) -> RateCardComponent:
    fields = _require_dict(raw, where)
    _require_keys(fields, {"mode", "usd_per_mtok"}, where)
    mode = fields["mode"]
    price = fields["usd_per_mtok"]
    if mode == "supported":
        if isinstance(price, bool) or not isinstance(price, (int, float)) or price < 0:
            raise CatalogInvalidError(
                f"{where}.usd_per_mtok must be a non-negative number for a supported component"
            )
        return RateCardComponent(mode=mode, usd_per_mtok=float(price))
    if mode == "unsupported":
        if price is not None:
            raise CatalogInvalidError(
                f"{where}.usd_per_mtok must be null for an unsupported component"
            )
        return RateCardComponent(mode=mode, usd_per_mtok=None)
    raise CatalogInvalidError(f"{where}.mode {mode!r} is not a known component mode")


def _validate_model_tokens(
    raw: object, models: tuple[CatalogModel, ...]
) -> tuple[ModelTokenEntry, ...]:
    entries = _require_list(raw, "model_tokens")
    models_by_key = {model.key: model for model in models}
    validated = []
    seen_pairs: set[tuple[str, str]] = set()
    primary_keys: set[str] = set()
    for position, entry in enumerate(entries):
        where = f"model_tokens[{position}]"
        fields = _require_dict(entry, where)
        _require_keys(fields, {"backend", "token", "model_key", "primary"}, where)
        backend = fields["backend"]
        if backend not in _BACKENDS:
            raise CatalogInvalidError(
                f"{where}.backend {backend!r} is not a known backend"
            )
        token = _require_str(fields["token"], f"{where}.token")
        model_key = fields["model_key"]
        if model_key not in models_by_key:
            raise CatalogInvalidError(
                f"{where}.model_key {model_key!r} is not a catalog model"
            )
        model = models_by_key[model_key]
        if not model.active:
            raise CatalogInvalidError(
                f"{where}.model_key {model_key!r} is not an active model"
            )
        if model.backend != backend:
            raise CatalogInvalidError(
                f"{where}.backend {backend!r} does not match the backend of {model_key!r}"
            )
        if not isinstance(fields["primary"], bool):
            raise CatalogInvalidError(f"{where}.primary must be a boolean")
        pair = (backend, token)
        if pair in seen_pairs:
            raise CatalogInvalidError(
                f"duplicate model token {token!r} for backend {backend!r}"
            )
        seen_pairs.add(pair)
        if fields["primary"]:
            if model_key in primary_keys:
                raise CatalogInvalidError(
                    f"model {model_key!r} has more than one primary token"
                )
            primary_keys.add(model_key)
        validated.append(
            ModelTokenEntry(
                backend=backend,
                token=token,
                model_key=model_key,
                primary=fields["primary"],
            )
        )
    for model in models:
        if model.active and model.key not in primary_keys:
            raise CatalogInvalidError(
                f"active model {model.key!r} has no primary token"
            )
    return tuple(validated)


def _require_dict(value: object, where: str) -> dict:
    if not isinstance(value, dict):
        raise CatalogInvalidError(f"{where} must be a JSON object")
    return value


def _require_list(value: object, where: str) -> list:
    if not isinstance(value, list):
        raise CatalogInvalidError(f"{where} must be a JSON array")
    return value


def _require_keys(fields: dict, expected: set[str], where: str) -> None:
    if set(fields) != expected:
        missing = expected - set(fields)
        unexpected = set(fields) - expected
        raise CatalogInvalidError(
            f"{where} has wrong keys (missing: {sorted(missing)}, unexpected: {sorted(unexpected)})"
        )


def _require_str(value: object, where: str) -> str:
    if not isinstance(value, str) or not value:
        raise CatalogInvalidError(f"{where} must be a non-empty string")
    return value


def _is_int(value: object) -> TypeGuard[int]:
    return isinstance(value, int) and not isinstance(value, bool)


def _validate_int(
    value: object, where: str, *, minimum: int | None = None, maximum: int | None = None
) -> int:
    if not _is_int(value):
        raise CatalogInvalidError(f"{where} must be an integer")
    if minimum is not None and value < minimum:
        raise CatalogInvalidError(f"{where} must be >= {minimum}")
    if maximum is not None and value > maximum:
        raise CatalogInvalidError(f"{where} must be <= {maximum}")
    return value


def _parse_timestamp(value: object, where: str) -> datetime:
    if not isinstance(value, str):
        raise CatalogInvalidError(f"{where} must be an ISO 8601 timestamp string")
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError as exc:
        raise CatalogInvalidError(
            f"{where} is not a valid ISO 8601 timestamp: {value!r}"
        ) from exc
    if parsed.utcoffset() is None:
        raise CatalogInvalidError(f"{where} must be timezone-aware: {value!r}")
    return parsed


def _parse_date(value: object, where: str) -> date:
    if not isinstance(value, str):
        raise CatalogInvalidError(f"{where} must be an ISO 8601 date string")
    try:
        return date.fromisoformat(value)
    except ValueError as exc:
        raise CatalogInvalidError(
            f"{where} is not a valid ISO 8601 date: {value!r}"
        ) from exc


class ModelSelectionError(Exception):
    """A selection request failed; ``code`` is the stable error-contract code."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


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
    canonical_token: str
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
    canonical_token: str | None
    estimated_usd: float | None
    estimate_status: str


def select_model(
    catalog: Catalog,
    *,
    role: str,
    ready_backends: frozenset[str],
    now: datetime,
    requires: Mapping[str, int] | None = None,
    token_estimate: Mapping[str, int] | None = None,
    backend: str | None = None,
) -> SelectionResult:
    """Deterministically select a model for ``role`` per the catalog's policies.

    Ordinary roles minimize estimated USD cost among capability-eligible models;
    ``monitor`` selects the least-cost monitor-capable model; ``reviewer`` selects
    the highest global rank among reviewer-capable models.
    """
    task_profile = _resolve_task_profile(catalog, role, requires)
    role_profile = catalog.role_profiles[role]
    estimate = _resolve_token_estimate(
        token_estimate, catalog.token_profiles[role_profile.token_profile]
    )
    if backend is not None and backend not in _BACKENDS:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown backend override {backend!r}"
        )
    _require_fresh(catalog, now)
    requested = _BACKENDS if backend is None else frozenset({backend})
    candidate_backends = requested & ready_backends
    if not candidate_backends:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    candidates, eligible = _enumerate_candidates(
        catalog, candidate_backends, task_profile, estimate, now
    )
    if not eligible:
        raise ModelSelectionError(
            "MODEL_NO_ELIGIBLE_CANDIDATE",
            f"no catalog candidate meets every constraint for role {role!r}",
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
        selected=_selected(catalog, best_model, best_cost),
    )


def select_replacement(
    catalog: Catalog,
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
    global rank than the failed model, skips already-attempted models, and
    minimizes cost only within that stronger eligible set.
    """
    task_profile = _resolve_task_profile(catalog, role, requires)
    models_by_key = {model.key: model for model in catalog.models}
    if failed_model_key not in models_by_key:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"failed model {failed_model_key!r} is not a catalog model",
        )
    failed_rank = models_by_key[failed_model_key].capability.global_rank
    for dimension in failed_dimensions:
        if dimension not in CAPABILITY_DIMENSIONS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown failed capability dimension {dimension!r}",
            )
        task_profile[dimension] = min(5, task_profile.get(dimension, 0) + 1)
    role_profile = catalog.role_profiles[role]
    estimate = _resolve_token_estimate(
        token_estimate, catalog.token_profiles[role_profile.token_profile]
    )
    _require_fresh(catalog, now)
    candidate_backends = _BACKENDS & ready_backends
    if not candidate_backends:
        raise ModelSelectionError(
            "MODEL_BACKEND_UNAVAILABLE",
            "no requested candidate backend passes its readiness contract",
        )
    candidates, eligible = _enumerate_candidates(
        catalog,
        candidate_backends,
        task_profile,
        estimate,
        now,
        min_rank_exclusive=failed_rank,
        excluded_keys=attempted_model_keys | {failed_model_key},
    )
    if not eligible:
        raise ModelSelectionError(
            "MODEL_UPGRADE_UNAVAILABLE",
            f"no strictly stronger eligible replacement for {failed_model_key!r}",
        )
    best_model, best_cost = min(eligible, key=_cost_order)
    return SelectionResult(
        policy="replacement_upgrade",
        role=role,
        task_profile=task_profile,
        token_estimate=estimate,
        candidates=candidates,
        selected=_selected(catalog, best_model, best_cost),
    )


def resolve_manual_override(
    catalog: Catalog,
    *,
    model: str,
    now: datetime,
    backend: str | None = None,
    token_estimate: Mapping[str, int] | None = None,
) -> ManualOverrideResult:
    """Resolve an explicit model pin through the exact token map.

    A mapped token fixes the backend and yields an estimate only when the
    catalog is fresh, a valid rate card exists, and a token estimate was given;
    an unmapped token stays permitted for the manual spawn path with
    ``estimate_status: "unavailable"``. A backend pin that conflicts with the
    token's mapped backend is rejected.
    """
    if backend is not None and backend not in _BACKENDS:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown backend override {backend!r}"
        )
    matches = [entry for entry in catalog.model_tokens if entry.token == model]
    if backend is not None:
        backend_matches = [entry for entry in matches if entry.backend == backend]
        if not backend_matches and matches:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"model {model!r} is mapped to backend"
                f" {matches[0].backend!r}, not {backend!r}",
            )
        matches = backend_matches
    if len({entry.model_key for entry in matches}) > 1:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST",
            f"model {model!r} is ambiguous across backends; pass an explicit backend",
        )
    if not matches:
        return ManualOverrideResult(
            policy="manual_override",
            backend=backend,
            model=model,
            canonical_token=None,
            estimated_usd=None,
            estimate_status="unavailable",
        )
    catalog_model = next(m for m in catalog.models if m.key == matches[0].model_key)
    canonical_token = _primary_tokens(catalog)[catalog_model.key]
    estimated_usd = None
    if token_estimate is not None and _stale_reason(catalog, now) is None:
        estimate = _resolve_token_estimate(token_estimate, None)
        estimated_usd = _best_card_cost(catalog_model, estimate, now)
    return ManualOverrideResult(
        policy="manual_override",
        backend=catalog_model.backend,
        model=canonical_token,
        canonical_token=canonical_token,
        estimated_usd=estimated_usd,
        estimate_status="ok" if estimated_usd is not None else "unavailable",
    )


def _resolve_task_profile(
    catalog: Catalog, role: str, requires: Mapping[str, int] | None
) -> dict[str, int]:
    if role not in catalog.role_profiles:
        raise ModelSelectionError(
            "MODEL_SELECTION_INVALID_REQUEST", f"unknown role {role!r}"
        )
    merged = dict(catalog.role_profiles[role].requires)
    for dimension, level in (requires or {}).items():
        if dimension not in CAPABILITY_DIMENSIONS:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"unknown capability dimension {dimension!r}",
            )
        if not _is_int(level) or not 1 <= level <= 5:
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
        if not _is_int(value) or value < 0:
            raise ModelSelectionError(
                "MODEL_SELECTION_INVALID_REQUEST",
                f"token estimate for {name!r} must be a non-negative integer",
            )
        components[name] = value
    return TokenEstimate(**components)


def _stale_reason(catalog: Catalog, now: datetime) -> str | None:
    for name, source in catalog.sources.items():
        age = now - source.retrieved_at
        if age > timedelta(days=catalog.freshness_days):
            return (
                f"source {name!r} was retrieved more than"
                f" {catalog.freshness_days} days before selection time"
            )
        if -age > timedelta(minutes=5):
            return f"source {name!r} was retrieved more than five minutes after selection time"
    return None


def _require_fresh(catalog: Catalog, now: datetime) -> None:
    reason = _stale_reason(catalog, now)
    if reason is not None:
        raise ModelSelectionError("MODEL_CATALOG_STALE", reason)


def _enumerate_candidates(
    catalog: Catalog,
    candidate_backends: frozenset[str],
    task_profile: Mapping[str, int],
    estimate: TokenEstimate,
    now: datetime,
    *,
    min_rank_exclusive: int | None = None,
    excluded_keys: frozenset[str] = frozenset(),
) -> tuple[tuple[CandidateRecord, ...], list[tuple[CatalogModel, float]]]:
    records = []
    eligible = []
    for model in catalog.models:
        cost = None
        if not model.active:
            reason = "inactive model"
        elif model.backend not in candidate_backends:
            reason = f"backend {model.backend!r} is not a ready requested backend"
        elif model.key in excluded_keys:
            reason = "model already attempted for this task"
        elif (
            min_rank_exclusive is not None
            and model.capability.global_rank <= min_rank_exclusive
        ):
            reason = (
                f"global_rank {model.capability.global_rank} is not strictly"
                f" greater than the failed model's {min_rank_exclusive}"
            )
        else:
            reason = _capability_shortfall(model, task_profile)
            if reason is None:
                cost = _best_card_cost(model, estimate, now)
                if cost is None:
                    reason = "no active known rate card supports the token estimate"
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
    model: CatalogModel, task_profile: Mapping[str, int]
) -> str | None:
    for dimension in CAPABILITY_DIMENSIONS:
        required = task_profile.get(dimension)
        if required is None:
            continue
        level = model.capability.levels[dimension]
        if level < required:
            return f"{dimension} capability {level} < {required}"
    return None


def _best_card_cost(
    model: CatalogModel, estimate: TokenEstimate, now: datetime
) -> float | None:
    selection_date = now.astimezone(UTC).date()
    costs = []
    for card in model.rate_cards:
        if card.status != "known":
            continue
        if card.effective_from > selection_date:
            continue
        if card.effective_until is not None and card.effective_until < selection_date:
            continue
        cost = _card_cost(card, estimate)
        if cost is not None:
            costs.append(cost)
    if not costs:
        return None
    return min(costs)


def _card_cost(card: RateCard, estimate: TokenEstimate) -> float | None:
    total = (
        estimate.input + estimate.cached_input + estimate.cache_write + estimate.output
    )
    if total > card.max_total_tokens:
        return None
    cost = 0.0
    for name in _TOKEN_COMPONENTS:
        tokens = getattr(estimate, name)
        component = card.components[name]
        if component.mode == "unsupported":
            if tokens:
                return None
            continue
        if component.usd_per_mtok is None:
            raise CatalogInvalidError(
                f"supported component {name!r} on rate card {card.id!r} has no price"
            )
        cost += tokens / 1_000_000 * component.usd_per_mtok
    return cost


def _cost_order(candidate: tuple[CatalogModel, float]) -> tuple[float, int, str]:
    model, cost = candidate
    return (cost, -model.capability.global_rank, model.key)


def _reviewer_order(candidate: tuple[CatalogModel, float]) -> tuple[int, float, str]:
    model, cost = candidate
    return (-model.capability.global_rank, cost, model.key)


def _primary_tokens(catalog: Catalog) -> dict[str, str]:
    return {
        entry.model_key: entry.token for entry in catalog.model_tokens if entry.primary
    }


def _selected(catalog: Catalog, model: CatalogModel, cost: float) -> SelectedModel:
    canonical_token = _primary_tokens(catalog)[model.key]
    return SelectedModel(
        key=model.key,
        backend=model.backend,
        model=canonical_token,
        canonical_token=canonical_token,
        effort=None,
        estimated_usd=cost,
    )
