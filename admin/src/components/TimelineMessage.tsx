import type { TimelineEntry, Agent } from "../types";
import ReactionBar from "./ReactionBar";

interface TimelineMessageProps {
  entry: TimelineEntry;
  agents: Agent[];
}

function MentionChip({ name }: { name: string }) {
  return (
    <span className="inline-block px-1.5 py-0.5 text-xs rounded bg-blue-100 text-blue-700 font-medium">
      @{name}
    </span>
  );
}

function senderName(entry: TimelineEntry): string {
  if (entry.kind === "unicast") return entry.message.from_agent_name || "?";
  return entry.rows[0]?.from_agent_name || "?";
}

function body(entry: TimelineEntry): string {
  if (entry.kind === "unicast") return entry.message.body;
  return entry.rows[0]?.body || "";
}

function isCanceled(entry: TimelineEntry): boolean {
  if (entry.kind === "unicast") return entry.message.status === "canceled";
  return entry.rows.every((r) => r.status === "canceled");
}

function recipientNames(entry: TimelineEntry): string[] {
  if (entry.kind === "unicast") {
    return [entry.message.to_agent_name || "?"];
  }
  return entry.rows.map((r) => r.to_agent_name || "?");
}

function createdAt(entry: TimelineEntry): string {
  if (entry.kind === "unicast") return entry.message.created_at;
  return entry.sortKey;
}

function formatTime(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default function TimelineMessageComponent({
  entry,
  agents,
}: TimelineMessageProps) {
  const canceled = isCanceled(entry);

  return (
    <div className="px-4 py-2 hover:bg-gray-50">
      <div className="flex items-baseline gap-1.5">
        <span className="text-xs text-gray-400 shrink-0">
          {formatTime(createdAt(entry))}
        </span>
        <span className="font-medium text-sm text-gray-900">
          {senderName(entry)}
        </span>
        <span className="text-xs text-gray-400">&rarr;</span>
        {recipientNames(entry).map((name, i) => (
          <MentionChip key={i} name={name} />
        ))}
      </div>
      {canceled ? (
        <p className="mt-0.5 text-sm opacity-60">
          <s>{body(entry)}</s>
        </p>
      ) : (
        <>
          <p className="mt-0.5 text-sm text-gray-700">{body(entry)}</p>
          <ReactionBar entry={entry} agents={agents} />
        </>
      )}
    </div>
  );
}
