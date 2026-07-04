import { useState, useLayoutEffect, useRef, useCallback } from "react";
import type { ReactNode } from "react";
import { Inbox } from "lucide-react";
import type { TimelineMessage, TimelineEntry, Agent } from "../types";
import { fetchTimeline } from "../api";
import { useRefreshKeyLoad } from "../hooks/useRefreshKeyLoad";
import { entrySortKey } from "../timeline";
import TimelineMessageComponent from "./TimelineMessage";
import EmptyState from "./EmptyState";
import Skeleton from "./Skeleton";

interface TimelineProps {
  agents: Agent[];
  refreshKey: number;
}

function entryKey(entry: TimelineEntry): string {
  return entry.kind === "unicast"
    ? String(entry.message.task_id)
    : `bcast:${entry.rows[0].origin_task_id ?? entry.rows[0].task_id}`;
}

function groupMessages(msgs: TimelineMessage[]): TimelineEntry[] {
  const groups = new Map<number, TimelineMessage[]>();
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

function dayLabel(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

function DayDivider({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-3 px-4 py-2">
      <div className="h-px flex-1 bg-border" aria-hidden="true" />
      <span className="text-xs font-medium text-text-faint">{label}</span>
      <div className="h-px flex-1 bg-border" aria-hidden="true" />
    </div>
  );
}

function TimelineSkeleton() {
  return (
    <div className="flex-1 overflow-y-auto px-4 py-3">
      {[0, 1, 2, 3].map((i) => (
        <div key={i} className="flex gap-3 py-2.5">
          <Skeleton className="size-8 rounded-full" />
          <div className="flex-1">
            <Skeleton className="h-3 w-40" />
            <Skeleton className="mt-2 h-3 w-72 max-w-full" />
          </div>
        </div>
      ))}
    </div>
  );
}

export default function Timeline({ agents, refreshKey }: TimelineProps) {
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const bottomRef = useRef<HTMLDivElement>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const prevScrollHeightRef = useRef<number | null>(null);

  // Timeline is driven entirely by Dashboard's `refreshKey` bump, which Dashboard's
  // usePolling fires every POLL_INTERVAL_MS. Owning a second usePolling here would
  // double the fetch rate without adding coverage. useRefreshKeyLoad owns the
  // in-flight guard, which still matters because a refreshKey bump can land while a
  // slow fetchTimeline() is pending.
  const loadTimeline = useCallback(async () => {
    try {
      const data = await fetchTimeline();
      setEntries(groupMessages(data.messages));
    } catch {
      /* swallow — preserve last-known entries; next bump re-attempts */
    } finally {
      setLoading(false);
    }
  }, []);

  useRefreshKeyLoad(loadTimeline, refreshKey);

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
    return <TimelineSkeleton />;
  }

  if (entries.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <EmptyState icon={Inbox} title="No messages yet" />
      </div>
    );
  }

  const items: ReactNode[] = [];
  let prevDay: string | null = null;
  for (const entry of entries) {
    const day = new Date(entrySortKey(entry)).toDateString();
    if (day !== prevDay) {
      items.push(
        <DayDivider key={`day:${day}`} label={dayLabel(entrySortKey(entry))} />,
      );
      prevDay = day;
    }
    items.push(
      <TimelineMessageComponent
        key={entryKey(entry)}
        entry={entry}
        agents={agents}
      />,
    );
  }

  return (
    <div ref={scrollerRef} className="flex-1 overflow-y-auto py-2">
      {items}
      <div ref={bottomRef} />
    </div>
  );
}
