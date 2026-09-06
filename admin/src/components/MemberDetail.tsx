import { useEffect, useCallback } from "react";
import { Inbox, X } from "lucide-react";
import { Tabs } from "radix-ui";
import type { Member, TimelineMessage } from "../types";
import { ApiError } from "../api";
import type { FleetClient } from "../api";
import { HISTORY_ROW_CAP, selectHistory } from "../history";
import type { HistoryWindow } from "../history";
import { useResource } from "../hooks/useResource";
import ResourceNotice from "./ResourceNotice";
import { formatDateTime } from "../format";
import MemberAvatar from "./MemberAvatar";
import EmptyState from "./EmptyState";

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
}: {
  history: HistoryWindow;
  direction: "inbox" | "sent";
}) {
  const { visible, truncated } = history;
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

export interface MemberDetailProps {
  client: FleetClient;
  member: Member;
  refreshKey: number;
  onClose: () => void;
}

export default function MemberDetail({
  client,
  member,
  refreshKey,
  onClose,
}: MemberDetailProps) {
  const id = member.member_id;
  const loadInbox = useCallback((signal: AbortSignal) => client.fetchInbox(id, { signal }).then(({ messages }) => selectHistory(messages)), [client, id]);
  const loadSent = useCallback((signal: AbortSignal) => client.fetchSent(id, { signal }).then(({ messages }) => selectHistory(messages)), [client, id]);
  const inbox = useResource({ key: `${client.fleetId}:${id}:inbox`, load: loadInbox, refreshKey });
  const sent = useResource({ key: `${client.fleetId}:${id}:sent`, load: loadSent, refreshKey });

  useEffect(() => {
    if ([inbox.state.error, sent.state.error].some((error) =>
      error instanceof ApiError && error.status === 404 && error.message === "Fleet not found")) {
      window.location.replace("#/fleets");
    }
  }, [inbox.state.error, sent.state.error]);

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
          <ResourceNotice state={inbox.state} name="inbox" retry={inbox.refresh} />
          {inbox.state.status === "success" && <MessageList history={inbox.state.data} direction="inbox" />}
        </Tabs.Content>
        <Tabs.Content value="sent" className="min-h-0 flex-1 overflow-y-auto">
          <ResourceNotice state={sent.state} name="sent" retry={sent.refresh} />
          {sent.state.status === "success" && <MessageList history={sent.state.data} direction="sent" />}
        </Tabs.Content>
      </Tabs.Root>
    </aside>
  );
}
