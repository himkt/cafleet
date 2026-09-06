import type { MembersResponse, TimelineResponse, FleetListItem, MonitorRuntime } from "./types";

export interface RequestOptions { signal?: AbortSignal }
export class ApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}
export interface FleetClient {
  readonly fleetId: number;
  getMembers(options?: RequestOptions): Promise<MembersResponse>;
  fetchTimeline(options?: RequestOptions): Promise<TimelineResponse>;
  fetchInbox(memberId: number, options?: RequestOptions): Promise<TimelineResponse>;
  fetchSent(memberId: number, options?: RequestOptions): Promise<TimelineResponse>;
  getMonitor(options?: RequestOptions): Promise<MonitorRuntime>;
  sendMessage(fromMemberId: number, toMemberId: number | "*", text: string, options?: RequestOptions): Promise<void>;
  patchMonitor(wakeIntervalSeconds: number, options?: RequestOptions): Promise<void>;
  postMonitorWake(options?: RequestOptions): Promise<{ wake_requested_at: string }>;
}
async function request<T>(path: string, fleetId: number | null, options: RequestInit = {}): Promise<T> {
  const headers: Record<string, string> = {};
  if (fleetId !== null) headers["X-Fleet-Id"] = String(fleetId);
  if (typeof options.body === "string") headers["Content-Type"] = "application/json";
  const response = await fetch(`/api${path}`, { ...options, headers });
  if (!response.ok) {
    const data = await response.json().catch(() => null);
    throw new ApiError(response.status, data?.error || data?.detail || `HTTP ${response.status}`);
  }
  return response.json() as Promise<T>;
}
export function listFleets(options: RequestOptions = {}): Promise<FleetListItem[]> {
  return request("/fleets", null, options);
}
export function createFleetClient(fleetId: number): FleetClient {
  if (!Number.isSafeInteger(fleetId) || fleetId <= 0) throw new RangeError("Invalid fleet ID");
  return Object.freeze({
    fleetId,
    getMembers: (options: RequestOptions = {}) => request<MembersResponse>("/members", fleetId, options),
    fetchTimeline: (options: RequestOptions = {}) => request<TimelineResponse>("/timeline", fleetId, options),
    fetchInbox: (memberId: number, options: RequestOptions = {}) => request<TimelineResponse>(`/members/${memberId}/inbox`, fleetId, options),
    fetchSent: (memberId: number, options: RequestOptions = {}) => request<TimelineResponse>(`/members/${memberId}/sent`, fleetId, options),
    getMonitor: (options: RequestOptions = {}) => request<MonitorRuntime>("/monitor", fleetId, options),
    async sendMessage(fromMemberId: number, toMemberId: number | "*", text: string, options: RequestOptions = {}) {
      await request("/messages/send", fleetId, { ...options, method: "POST", body: JSON.stringify({ from_member_id: fromMemberId, to_member_id: toMemberId, text }) });
    },
    async patchMonitor(wakeIntervalSeconds: number, options: RequestOptions = {}) {
      await request("/monitor", fleetId, { ...options, method: "PATCH", body: JSON.stringify({ wake_interval_seconds: wakeIntervalSeconds }) });
    },
    postMonitorWake: (options: RequestOptions = {}) => request<{ wake_requested_at: string }>("/monitor/wake", fleetId, { ...options, method: "POST" }),
  });
}
