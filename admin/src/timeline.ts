import type { TimelineEntry } from "./types";

export function entrySortKey(entry: TimelineEntry): string {
  return entry.kind === "unicast" ? entry.message.created_at : entry.sortKey;
}
