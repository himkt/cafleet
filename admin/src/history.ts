import type { FormattedMessage, TimelineMessage } from "./types";

export const HISTORY_ROW_CAP = 200;

export interface HistoryWindow {
  visible: TimelineMessage[];
  truncated: boolean;
}

export function selectHistory(rows: readonly FormattedMessage[]): HistoryWindow {
  const visible: TimelineMessage[] = [];
  for (const row of rows) {
    if (row.type !== "unicast") continue;
    if (visible.length === HISTORY_ROW_CAP) {
      return { visible, truncated: true };
    }
    visible.push(row);
  }
  return { visible, truncated: false };
}
