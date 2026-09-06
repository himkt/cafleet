import type { FormattedMessage, TimelineEntry, TimelineMessage } from "./types";

export function entrySortKey(entry: TimelineEntry): string {
  return entry.kind === "unicast" ? entry.message.created_at : entry.sortKey;
}

export function groupMessages(msgs: readonly FormattedMessage[]): TimelineEntry[] {
  const groups = new Map<number, TimelineMessage[]>();
  const singletons: TimelineEntry[] = [];

  for (const m of msgs) {
    if (m.type !== "unicast") continue;
    if (m.origin_message_id === null) {
      singletons.push({ kind: "unicast", message: m });
      continue;
    }
    const existing = groups.get(m.origin_message_id);
    if (existing) {
      existing.push(m);
    } else {
      groups.set(m.origin_message_id, [m]);
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
