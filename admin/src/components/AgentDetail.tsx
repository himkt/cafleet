import { useState, useEffect, useRef, useCallback } from "react";
import { Inbox, X } from "lucide-react";
import { Tabs } from "radix-ui";
import type { Agent, TimelineMessage } from "../types";
import { fetchInbox, fetchSent } from "../api";
import AgentAvatar from "./AgentAvatar";
import EmptyState from "./EmptyState";
import Skeleton from "./Skeleton";

const ROW_CAP = 200;

const STATUS_CHIPS: Record<
  TimelineMessage["status"],
  { label: string; className: string }
> = {
  input_required: { label: "pending", className: "bg-surface-hover text-text-muted" },
  completed: { label: "acked", className: "bg-success-soft text-success" },
  canceled: { label: "canceled", className: "bg-danger-soft text-danger" },
};

function StatusChip({ status }: { status: TimelineMessage["status"] }) {
  const chip = STATUS_CHIPS[status];
  return (
    <span
      className={`inline-block rounded-full px-1.5 py-0.5 text-[10px] font-medium ${chip.className}`}
    >
      {chip.label}
    </span>
  );
}

function formatDateTime(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())} · ${d.toLocaleDateString()}`;
}

function MessageList({
  rows,
  direction,
  loading,
}: {
  rows: TimelineMessage[];
  direction: "inbox" | "sent";
  loading: boolean;
}) {
  if (loading && rows.length === 0) {
    return (
      <div className="px-4 py-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="py-2">
            <Skeleton className="h-3 w-32" />
            <Skeleton className="mt-2 h-3 w-56 max-w-full" />
          </div>
        ))}
      </div>
    );
  }

  if (rows.length === 0) {
    return <EmptyState icon={Inbox} title="No messages" />;
  }

  const truncated = rows.length > ROW_CAP;
  const visible = rows.slice(0, ROW_CAP);

  return (
    <div className="divide-y divide-border">
      {visible.map((row) => (
        <div key={row.task_id} className="px-4 py-2.5">
          <div className="flex flex-wrap items-baseline gap-1.5 text-xs">
            <span className="font-medium text-text-muted">
              {direction === "inbox"
                ? `from ${row.from_agent_name}`
                : `to ${row.to_agent_name}`}
            </span>
            <StatusChip status={row.status} />
            <span className="ml-auto text-text-faint">
              {formatDateTime(row.created_at)}
            </span>
          </div>
          <p className="mt-1 whitespace-pre-wrap break-words text-sm">
            {row.body}
          </p>
        </div>
      ))}
      {truncated && (
        <p className="px-4 py-3 text-center text-xs text-text-faint">
          Showing the {ROW_CAP} most recent messages
        </p>
      )}
    </div>
  );
}

const TAB_TRIGGER_CLASS =
  "border-b-2 border-transparent px-3 py-1.5 text-sm text-text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent data-[state=active]:border-accent data-[state=active]:font-medium data-[state=active]:text-text";

interface AgentDetailProps {
  agent: Agent;
  refreshKey: number;
  onClose: () => void;
}

export default function AgentDetail({
  agent,
  refreshKey,
  onClose,
}: AgentDetailProps) {
  const [inbox, setInbox] = useState<TimelineMessage[]>([]);
  const [sent, setSent] = useState<TimelineMessage[]>([]);
  const [loading, setLoading] = useState(true);
  const inFlightRef = useRef(false);

  useEffect(() => {
    setInbox([]);
    setSent([]);
    setLoading(true);
  }, [agent.agent_id]);

  // Mirrors Timeline's pattern: refetches ride Dashboard's refreshKey bumps
  // (5 s poll / manual Refresh / post-send) instead of a second polling loop;
  // the in-flight guard absorbs bumps landing during a slow fetch.
  const load = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    try {
      const [inboxData, sentData] = await Promise.all([
        fetchInbox(agent.agent_id),
        fetchSent(agent.agent_id),
      ]);
      setInbox(inboxData.messages);
      setSent(sentData.messages);
    } catch {
      /* swallow — keep last-known lists; next bump re-attempts */
    } finally {
      setLoading(false);
      inFlightRef.current = false;
    }
  }, [agent.agent_id]);

  useEffect(() => {
    void load();
  }, [refreshKey, load]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <aside className="flex w-96 shrink-0 flex-col border-l border-border bg-surface-raised">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <AgentAvatar agent={agent} size="lg" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <h2 className="truncate text-base font-semibold">{agent.name}</h2>
            <span
              className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                agent.status === "active"
                  ? "bg-success-soft text-success"
                  : "bg-surface-hover text-text-muted"
              }`}
            >
              {agent.status === "active" ? "Active" : "Deregistered"}
            </span>
            {agent.kind === "builtin-administrator" && (
              <span className="rounded bg-accent-soft px-1.5 py-0.5 text-[10px] font-medium text-accent">
                Admin
              </span>
            )}
          </div>
          {agent.description && (
            <p className="mt-1 text-sm text-text-muted">{agent.description}</p>
          )}
          <p className="mt-1 font-mono text-xs text-text-faint">
            #{agent.agent_id} · registered{" "}
            {new Date(agent.registered_at).toLocaleString()}
          </p>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          title="Close"
          className="shrink-0 rounded-lg p-1.5 text-text-muted hover:bg-surface-hover hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>

      <Tabs.Root defaultValue="inbox" className="flex min-h-0 flex-1 flex-col">
        <Tabs.List
          aria-label="Agent messages"
          className="flex shrink-0 gap-1 border-b border-border px-4"
        >
          <Tabs.Trigger value="inbox" className={TAB_TRIGGER_CLASS}>
            Inbox
          </Tabs.Trigger>
          <Tabs.Trigger value="sent" className={TAB_TRIGGER_CLASS}>
            Sent
          </Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="inbox" className="min-h-0 flex-1 overflow-y-auto">
          <MessageList rows={inbox} direction="inbox" loading={loading} />
        </Tabs.Content>
        <Tabs.Content value="sent" className="min-h-0 flex-1 overflow-y-auto">
          <MessageList rows={sent} direction="sent" loading={loading} />
        </Tabs.Content>
      </Tabs.Root>
    </aside>
  );
}
