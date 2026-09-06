import { describe, expect, it } from "vitest";
import { selectHistory } from "../src/history";
import type { HistoryWindow } from "../src/history";
import type { FormattedMessage, TimelineMessage } from "../src/types";

function delivery(id: number): TimelineMessage {
  return {
    message_id: id,
    from_member_id: 1,
    from_member_name: "Director",
    to_member_id: 2,
    to_member_name: "Worker",
    type: "unicast",
    status: id % 2 ? "completed" : "input_required",
    created_at: `created-${id}`,
    status_timestamp: "equal-status-time",
    origin_message_id: id % 2 ? null : 0,
    body: "body",
  };
}

function summary(): FormattedMessage {
  return {
    ...delivery(9999),
    type: "broadcast_summary",
    to_member_id: null,
    to_member_name: null,
    status: "completed",
  };
}

describe("member history display window", () => {
  it.each([0, 200, 201, 1205])("selects up to 200 of %i deliveries and reports actual truncation", (count) => {
    const rows = Array.from({ length: count }, (_, i) => delivery(count - i));
    const result: HistoryWindow = selectHistory(rows);
    expect(result.visible).toEqual(rows.slice(0, 200));
    expect(result.truncated).toBe(count > 200);
  });

  it("does not count summaries as hidden or visible deliveries", () => {
    expect(selectHistory([summary(), summary()])).toEqual({ visible: [], truncated: false });
    const rows = Array.from({ length: 200 }, (_, i) => delivery(i));
    expect(selectHistory([summary(), ...rows, summary()])).toEqual({ visible: rows, truncated: false });
    expect(selectHistory([summary(), ...rows, summary(), delivery(201)])).toEqual({ visible: rows, truncated: true });
  });

  it("preserves fetched order and fields without sorting by creation time or mutating input", () => {
    const rows: FormattedMessage[] = [delivery(3), summary(), delivery(1), delivery(2)];
    const before = structuredClone(rows);
    rows.forEach(Object.freeze);
    Object.freeze(rows);
    const result = selectHistory(rows);
    expect(result.visible).toEqual([before[0], before[2], before[3]]);
    expect(result.visible.every((row) => row.type === "unicast")).toBe(true);
    expect(result.truncated).toBe(false);
    expect(rows).toEqual(before);
  });
});
