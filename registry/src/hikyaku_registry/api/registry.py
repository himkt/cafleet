from typing import Any

from fastapi import APIRouter, Depends, Request, Response
from fastapi.responses import JSONResponse

from hikyaku_registry.auth import get_authenticated_agent, get_registration_tenant
from hikyaku_registry.db.engine import get_sessionmaker
from hikyaku_registry.models import (
    AgentSummary,
    ErrorDetail,
    ErrorResponse,
    ListAgentsResponse,
    PlacementPatch,
    PlacementView,
    RegisterAgentRequest,
    RegisterAgentResponse,
)
from hikyaku_registry.registry_store import CreateAgentResult, RegistryStore

registry_router = APIRouter()


async def get_registry_store() -> RegistryStore:
    return RegistryStore(get_sessionmaker())


@registry_router.post("/agents", status_code=201, response_model=RegisterAgentResponse)
async def register_agent(
    body: RegisterAgentRequest,
    request: Request,
    store: RegistryStore = Depends(get_registry_store),
) -> dict[str, Any]:
    api_key, _tenant_id = await get_registration_tenant(request, store)

    if body.placement is not None:
        caller_id = request.headers.get("x-agent-id")
        if not caller_id or caller_id != body.placement.director_agent_id:
            return JSONResponse(
                status_code=401,
                content=ErrorResponse(
                    error=ErrorDetail(
                        code="UNAUTHORIZED",
                        message="X-Agent-Id must match placement.director_agent_id",
                    )
                ).model_dump(),
            )
        director_in_tenant = await store.verify_agent_tenant(caller_id, _tenant_id)
        if not director_in_tenant:
            return JSONResponse(
                status_code=403,
                content=ErrorResponse(
                    error=ErrorDetail(
                        code="FORBIDDEN",
                        message="Director agent is not in the caller's tenant",
                    )
                ).model_dump(),
            )

    result = await store.create_agent_with_placement(
        name=body.name,
        description=body.description,
        skills=body.skills,
        api_key=api_key,
        placement=body.placement,
    )

    response: dict[str, Any] = dict(result)
    if body.placement is not None:
        placement = await store.get_placement(result["agent_id"])
        if placement:
            response["placement"] = PlacementView(
                director_agent_id=placement["director_agent_id"],
                tmux_session=placement["tmux_session"],
                tmux_window_id=placement["tmux_window_id"],
                tmux_pane_id=placement["tmux_pane_id"],
                created_at=placement["created_at"],
            ).model_dump()
    return response


@registry_router.get("/agents", response_model=ListAgentsResponse)
async def list_agents(
    request: Request,
    director_agent_id: str | None = None,
    auth: tuple = Depends(get_authenticated_agent),
    store: RegistryStore = Depends(get_registry_store),
) -> dict[str, Any]:
    _agent_id, tenant_id = auth

    if director_agent_id is not None:
        members = await store.list_placements_for_director(
            tenant_id=tenant_id, director_agent_id=director_agent_id
        )
        return {
            "agents": [
                AgentSummary(
                    agent_id=m["agent_id"],
                    name=m["name"],
                    description=m["description"],
                    status=m["status"],
                    registered_at=m["registered_at"],
                    placement=PlacementView(**m["placement"])
                    if m.get("placement")
                    else None,
                ).model_dump()
                for m in members
            ]
        }

    agents = await store.list_active_agents(tenant_id=tenant_id)
    return {
        "agents": [
            AgentSummary(
                agent_id=a["agent_id"],
                name=a["name"],
                description=a.get("description", ""),
                status="active",
                registered_at=a["registered_at"],
            ).model_dump()
            for a in agents
        ]
    }


@registry_router.get("/agents/{agent_id}")
async def get_agent_detail(
    agent_id: str,
    auth: tuple = Depends(get_authenticated_agent),
    store: RegistryStore = Depends(get_registry_store),
):
    _caller_id, tenant_id = auth

    agent = await store.get_agent(agent_id)
    if agent is None or agent.get("status") == "deregistered":
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="AGENT_NOT_FOUND",
                    message=f"Agent with id '{agent_id}' not found",
                )
            ).model_dump(),
        )

    is_same_tenant = await store.verify_agent_tenant(agent_id, tenant_id)
    if not is_same_tenant:
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="AGENT_NOT_FOUND",
                    message=f"Agent with id '{agent_id}' not found",
                )
            ).model_dump(),
        )

    placement = await store.get_placement(agent_id)
    response = dict(agent)
    if placement:
        response["placement"] = PlacementView(
            director_agent_id=placement["director_agent_id"],
            tmux_session=placement["tmux_session"],
            tmux_window_id=placement["tmux_window_id"],
            tmux_pane_id=placement["tmux_pane_id"],
            created_at=placement["created_at"],
        ).model_dump()
    else:
        response["placement"] = None
    return response


@registry_router.delete("/agents/{agent_id}")
async def deregister_agent(
    agent_id: str,
    auth: tuple = Depends(get_authenticated_agent),
    store: RegistryStore = Depends(get_registry_store),
):
    caller_id, _tenant_id = auth

    agent = await store.get_agent(agent_id)
    if agent is None or agent.get("status") == "deregistered":
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="AGENT_NOT_FOUND",
                    message=f"Agent with id '{agent_id}' not found",
                )
            ).model_dump(),
        )

    allowed = False
    if agent["agent_id"] == caller_id:
        allowed = True
    else:
        placement = await store.get_placement(agent_id)
        if placement and placement["director_agent_id"] == caller_id:
            allowed = True

    if not allowed:
        return JSONResponse(
            status_code=403,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="FORBIDDEN",
                    message="API key does not match the target resource",
                )
            ).model_dump(),
        )

    await store.deregister_agent(agent_id)
    return Response(status_code=204)


@registry_router.patch("/agents/{agent_id}/placement")
async def patch_placement(
    agent_id: str,
    body: PlacementPatch,
    auth: tuple = Depends(get_authenticated_agent),
    store: RegistryStore = Depends(get_registry_store),
):
    caller_id, _tenant_id = auth

    placement = await store.get_placement(agent_id)
    if placement is None:
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="NOT_FOUND",
                    message=f"No placement found for agent '{agent_id}'",
                )
            ).model_dump(),
        )

    if placement["director_agent_id"] != caller_id:
        return JSONResponse(
            status_code=403,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="FORBIDDEN",
                    message="Only the Director can update this placement",
                )
            ).model_dump(),
        )

    updated = await store.update_placement_pane_id(agent_id, body.tmux_pane_id)
    if updated is None:
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="NOT_FOUND",
                    message=f"No placement found for agent '{agent_id}'",
                )
            ).model_dump(),
        )

    return PlacementView(
        director_agent_id=updated["director_agent_id"],
        tmux_session=updated["tmux_session"],
        tmux_window_id=updated["tmux_window_id"],
        tmux_pane_id=updated["tmux_pane_id"],
        created_at=updated["created_at"],
    ).model_dump()
