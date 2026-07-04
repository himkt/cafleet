"""Regression tests: kind predicates are null/non-object safe (design 0000118, item 2.4).

``is_administrator`` / ``is_monitoring_member`` read ``card["cafleet"]["kind"]``.
A malformed card whose ``cafleet`` value is null or a non-object
(``{"cafleet": null}``, ``{"cafleet": "x"}``) must return ``False`` rather than
raising ``AttributeError`` — the guard extends beyond the JSON-parse
``except ValueError`` to also handle a ``cafleet`` value that is not a dict.
"""

import json

import pytest

from cafleet.broker import _shared

_PREDICATES = [_shared.is_administrator, _shared.is_monitoring_member]

_MALFORMED_CARDS = [
    '{"cafleet": null}',
    '{"cafleet": "x"}',
    '{"cafleet": 42}',
    '{"cafleet": [1, 2]}',
]


@pytest.mark.parametrize("predicate", _PREDICATES)
@pytest.mark.parametrize("card", _MALFORMED_CARDS)
def test_kind_predicate_returns_false_on_non_object_cafleet(predicate, card):
    assert predicate(card) is False


@pytest.mark.parametrize("predicate", _PREDICATES)
def test_kind_predicate_returns_false_on_missing_or_empty_cafleet(predicate):
    assert predicate('{"name": "agent"}') is False
    assert predicate('{"cafleet": {}}') is False


def test_is_administrator_still_true_on_well_formed_card():
    card = json.dumps({"cafleet": {"kind": _shared.ADMINISTRATOR_KIND}})
    assert _shared.is_administrator(card) is True
    assert _shared.is_monitoring_member(card) is False


def test_is_monitoring_member_still_true_on_well_formed_card():
    card = json.dumps({"cafleet": {"kind": _shared.MONITORING_MEMBER_KIND}})
    assert _shared.is_monitoring_member(card) is True
    assert _shared.is_administrator(card) is False
