import { Tooltip } from "radix-ui";
import type { TimelineEntry, Member, TimelineMessage } from "../types";

interface ReactionBarProps {
  entry: TimelineEntry;
  members: Member[];
}

function formatIso(iso: string): string {
  return new Date(iso).toISOString();
}

function memberLabel(memberId: number, members: Member[]): string {
  const member = members.find((m) => m.member_id === memberId);
  if (!member) return String(memberId);
  const suffix = member.status === "deregistered" ? " (deregistered)" : "";
  return `@${member.name}${suffix}`;
}

function getCompletedRows(entry: TimelineEntry): TimelineMessage[] {
  if (entry.kind === "unicast") {
    return entry.message.status === "completed" ? [entry.message] : [];
  }
  return entry.rows.filter((r) => r.status === "completed");
}

export default function ReactionBar({ entry, members }: ReactionBarProps) {
  const completedRows = getCompletedRows(entry);

  if (completedRows.length === 0) return null;

  return (
    <Tooltip.Provider delayDuration={200}>
      <div className="mt-1 flex flex-wrap gap-1">
        {completedRows.map((row) => (
          <Tooltip.Root key={row.message_id}>
            <Tooltip.Trigger className="cursor-default rounded-full bg-success-soft px-2 py-0.5 text-xs font-medium text-success focus-visible:outline-2 focus-visible:outline-accent">
              [ack]
            </Tooltip.Trigger>
            <Tooltip.Portal>
              <Tooltip.Content
                side="top"
                align="start"
                sideOffset={4}
                className="z-10 rounded-md bg-text px-2 py-1 text-xs text-surface shadow-md"
              >
                {memberLabel(row.to_member_id, members)} —{" "}
                <span className="font-mono">
                  {formatIso(row.status_timestamp)}
                </span>
              </Tooltip.Content>
            </Tooltip.Portal>
          </Tooltip.Root>
        ))}
      </div>
    </Tooltip.Provider>
  );
}
