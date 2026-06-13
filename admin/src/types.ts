export interface MonitorConfig {
  interval_seconds: number;
  last_ping_at: string | null;
  enabled: boolean;
}

export interface MonitorRuntime {
  running: boolean;
  pid: number | null;
  tick_seconds: number | null;
  last_tick_at: string | null;
  last_tick_age_seconds: number | null;
  started_at: string | null;
}

export interface Agent {
  agent_id: number;
  name: string;
  description: string;
  status: "active" | "deregistered";
  registered_at: string;
  kind: "builtin-administrator" | "user";
  monitor: MonitorConfig | null;
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
