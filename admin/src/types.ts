export interface Agent {
  agent_id: string;
  name: string;
  description: string;
  status: "active" | "deregistered";
  registered_at: string;
  kind: "builtin-administrator" | "user";
}

export interface TimelineMessage {
  task_id: string;
  from_agent_id: string;
  from_agent_name: string;
  to_agent_id: string;
  to_agent_name: string;
  type: string;
  status: "input_required" | "completed" | "canceled";
  created_at: string;
  origin_task_id: string | null;
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

export interface SessionListItem {
  session_id: string;
  label: string | null;
  created_at: string;
  agent_count: number;
}
