import {
  useState,
  useEffect,
  useLayoutEffect,
  useRef,
  useCallback,
} from "react";
import type { TimelineMessage, TimelineEntry, Agent } from "../types";
import { fetchTimeline } from "../api";
import { entrySortKey } from "../timeline";
import TimelineMessageComponent from "./TimelineMessage";

interface TimelineProps {
  agents: Agent[];
  refreshKey: number;
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
    if (!m.origin_task_id) {
      singletons.push({ kind: "unicast", message: m });
      continue;
    }
    const existing = groups.get(m.origin_task_id);
    if (existing) {
      existing.push(m);
    } else {
      groups.set(m.origin_task_id, [m]);
    }
  }

  const broadcasts = Array.from(
    groups.values(),
    (rows): TimelineEntry => ({
      kind: "broadcast",
      rows,
      sortKey: rows.reduce((a, b) =>
        a.created_at < b.created_at ? a : b,
      ).created_at,
    }),
  );

  return [...singletons, ...broadcasts].sort((a, b) =>
    entrySortKey(a).localeCompare(entrySortKey(b)),
  );
}

export default function Timeline({ agents, refreshKey }: TimelineProps) {
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [isPolling, setIsPolling] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const prevScrollHeightRef = useRef<number | null>(null);
  const inFlightRef = useRef(false);

  // Timeline is driven entirely by Dashboard's `refreshKey` bump, which Dashboard's
  // usePolling fires every POLL_INTERVAL_MS. Owning a second usePolling here would
  // double the fetch rate without adding coverage. The local in-flight guard still
  // matters because a refreshKey bump can land while a slow fetchTimeline() is
  // pending.
  const loadTimeline = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    setIsPolling(true);
    try {
      const data = await fetchTimeline();
      setEntries(groupMessages(data.messages));
    } catch {
      /* swallow — preserve last-known entries; next bump re-attempts */
    } finally {
      setLoading(false);
      setIsPolling(false);
      inFlightRef.current = false;
    }
  }, []);

  useEffect(() => {
    void loadTimeline();
  }, [refreshKey, loadTimeline]);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const NEAR_BOTTOM_PX = 80;
    const prev = prevScrollHeightRef.current;
    const wasNearBottom =
      prev === null ? true : prev - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
    if (wasNearBottom) {
      bottomRef.current?.scrollIntoView({ behavior: "auto" });
    }
    prevScrollHeightRef.current = el.scrollHeight;
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
      <div className="flex-1 flex items-center justify-center relative">
        {isPolling && (
          <span className="absolute top-2 right-2 text-xs text-gray-400 italic">
            Updating…
          </span>
        )}
        <p className="text-gray-400 text-sm">No messages yet</p>
      </div>
    );
  }

  return (
    <div ref={scrollerRef} className="flex-1 overflow-y-auto relative">
      {isPolling && (
        <span className="absolute top-2 right-2 text-xs text-gray-400 italic">
          Updating…
        </span>
      )}
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
