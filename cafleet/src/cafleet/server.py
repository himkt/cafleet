"""Minimal FastAPI app — Admin WebUI only.

Serves the WebUI API endpoints and the SPA static files.
CLI commands access SQLite directly through the ``broker`` module
and do not require this server.
"""

import sys
from pathlib import Path

from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException

from cafleet.webui_api import webui_router


def default_webui_dist_dir() -> Path:
    return Path(__file__).resolve().parent / "webui"


class SPAStaticFiles(StaticFiles):
    """StaticFiles subclass that falls back to index.html for SPA routing."""

    async def get_response(self, path, scope):
        try:
            return await super().get_response(path, scope)
        except StarletteHTTPException as e:
            if e.status_code == 404:
                return await super().get_response("index.html", scope)
            raise


def create_app(webui_dist_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="CAFleet Admin", version="0.1.0")
    app.include_router(webui_router)

    emit_warning_if_missing = webui_dist_dir is None
    if webui_dist_dir is None:
        webui_dist_dir = str(default_webui_dist_dir())
    dist_path = Path(webui_dist_dir)

    if emit_warning_if_missing and not dist_path.exists():
        print(
            "warning: admin WebUI is not built. /ui/ will return 404. "
            "Run 'mise //admin:build'.",
            file=sys.stderr,
        )

    if dist_path.exists():
        app.mount(
            "/ui",
            SPAStaticFiles(directory=str(dist_path)),
            name="webui",
        )

    return app


app = create_app()
