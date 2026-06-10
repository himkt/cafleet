import { Tooltip } from "radix-ui";
import type { TimelineEntry, Agent, TimelineMessage } from "../types";

interface ReactionBarProps {
  entry: TimelineEntry;
  agents: Agent[];
}

function formatIso(iso: string): string {
  return new Date(iso).toISOString();
}

function agentLabel(agentId: number, agents: Agent[]): string {
  const agent = agents.find((a) => a.agent_id === agentId);
  if (!agent) return String(agentId);
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
    <Tooltip.Provider delayDuration={200}>
      <div className="mt-1 flex flex-wrap gap-1">
        {completedRows.map((row) => (
          <Tooltip.Root key={row.task_id}>
            <Tooltip.Trigger className="cursor-default rounded-full bg-success-soft px-2 py-0.5 text-xs font-medium text-success focus-visible:outline-2 focus-visible:outline-accent">
              [ack]
            </Tooltip.Trigger>
            <Tooltip.Portal>
              <Tooltip.Content
                side="top"
                align="start"
                sideOffset={4}
                className="z-10 rounded-md bg-text px-2 py-1 text-xs text-surface shadow-md"
              >
                {agentLabel(row.to_agent_id, agents)} —{" "}
                <span className="font-mono">
                  {formatIso(row.status_timestamp)}
                </span>
              </Tooltip.Content>
            </Tooltip.Portal>
          </Tooltip.Root>
        ))}
      </div>
    </Tooltip.Provider>
  );
}
