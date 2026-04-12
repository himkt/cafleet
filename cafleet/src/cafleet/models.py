from pydantic import BaseModel


class PlacementCreate(BaseModel):
    director_agent_id: str
    tmux_session: str
    tmux_window_id: str
    tmux_pane_id: str | None = None
    coding_agent: str = "claude"


class PlacementView(BaseModel):
    director_agent_id: str
    tmux_session: str
    tmux_window_id: str
    tmux_pane_id: str | None
    coding_agent: str
    created_at: str


class PlacementPatch(BaseModel):
    tmux_pane_id: str


class RegisterAgentRequest(BaseModel):
    session_id: str
    name: str
    description: str
    skills: list[dict] | None = None
    placement: PlacementCreate | None = None


class RegisterAgentResponse(BaseModel):
    agent_id: str
    name: str
    registered_at: str
    placement: PlacementView | None = None


class AgentSummary(BaseModel):
    agent_id: str
    name: str
    description: str
    status: str
    registered_at: str
    placement: PlacementView | None = None


class ListAgentsResponse(BaseModel):
    agents: list[AgentSummary]


class ErrorDetail(BaseModel):
    code: str
    message: str


class ErrorResponse(BaseModel):
    error: ErrorDetail
