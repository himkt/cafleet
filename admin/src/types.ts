export interface Agent {
  agent_id: number;
  name: string;
  description: string;
  status: "active" | "deregistered";
  registered_at: string;
  kind: "builtin-administrator" | "user";
}

export interface TimelineMessage {
  task_id: number;
  from_agent_id: number;
  from_agent_name: string;
  to_agent_id: number;
  to_agent_name: string;
  type: string;
  status: "input_required" | "completed" | "canceled";
  created_at: string;
  origin_task_id: number | null;
  status_timestamp: string;
  body: string;
}

export type TimelineEntry =
  | { kind: "unicast"; message: TimelineMessage }
  | { kind: "broadcast"; rows: TimelineMessage[]; sortKey: string };

export interface AgentsResponse {
  agents: Agent[];
}

export interface TimelineResponse {
  messages: TimelineMessage[];
}

export interface FleetListItem {
  fleet_id: number;
  label: string | null;
  created_at: string;
  agent_count: number;
}
