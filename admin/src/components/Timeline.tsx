import { useState, useEffect, useRef, useCallback } from "react";
import type { TimelineMessage, TimelineEntry, Agent } from "../types";
import { fetchTimeline } from "../api";
import TimelineMessageComponent from "./TimelineMessage";

interface TimelineProps {
  agents: Agent[];
  refreshKey: number;
}

function entrySortKey(entry: TimelineEntry): string {
  return entry.kind === "unicast" ? entry.message.created_at : entry.sortKey;
}

function entryKey(entry: TimelineEntry): string {
  return entry.kind === "unicast"
    ? entry.message.task_id
    : `bcast:${entry.rows[0].origin_task_id ?? entry.rows[0].task_id}`;
}

function groupMessages(msgs: TimelineMessage[]): TimelineEntry[] {
  const groups = new Map<string, TimelineMessage[]>();
  const singletons: TimelineEntry[] = [];

  for (const m of msgs) {
    if (m.origin_task_id) {
      const g = groups.get(m.origin_task_id) ?? [];
      g.push(m);
      groups.set(m.origin_task_id, g);
    } else {
      singletons.push({ kind: "unicast", message: m });
    }
  }

  const broadcasts: TimelineEntry[] = [...groups.values()].map((rows) => ({
    kind: "broadcast" as const,
    rows,
    sortKey: rows.reduce(
      (min, r) => (r.created_at < min ? r.created_at : min),
      rows[0].created_at,
    ),
  }));

  return [...singletons, ...broadcasts].sort((a, b) =>
    entrySortKey(a).localeCompare(entrySortKey(b)),
  );
}

export default function Timeline({ agents, refreshKey }: TimelineProps) {
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const bottomRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    try {
      const data = await fetchTimeline();
      setEntries(groupMessages(data.messages));
    } catch {
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "auto" });
  }, [entries]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <p className="text-gray-400 text-sm">Loading timeline...</p>
      </div>
    );
  }

  if (entries.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <p className="text-gray-400 text-sm">No messages yet</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="divide-y divide-gray-100">
        {entries.map((entry) => (
          <TimelineMessageComponent
            key={entryKey(entry)}
            entry={entry}
            agents={agents}
          />
        ))}
      </div>
      <div ref={bottomRef} />
    </div>
  );
}
