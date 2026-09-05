import { useState, useEffect, useCallback } from "react";
import { Inbox, X } from "lucide-react";
import { Tabs } from "radix-ui";
import type { Member, TimelineMessage } from "../types";
import { fetchInbox, fetchSent } from "../api";
import { HISTORY_ROW_CAP, selectHistory } from "../history";
import type { HistoryWindow } from "../history";
import { useRefreshKeyLoad } from "../hooks/useRefreshKeyLoad";
import { formatDateTime } from "../format";
import MemberAvatar from "./MemberAvatar";
import EmptyState from "./EmptyState";
import Skeleton from "./Skeleton";

const STATUS_CHIPS: Record<
  TimelineMessage["status"],
  { label: string; className: string }
> = {
  input_required: { label: "pending", className: "bg-surface-hover text-text-muted" },
  completed: { label: "acked", className: "bg-success-soft text-success" },
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
  history,
  direction,
  loading,
}: {
  history: HistoryWindow;
  direction: "inbox" | "sent";
  loading: boolean;
}) {
  const { visible, truncated } = history;
  if (loading && visible.length === 0) {
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

  if (visible.length === 0) {
    return <EmptyState icon={Inbox} title="No messages" />;
  }

  return (
    <div className="divide-y divide-border">
      {visible.map((row) => (
        <div key={row.message_id} className="px-4 py-2.5">
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
          Showing the {HISTORY_ROW_CAP} most recent messages
        </p>
      )}
    </div>
  );
}

const TAB_TRIGGER_CLASS =
  "border-b-2 border-transparent px-3 py-1.5 text-sm text-text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent data-[state=active]:border-accent data-[state=active]:font-medium data-[state=active]:text-text";

interface MemberDetailProps {
  member: Member;
  refreshKey: number;
  onClose: () => void;
}

export default function MemberDetail({
  member,
  refreshKey,
  onClose,
}: MemberDetailProps) {
  const [inbox, setInbox] = useState<HistoryWindow>(() => selectHistory([]));
  const [sent, setSent] = useState<HistoryWindow>(() => selectHistory([]));
  const [loading, setLoading] = useState(true);

  // The panel is keyed by member_id at its call site, so switching members
  // remounts it with fresh empty/loading state — no reset effect needed.

  // Refetches ride Dashboard's refreshKey bumps (5 s poll / manual Refresh /
  // post-send) via useRefreshKeyLoad instead of a second polling loop; the
  // hook's in-flight guard absorbs bumps landing during a slow fetch, and its
  // mount-time run fetches the freshly keyed member.
  const load = useCallback(async () => {
    try {
      const [inboxData, sentData] = await Promise.all([
        fetchInbox(member.member_id),
        fetchSent(member.member_id),
      ]);
      setInbox(selectHistory(inboxData.messages));
      setSent(selectHistory(sentData.messages));
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
          <MessageList history={inbox} direction="inbox" loading={loading} />
        </Tabs.Content>
        <Tabs.Content value="sent" className="min-h-0 flex-1 overflow-y-auto">
          <MessageList history={sent} direction="sent" loading={loading} />
        </Tabs.Content>
      </Tabs.Root>
    </aside>
  );
}
