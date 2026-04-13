"""Minimal FastAPI app — Admin WebUI only.

Serves the WebUI API endpoints and the SPA static files.
CLI commands access SQLite directly through the ``broker`` module
and do not require this server.
"""

import logging
from pathlib import Path

import uvicorn
from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException

from cafleet.config import settings
from cafleet.webui_api import webui_router

logger = logging.getLogger(__name__)


def _default_webui_dist_dir() -> Path:
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

    if webui_dist_dir is None:
        webui_dist_dir = str(_default_webui_dist_dir())
    dist_path = Path(webui_dist_dir)
    if dist_path.exists():
        app.mount(
            "/ui",
            SPAStaticFiles(directory=str(dist_path)),
            name="webui",
        )

    return app


app = create_app()


if __name__ == "__main__":
    uvicorn.run(
        "cafleet.server:app",
        host=settings.broker_host,
        port=settings.broker_port,
        reload=True,
    )
