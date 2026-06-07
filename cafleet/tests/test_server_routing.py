"""Tests for FastAPI app routing: SPA mount, reserved-prefix re-raise, mount order.

Guards the following behaviors:

- The mount at ``/`` serves the SPA shell for ``/`` and unknown client-side routes.
- ``SPAStaticFiles.get_response`` re-raises 404 for paths under the reserved
  ``ui/`` and ``api/`` prefixes instead of falling back to ``index.html``.
- ``include_router(webui_router)`` precedes ``app.mount("/", ...)``, so known
  ``/api/*`` routes reach the FastAPI router and are not intercepted by the
  catch-all SPA mount.
"""

import pytest
from fastapi.testclient import TestClient

from cafleet import server as server_mod


@pytest.fixture
def webui_dist(tmp_path):
    dist = tmp_path / "webui"
    dist.mkdir()
    (dist / "index.html").write_text(
        "<!doctype html><html><body data-spa-marker>spa</body></html>"
    )
    return dist


@pytest.fixture
def client(webui_dist, monkeypatch):
    monkeypatch.setattr(server_mod, "default_webui_dist_dir", lambda: webui_dist)
    app = server_mod.create_app()
    return TestClient(app)


def test_create_app__root_serves_spa_index(client):
    response = client.get("/")
    assert response.status_code == 200
    assert "data-spa-marker" in response.text
    assert response.headers["content-type"].startswith("text/html")


def test_create_app__unknown_client_route_falls_back_to_spa(client):
    response = client.get("/some/client-side/route")
    assert response.status_code == 200
    assert "data-spa-marker" in response.text


@pytest.mark.parametrize("path", ["/ui", "/ui/", "/ui/foo", "/ui/nested/deep"])
def test_create_app__ui_prefix_returns_404_not_spa(client, path):
    response = client.get(path)
    assert response.status_code == 404
    assert "data-spa-marker" not in response.text


@pytest.mark.parametrize("path", ["/api/does-not-exist", "/api/nested/unknown"])
def test_create_app__unknown_api_path_returns_404_not_spa(client, path):
    response = client.get(path)
    assert response.status_code == 404
    assert "data-spa-marker" not in response.text


def test_create_app__known_api_route_reaches_router_returns_json_400(client):
    # Mount-order regression guard: if the SPA
    # mount intercepts /api/* before the router, this would return the SPA
    # mount's 404 instead of the expected JSON 400 from get_webui_fleet().
    # The 400 short-circuits before any DB access, so no broker fixture needed.
    response = client.get("/api/agents")
    assert response.status_code == 400
    assert response.headers["content-type"].startswith("application/json")
    assert response.json() == {"detail": "X-Fleet-Id header required"}


def test_create_app__non_integer_fleet_id_header_returns_json_400(client):
    # get_webui_fleet int-coerces X-Fleet-Id; a non-integer header is rejected
    # with 400 before any DB access, so no broker fixture is needed.
    response = client.get("/api/agents", headers={"X-Fleet-Id": "not-an-int"})
    assert response.status_code == 400
    assert response.headers["content-type"].startswith("application/json")
    assert response.json() == {"detail": "X-Fleet-Id must be an integer"}
