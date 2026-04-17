import type { TimelineEntry, Agent, TimelineMessage } from "../types";

interface ReactionBarProps {
  entry: TimelineEntry;
  agents: Agent[];
}

function formatIso(iso: string): string {
  return new Date(iso).toISOString();
}

function agentLabel(agentId: string, agents: Agent[]): string {
  const agent = agents.find((a) => a.agent_id === agentId);
  if (!agent) return agentId.slice(0, 8);
  const suffix = agent.status === "deregistered" ? " (deregistered)" : "";
  return `@${agent.name}${suffix}`;
}

function getCompletedRows(entry: TimelineEntry): TimelineMessage[] {
  if (entry.kind === "unicast") {
    return entry.message.status === "completed" ? [entry.message] : [];
  }
  return entry.rows.filter((r) => r.status === "completed");
}

export default function ReactionBar({ entry, agents }: ReactionBarProps) {
  const completedRows = getCompletedRows(entry);

  if (completedRows.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1 mt-1">
      {completedRows.map((row) => (
        <span key={row.task_id} className="group relative inline-flex">
          <span className="inline-block px-1.5 py-0.5 text-xs rounded bg-green-100 text-green-700">
            [ack]
          </span>
          <span className="absolute bottom-full left-0 mb-1 whitespace-nowrap rounded bg-gray-900 px-2 py-1 text-xs text-white opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-10">
            {agentLabel(row.to_agent_id, agents)} —{" "}
            {formatIso(row.status_timestamp)}
          </span>
        </span>
      ))}
    </div>
  );
}
