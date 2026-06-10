import type { TimelineEntry, Agent } from "../types";
import { entrySortKey } from "../timeline";
import AgentAvatar from "./AgentAvatar";
import ReactionBar from "./ReactionBar";

interface TimelineMessageProps {
  entry: TimelineEntry;
  agents: Agent[];
}

function MentionChip({ name }: { name: string }) {
  return (
    <span className="inline-block rounded bg-accent-soft px-1.5 py-0.5 text-xs font-medium text-accent">
      @{name}
    </span>
  );
}

function firstRow(entry: TimelineEntry) {
  return entry.kind === "unicast" ? entry.message : entry.rows[0];
}

function body(entry: TimelineEntry): string {
  return firstRow(entry).body;
}

function isCanceled(entry: TimelineEntry): boolean {
  if (entry.kind === "unicast") return entry.message.status === "canceled";
  return entry.rows.every((r) => r.status === "canceled");
}

function recipientNames(entry: TimelineEntry): string[] {
  if (entry.kind === "unicast") {
    return [entry.message.to_agent_name];
  }
  return entry.rows.map((r) => r.to_agent_name);
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default function TimelineMessageComponent({
  entry,
  agents,
}: TimelineMessageProps) {
  const canceled = isCanceled(entry);
  const row = firstRow(entry);
  const sender = agents.find((a) => a.agent_id === row.from_agent_id) ?? {
    agent_id: row.from_agent_id,
    name: row.from_agent_name,
    kind: "user" as const,
  };

  return (
    <div className="flex gap-3 px-4 py-2 hover:bg-surface-hover motion-safe:animate-rise-in">
      <AgentAvatar agent={sender} size="md" />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-1.5">
          <span className="text-sm font-semibold">{row.from_agent_name}</span>
          <span className="text-xs text-text-faint" aria-hidden="true">
            &rarr;
          </span>
          {recipientNames(entry).map((name, i) => (
            <MentionChip key={i} name={name} />
          ))}
          <span className="text-xs text-text-faint">
            {formatTime(entrySortKey(entry))}
          </span>
        </div>
        {canceled ? (
          <p className="mt-0.5 whitespace-pre-wrap break-words text-sm opacity-60">
            <s>{body(entry)}</s>
          </p>
        ) : (
          <>
            <p className="mt-0.5 whitespace-pre-wrap break-words text-sm">
              {body(entry)}
            </p>
            <ReactionBar entry={entry} agents={agents} />
          </>
        )}
      </div>
    </div>
  );
}
