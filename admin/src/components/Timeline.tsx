import { useLayoutEffect, useRef, useCallback, useMemo, useEffect } from "react";
import type { ReactNode } from "react";
import { Inbox } from "lucide-react";
import type { TimelineEntry, Member } from "../types";
import { ApiError } from "../api";
import type { FleetClient } from "../api";
import { useResource } from "../hooks/useResource";
import ResourceNotice from "./ResourceNotice";
import { entrySortKey, groupMessages } from "../timeline";
import TimelineMessageComponent from "./TimelineMessage";
import EmptyState from "./EmptyState";
import Skeleton from "./Skeleton";

export interface TimelineProps {
  client: FleetClient;
  members: Member[];
  refreshKey: number;
}

function entryKey(entry: TimelineEntry): string {
  return entry.kind === "unicast"
    ? String(entry.message.message_id)
    : `bcast:${entry.rows[0].origin_message_id ?? entry.rows[0].message_id}`;
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

export default function Timeline({ client, members, refreshKey }: TimelineProps) {
  const load = useCallback((signal: AbortSignal) => client.fetchTimeline({ signal }), [client]);
  const { state, refresh } = useResource({ key: `${client.fleetId}:timeline`, load, refreshKey });
  useEffect(() => {
    const error = state.error;
    if (error instanceof ApiError && error.status === 404 && error.message === "Fleet not found") {
      window.location.replace("#/fleets");
    }
  }, [state.error]);
  const entries = useMemo(() => state.data ? groupMessages(state.data.messages) : [], [state.data]);
  const bottomRef = useRef<HTMLDivElement>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const prevScrollHeightRef = useRef<number | null>(null);

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

  if (state.status === "loading") return <div aria-label="Loading timeline"><TimelineSkeleton /></div>;
  const notice = <ResourceNotice state={state} name="timeline" retry={refresh} />;
  if (state.status === "error") return notice;

  if (entries.length === 0) {
    return (
      <><ResourceNotice state={state} name="timeline" retry={refresh} /><div className="flex flex-1 items-center justify-center">
        <EmptyState icon={Inbox} title="No messages yet" />
      </div></>
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
        members={members}
      />,
    );
  }

  return (
    <>{notice}<div ref={scrollerRef} className="flex-1 overflow-y-auto py-2">
      {items}
      <div ref={bottomRef} />
    </div></>
  );
}
