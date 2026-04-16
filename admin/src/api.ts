import type {
  AgentsResponse,
  TimelineResponse,
  SendMessageResponse,
  SessionListItem,
} from "./types";

let sessionId: string | null = null;

export function setSessionId(id: string | null): void {
  sessionId = id;
}

export function getSessionId(): string | null {
  return sessionId;
}

async function request<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const headers: Record<string, string> = {
    ...(options.headers as Record<string, string>),
  };

  if (sessionId) {
    headers["X-Session-Id"] = sessionId;
  }

  if (options.body && typeof options.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  const resp = await fetch(`/ui/api${path}`, { ...options, headers });

  if (!resp.ok) {
    const data = await resp.json().catch(() => ({}));
    throw new Error(data.error || data.detail || `HTTP ${resp.status}`);
  }

  if (resp.status === 204) {
    return undefined as T;
  }

  return resp.json() as Promise<T>;
}

export async function listSessions(): Promise<SessionListItem[]> {
  return request<SessionListItem[]>("/sessions");
}

export async function getAgents(): Promise<AgentsResponse> {
  return request<AgentsResponse>("/agents");
}

export async function fetchTimeline(): Promise<TimelineResponse> {
  return request<TimelineResponse>("/timeline");
}

export async function sendMessage(
  fromAgentId: string,
  toAgentId: string,
  text: string,
): Promise<SendMessageResponse> {
  return request<SendMessageResponse>("/messages/send", {
    method: "POST",
    body: JSON.stringify({
      from_agent_id: fromAgentId,
      to_agent_id: toAgentId,
      text,
    }),
  });
}
