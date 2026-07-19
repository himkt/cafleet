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
from datetime import date, datetime
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
