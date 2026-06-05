"""FastAPI app for the admin WebUI (``/``) and its ``/api/*`` endpoints."""

import sys
from pathlib import Path

from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException

from cafleet.webui_api import webui_router


def default_webui_dist_dir() -> Path:
    return Path(__file__).resolve().parent / "webui"


class SPAStaticFiles(StaticFiles):
    """StaticFiles subclass that falls back to index.html for SPA routing.

    Re-raises 404 for paths under the reserved ``ui/`` and ``api/`` prefixes
    so they never silently resolve to ``index.html``:
    - ``ui/...`` — no UI is served under this prefix; the explicit 404 keeps
      it from silently resolving to ``index.html``.
    - ``api/...`` — backend prefix; unknown API paths must return JSON 404,
      not HTML, so client error paths (``resp.json()``) stay valid.
    """

    _RESERVED_PREFIXES = ("ui", "api")

    async def get_response(self, path, scope):
        try:
            return await super().get_response(path, scope)
        except StarletteHTTPException as e:
            if e.status_code != 404:
                raise
            first = path.split("/", 1)[0]
            if first in self._RESERVED_PREFIXES:
                raise
            return await super().get_response("index.html", scope)


def create_app(webui_dist_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="CAFleet Admin", version="0.1.0")
    app.include_router(webui_router)

    emit_warning_if_missing = webui_dist_dir is None
    if webui_dist_dir is None:
        webui_dist_dir = str(default_webui_dist_dir())
    dist_path = Path(webui_dist_dir)

    if emit_warning_if_missing and not dist_path.exists():
        print(
            "warning: admin WebUI is not built. / will return 404. "
            "Run 'mise //admin:build'.",
            file=sys.stderr,
        )

    if dist_path.exists():
        # include_router MUST precede this mount so API 404s are not swallowed by the SPA fallback.
        app.mount(
            "/",
            SPAStaticFiles(directory=str(dist_path)),
            name="webui",
        )

    return app


app = create_app()
