import { vi } from "vitest";
import type { FleetClient } from "../src/api";
import type { FleetListItem, Member, TimelineMessage } from "../src/types";

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
export const timestamp = "2026-09-06T01:02:03.000007+00:00";
export function members(fleetId = 1): Member[] {
  return [
    { member_id: fleetId * 10 + 1, name: `Director ${fleetId}`, kind: "director" },
    { member_id: fleetId * 10 + 2, name: `Worker ${fleetId}`, kind: "member" },
    { member_id: fleetId * 10 + 3, name: `Monitor ${fleetId}`, kind: "monitor" },
  ].map((row) => ({ ...row, kind: row.kind as Member["kind"], description: "fixture", status: "active", registered_at: timestamp }));
}
export function message(id = 1, body = "message body", fleetId = 1): TimelineMessage {
  return { message_id: id, from_member_id: fleetId * 10 + 1, from_member_name: `Director ${fleetId}`,
    to_member_id: fleetId * 10 + 2, to_member_name: `Worker ${fleetId}`, type: "unicast", status: "input_required",
    created_at: timestamp, status_timestamp: timestamp, origin_message_id: null, body };
}
export function fleets(): FleetListItem[] {
  return [1, 2].map((id) => ({ fleet_id: id, name: `Fleet ${id}`, created_at: timestamp, member_count: 3 }));
}
export function fakeClient(fleetId = 1, overrides: Partial<FleetClient> = {}): FleetClient {
  return {
    fleetId,
    getMembers: vi.fn<FleetClient["getMembers"]>().mockResolvedValue({ members: members(fleetId) }),
    fetchTimeline: vi.fn<FleetClient["fetchTimeline"]>().mockResolvedValue({ messages: [] }),
    fetchInbox: vi.fn<FleetClient["fetchInbox"]>().mockResolvedValue({ messages: [] }),
    fetchSent: vi.fn<FleetClient["fetchSent"]>().mockResolvedValue({ messages: [] }),
    getMonitor: vi.fn<FleetClient["getMonitor"]>().mockResolvedValue({ running: true, wake_interval_seconds: 600 }),
    sendMessage: vi.fn<FleetClient["sendMessage"]>().mockResolvedValue(undefined),
    patchMonitor: vi.fn<FleetClient["patchMonitor"]>().mockResolvedValue(undefined),
    postMonitorWake: vi.fn<FleetClient["postMonitorWake"]>().mockResolvedValue({ wake_requested_at: timestamp }),
    ...overrides,
  };
}
