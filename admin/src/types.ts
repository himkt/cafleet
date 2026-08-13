export interface MonitorRuntime {
  running: boolean;
  wake_interval_seconds: number | null;
}

export interface Member {
  member_id: number;
  name: string;
  description: string;
  status: "active" | "deregistered";
  registered_at: string;
  kind: "director" | "monitor" | "member";
}

export interface TimelineMessage {
  message_id: number;
  from_member_id: number;
  from_member_name: string;
  to_member_id: number;
  to_member_name: string;
  status: "input_required" | "completed";
  created_at: string;
  origin_message_id: number | null;
  status_timestamp: string;
  body: string;
}

export type TimelineEntry =
  | { kind: "unicast"; message: TimelineMessage }
  | { kind: "broadcast"; rows: TimelineMessage[]; sortKey: string };

export interface MembersResponse {
  members: Member[];
}

export interface TimelineResponse {
  messages: TimelineMessage[];
}

export interface FleetListItem {
  fleet_id: number;
  name: string | null;
  created_at: string;
  member_count: number;
}
