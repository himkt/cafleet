# Commands

**IMPORTANT**: Always `cd` into the package directory before running tests. Running from the project root causes module-not-found errors.

- Run registry tests: `cd registry` then `uv run pytest tests/ -v`
- Run client tests: `cd client` then `uv run pytest tests/ -v`
- Run specific test file: `cd registry` then `uv run pytest tests/test_executor.py -v`
- Start broker server: `cd registry` then `uv run uvicorn hikyaku_registry.main:app`
- Sync workspace: `uv sync` (from project root)
