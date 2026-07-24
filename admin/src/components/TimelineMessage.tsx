import type { TimelineEntry, Member } from "../types";
import { entrySortKey } from "../timeline";
import { formatTime } from "../format";
import MemberAvatar from "./MemberAvatar";
import ReactionBar from "./ReactionBar";

interface TimelineMessageProps {
  entry: TimelineEntry;
  members: Member[];
}

function MentionChip({ name }: { name: string }) {
  return (
    <span className="inline-block rounded bg-accent-soft px-1.5 py-0.5 text-xs font-medium text-accent">
      @{name}
    </span>
  );
}

function firstRow(entry: TimelineEntry) {
  return entry.kind === "unicast" ? entry.message : entry.rows[0];
}

function body(entry: TimelineEntry): string {
  return firstRow(entry).body;
}

function recipientNames(entry: TimelineEntry): string[] {
  if (entry.kind === "unicast") {
    return [entry.message.to_member_name];
  }
  return entry.rows.map((r) => r.to_member_name);
}

export default function TimelineMessageComponent({
  entry,
  members,
}: TimelineMessageProps) {
  const row = firstRow(entry);
  const sender = members.find((m) => m.member_id === row.from_member_id) ?? {
    member_id: row.from_member_id,
    name: row.from_member_name,
    kind: "member" as const,
  };

  return (
    <div className="flex gap-3 px-4 py-2 hover:bg-surface-hover motion-safe:animate-rise-in">
      <MemberAvatar member={sender} size="md" />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-1.5">
          <span className="text-sm font-semibold">{row.from_member_name}</span>
          <span className="text-xs text-text-faint" aria-hidden="true">
            &rarr;
          </span>
          {recipientNames(entry).map((name, i) => (
            <MentionChip key={i} name={name} />
          ))}
          <span className="text-xs text-text-faint">
            {formatTime(entrySortKey(entry))}
          </span>
        </div>
        <p className="mt-0.5 whitespace-pre-wrap break-words text-sm">
          {body(entry)}
        </p>
        <ReactionBar entry={entry} members={members} />
      </div>
    </div>
  );
}
