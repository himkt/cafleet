"""Shared test fixtures for hikyaku-registry tests.

The Redis-based fixtures (redis_client, store, task_store) were removed in the
SQLite migration (Step 1) because fakeredis is no longer in the dependency
group. The full SQLAlchemy-based fixture stack lands in Step 12.

Until then, test files that depended on the old fixtures will error on fixture
resolution at collection time — that brokenness is intentional and gets fixed
as each store is rewritten in Steps 5/6/7.
"""
