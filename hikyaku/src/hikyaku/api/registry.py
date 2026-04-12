from typing import Any

from fastapi import APIRouter, Depends, Request, Response
from fastapi.responses import JSONResponse

from hikyaku.db.engine import get_sessionmaker
from hikyaku.models import (
    AgentSummary,
    ErrorDetail,
    ErrorResponse,
    ListAgentsResponse,
    PlacementPatch,
    PlacementView,
    RegisterAgentRequest,
    RegisterAgentResponse,
)
from hikyaku.registry_store import RegistryStore

registry_router = APIRouter()


async def get_registry_store() -> RegistryStore:
    return RegistryStore(get_sessionmaker())


@registry_router.post("/agents", status_code=201, response_model=RegisterAgentResponse)
async def register_agent(
    body: RegisterAgentRequest,
    request: Request,
    store: RegistryStore = Depends(get_registry_store),
):
    session = await store.get_session(body.session_id)
    if session is None:
        return JSONResponse(
            status_code=404,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="SESSION_NOT_FOUND",
                    message=f"Session '{body.session_id}' not found",
                )
            ).model_dump(),
        )

    if body.placement is not None:
        caller_id = request.headers.get("x-agent-id")
        if not caller_id or caller_id != body.placement.director_agent_id:
            return JSONResponse(
                status_code=400,
                content=ErrorResponse(
                    error=ErrorDetail(
                        code="AGENT_ID_REQUIRED",
                        message="X-Agent-Id must match placement.director_agent_id",
                    )
                ).model_dump(),
            )
        director = await store.get_agent(caller_id)
        if (
            director is None
            or director.get("status") != "active"
            or not await store.verify_agent_session(caller_id, body.session_id)
        ):
            return JSONResponse(
                status_code=403,
                content=ErrorResponse(
                    error=ErrorDetail(
                        code="FORBIDDEN",
                        message="Director agent is not active in the session",
                    )
                ).model_dump(),
            )

    result = await store.create_agent_with_placement(
        name=body.name,
        description=body.description,
        skills=body.skills,
        session_id=body.session_id,
        placement=body.placement,
    )

    response: dict[str, Any] = {**result}
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
    session_id: str | None = None,
    director_agent_id: str | None = None,
    store: RegistryStore = Depends(get_registry_store),
) -> Any:
    if not session_id:
        return JSONResponse(
            status_code=400,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="SESSION_REQUIRED",
                    message="session_id query parameter is required",
                )
            ).model_dump(),
        )

    if director_agent_id is not None:
        members = await store.list_placements_for_director(
            session_id=session_id, director_agent_id=director_agent_id
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

    agents = await store.list_active_agents(session_id=session_id)
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
    request: Request,
    store: RegistryStore = Depends(get_registry_store),
):
    session_id = request.headers.get("x-session-id")
    if not session_id:
        return JSONResponse(
            status_code=400,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="SESSION_REQUIRED",
                    message="X-Session-Id header is required",
                )
            ).model_dump(),
        )

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

    is_same_session = await store.verify_agent_session(agent_id, session_id)
    if not is_same_session:
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
    request: Request,
    store: RegistryStore = Depends(get_registry_store),
):
    caller_id = request.headers.get("x-agent-id")
    if not caller_id:
        return JSONResponse(
            status_code=400,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="AGENT_ID_REQUIRED",
                    message="X-Agent-Id header is required",
                )
            ).model_dump(),
        )

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
                    message="Caller is not authorized to deregister this agent",
                )
            ).model_dump(),
        )

    await store.deregister_agent(agent_id)
    return Response(status_code=204)


@registry_router.patch("/agents/{agent_id}/placement")
async def patch_placement(
    agent_id: str,
    body: PlacementPatch,
    request: Request,
    store: RegistryStore = Depends(get_registry_store),
):
    caller_id = request.headers.get("x-agent-id")
    if not caller_id:
        return JSONResponse(
            status_code=400,
            content=ErrorResponse(
                error=ErrorDetail(
                    code="AGENT_ID_REQUIRED",
                    message="X-Agent-Id header is required",
                )
            ).model_dump(),
        )

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
