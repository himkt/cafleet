"""Regression tests: broker guards exit 1, not 2 (design 0000118, items 2.2 + 3.3).

The root-Director deregistration guard (``deregister_member``) and the
soft-deleted-fleet guard (``register_member``) both raise ``click.ClickException``
(exit 1), matching the sibling CLI guard at ``member.py:328-331`` — not
``click.UsageError`` (exit 2). ``UsageError`` is a ``ClickException`` subclass
that overrides ``exit_code`` to 2, so the contract is asserted on ``exit_code``.
"""

import click
import pytest

from cafleet import broker
from tests.broker._helpers import _create_fleet


def test_deregister_root_director_guard_exits_1():
    fleet = _create_fleet()
    director_id = fleet["director"]["member_id"]

    with pytest.raises(click.ClickException) as exc_info:
        broker.deregister_member(director_id)

    assert exc_info.value.exit_code == 1


def test_register_into_soft_deleted_fleet_guard_exits_1():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    broker.delete_fleet(sid)

    with pytest.raises(click.ClickException) as exc_info:
        broker.register_member(fleet_id=sid, name="late", description="too late")

    assert exc_info.value.exit_code == 1
