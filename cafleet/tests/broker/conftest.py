"""Broker-suite fixtures: every broker test runs against the patched in-memory sessionmaker."""

import pytest


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session
