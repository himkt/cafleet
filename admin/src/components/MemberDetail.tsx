import { useState, useEffect, useCallback } from "react";
import { Activity, Inbox, X } from "lucide-react";
import { Tabs } from "radix-ui";
import type { Member, TimelineMessage } from "../types";
import { fetchInbox, fetchSent, updateMemberMonitor } from "../api";
import { useRefreshKeyLoad } from "../hooks/useRefreshKeyLoad";
import { formatDateTime } from "../format";
import MemberAvatar from "./MemberAvatar";
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
                ? `from ${row.from_member_name}`
                : `to ${row.to_member_name}`}
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

function MonitoringSection({
  member,
  onChanged,
}: {
  member: Member;
  onChanged: () => void;
}) {
  const monitor = member.monitor;
  const monitorInterval = monitor?.interval_seconds ?? null;
  const [intervalInput, setIntervalInput] = useState(
    monitorInterval !== null ? String(monitorInterval) : "",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-sync the input only when the polled interval value changes (or when
  // switching members) — keying on the scalar, not the per-poll object identity,
  // so a 5 s refresh never clobbers an in-progress edit.
  useEffect(() => {
    if (monitorInterval !== null) setIntervalInput(String(monitorInterval));
  }, [monitorInterval, member.member_id]);

  if (monitor === null) return null;

  const saveInterval = async () => {
    const n = Number(intervalInput);
    if (!Number.isInteger(n) || n < 1) {
      setError("Interval must be a whole number ≥ 1.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await updateMemberMonitor(member.member_id, { interval_seconds: n });
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to update interval.");
    } finally {
      setBusy(false);
    }
  };

  const toggleEnabled = async () => {
    setBusy(true);
    setError(null);
    try {
      await updateMemberMonitor(member.member_id, { enabled: !monitor.enabled });
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to toggle monitoring.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="border-b border-border px-4 py-3">
      <div className="mb-2 flex items-center gap-1.5">
        <Activity size={14} className="text-text-muted" aria-hidden="true" />
        <h3 className="text-xs font-semibold uppercase tracking-wider text-text-muted">
          Monitoring
        </h3>
        <span
          className={`ml-auto rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
            monitor.enabled
              ? "bg-success-soft text-success"
              : "bg-surface-hover text-text-muted"
          }`}
        >
          {monitor.enabled ? "Enabled" : "Disabled"}
        </span>
      </div>
      <div className="flex items-end gap-2">
        <label className="flex flex-col gap-1 text-xs text-text-muted">
          Interval (s)
          <input
            type="number"
            min={1}
            value={intervalInput}
            onChange={(e) => setIntervalInput(e.target.value)}
            disabled={busy}
            className="w-20 rounded border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/30 disabled:opacity-50"
          />
        </label>
        <button
          type="button"
          onClick={() => {
            void saveInterval();
          }}
          disabled={busy}
          className="rounded-lg bg-accent px-2.5 py-1 text-sm font-medium text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
        >
          Save
        </button>
        <button
          type="button"
          onClick={() => {
            void toggleEnabled();
          }}
          disabled={busy}
          className="rounded-lg border border-border px-2.5 py-1 text-sm text-text-muted hover:bg-surface-hover hover:text-text focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
        >
          {monitor.enabled ? "Disable" : "Enable"}
        </button>
      </div>
      <p className="mt-2 text-xs text-text-faint">
        Last ping:{" "}
        {monitor.last_ping_at !== null
          ? formatDateTime(monitor.last_ping_at)
          : "never"}
      </p>
      {error !== null && <p className="mt-1 text-xs text-danger">{error}</p>}
    </section>
  );
}

interface MemberDetailProps {
  member: Member;
  refreshKey: number;
  onClose: () => void;
  onChanged: () => void;
}

export default function MemberDetail({
  member,
  refreshKey,
  onClose,
  onChanged,
}: MemberDetailProps) {
  const [inbox, setInbox] = useState<TimelineMessage[]>([]);
  const [sent, setSent] = useState<TimelineMessage[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setInbox([]);
    setSent([]);
    setLoading(true);
  }, [member.member_id]);

  // Refetches ride Dashboard's refreshKey bumps (5 s poll / manual Refresh /
  // post-send) via useRefreshKeyLoad instead of a second polling loop; the
  // hook's in-flight guard absorbs bumps landing during a slow fetch, and its
  // `load` dep triggers the reload when the member switches.
  const load = useCallback(async () => {
    try {
      const [inboxData, sentData] = await Promise.all([
        fetchInbox(member.member_id),
        fetchSent(member.member_id),
      ]);
      // The endpoints are unbounded; keep ROW_CAP + 1 rows so the
      // "Showing the 200 most recent" footer still knows about the overflow.
      setInbox(inboxData.messages.slice(0, ROW_CAP + 1));
      setSent(sentData.messages.slice(0, ROW_CAP + 1));
    } catch {
      /* swallow — keep last-known lists; next bump re-attempts */
    } finally {
      setLoading(false);
    }
  }, [member.member_id]);

  useRefreshKeyLoad(load, refreshKey);

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
        <MemberAvatar member={member} size="lg" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <h2 className="truncate text-base font-semibold">{member.name}</h2>
            <span
              className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                member.status === "active"
                  ? "bg-success-soft text-success"
                  : "bg-surface-hover text-text-muted"
              }`}
            >
              {member.status === "active" ? "Active" : "Deregistered"}
            </span>
          </div>
          {member.description && (
            <p className="mt-1 text-sm text-text-muted">{member.description}</p>
          )}
          <p className="mt-1 font-mono text-xs text-text-faint">
            #{member.member_id} · registered{" "}
            {new Date(member.registered_at).toLocaleString()}
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

      {/* key by member_id so switching members remounts the section and resets
          its transient busy/error/interval-input state (no leak across members). */}
      <MonitoringSection
        key={member.member_id}
        member={member}
        onChanged={onChanged}
      />

      <Tabs.Root defaultValue="inbox" className="flex min-h-0 flex-1 flex-col">
        <Tabs.List
          aria-label="Member messages"
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
