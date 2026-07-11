import type {
  MembersResponse,
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

export async function getMembers(): Promise<MembersResponse> {
  return request<MembersResponse>("/members");
}

export async function fetchTimeline(): Promise<TimelineResponse> {
  return request<TimelineResponse>("/timeline");
}

export async function fetchInbox(memberId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/members/${memberId}/inbox`);
}

export async function fetchSent(memberId: number): Promise<TimelineResponse> {
  return request<TimelineResponse>(`/members/${memberId}/sent`);
}

export async function sendMessage(
  fromMemberId: number,
  toMemberId: number | "*",
  text: string,
): Promise<void> {
  await request<unknown>("/messages/send", {
    method: "POST",
    body: JSON.stringify({
      from_member_id: fromMemberId,
      to_member_id: toMemberId,
      text,
    }),
  });
}

export async function getMonitor(): Promise<MonitorRuntime> {
  return request<MonitorRuntime>("/monitor");
}

export async function updateMemberMonitor(
  memberId: number,
  patch: { interval_seconds?: number; enabled?: boolean },
): Promise<MonitorConfig> {
  return request<MonitorConfig>(`/members/${memberId}/monitor`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
}
