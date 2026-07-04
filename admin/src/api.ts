import type {
  AgentsResponse,
  TimelineResponse,
  FleetListItem,
  MonitorConfig,
  MonitorRuntime,
} from "./types";

let fleetId: number | null = null;

export function setFleetId(id: number | null): void {
  fleetId = id;
}

async function request<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const headers: Record<string, string> = {
    ...(options.headers as Record<string, string>),
  };

  if (fleetId !== null) {
    headers["X-Fleet-Id"] = String(fleetId);
  }

  if (options.body && typeof options.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  const resp = await fetch(`/api${path}`, { ...options, headers });

  if (!resp.ok) {
    const data = await resp.json().catch(() => ({}));
    throw new Error(data.error || data.detail || `HTTP ${resp.status}`);
  }

  return resp.json() as Promise<T>;
}

export async function listFleets(): Promise<FleetListItem[]> {
  return request<FleetListItem[]>("/fleets");
}

export async function getAgents(): Promise<AgentsResponse> {
  return request<AgentsResponse>("/agents");
}

export async function fetchTimeline(): Promise<TimelineResponse> {
  return request<TimelineResponse>("/timeline");
}

export async function fetchInbox(agentId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/agents/${agentId}/inbox`);
}

export async function fetchSent(agentId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/agents/${agentId}/sent`);
}

export async function sendMessage(
  fromAgentId: number,
  toAgentId: number | "*",
  text: string,
): Promise<void> {
  await request<unknown>("/messages/send", {
    method: "POST",
    body: JSON.stringify({
      from_agent_id: fromAgentId,
      to_agent_id: toAgentId,
      text,
    }),
  });
}

export async function getMonitor(): Promise<MonitorRuntime> {
  return request<MonitorRuntime>("/monitor");
}

export async function updateAgentMonitor(
  agentId: number,
  patch: { interval_seconds?: number; enabled?: boolean },
): Promise<MonitorConfig> {
  return request<MonitorConfig>(`/agents/${agentId}/monitor`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
}
