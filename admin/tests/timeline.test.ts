import { describe, expect, it } from "vitest";
import { entrySortKey, groupMessages } from "../src/timeline";
import type { FormattedMessage, TimelineEntry } from "../src/types";

type Delivery = Extract<FormattedMessage, { type: "unicast" }>;
type Summary = Extract<FormattedMessage, { type: "broadcast_summary" }>;
const ts = "2026-09-05T10:00:00.000000+00:00";

function delivery(
  id: number,
  recipient: number,
  origin: number | null,
  status: Delivery["status"] = "input_required",
  createdAt = ts,
): Delivery {
  return {
    message_id: id,
    from_member_id: 1,
    from_member_name: "Director",
    to_member_id: recipient,
    to_member_name: `member-${recipient}`,
    type: "unicast",
    status,
    created_at: createdAt,
    status_timestamp: ts,
    origin_message_id: origin,
    body: "work",
  };
}

function summary(id: number, origin: number | null = id): Summary {
  return {
    message_id: id,
    from_member_id: 1,
    from_member_name: "Director",
    to_member_id: null,
    to_member_name: null,
    type: "broadcast_summary",
    status: "completed",
    created_at: ts,
    status_timestamp: ts,
    origin_message_id: origin,
    body: "Broadcast sent to 2 recipients",
  };
}

function onlyBroadcast(entries: TimelineEntry[]) {
  expect(entries).toHaveLength(1);
  const entry = entries[0];
  if (entry.kind !== "broadcast") throw new Error("expected one broadcast group");
  return entry;
}

function ids(entry: TimelineEntry): number[] {
  return entry.kind === "unicast"
    ? [entry.message.message_id]
    : entry.rows.map((row) => row.message_id).sort((a, b) => a - b);
}

describe("delivery-only timeline grouping", () => {
  it("drops summary-only input with either self origin or null origin", () => {
    expect(groupMessages([summary(10), summary(20, null)])).toEqual([]);
  });

  it.each([0, 1, 2])(
    "counts two fetched recipients and %i ACKs despite a completed summary",
    (acked) => {
      const rows: FormattedMessage[] = [
        delivery(11, 2, 10, acked >= 1 ? "completed" : "input_required"),
        summary(10),
        delivery(12, 3, 10, acked >= 2 ? "completed" : "input_required"),
      ];
      const group = onlyBroadcast(groupMessages(rows));
      expect(ids(group)).toEqual([11, 12]);
      expect(group.rows.map((row) => row.to_member_id).sort()).toEqual([2, 3]);
      expect(group.rows.map((row) => row.to_member_name).sort()).toEqual([
        "member-2",
        "member-3",
      ]);
      expect(group.rows.every((row) => row.type === "unicast")).toBe(true);
      expect(group.rows.filter((row) => row.status === "completed")).toHaveLength(acked);
    },
  );

  it("keeps different broadcast origins separate even for equal bodies and senders", () => {
    const entries = groupMessages([
      delivery(11, 2, 10),
      delivery(21, 2, 20),
      summary(10),
      summary(20),
      delivery(12, 3, 10),
    ]);
    expect(entries).toHaveLength(2);
    expect(entries.map(ids).sort((a, b) => a[0] - b[0])).toEqual([[11, 12], [21]]);
  });

  it("sorts UI entries by creation time using the earliest fetched broadcast delivery", () => {
    const early = "2026-01-01T00:00:00.000000+00:00";
    const middle = "2026-02-01T00:00:00.000000+00:00";
    const late = "2026-03-01T00:00:00.000000+00:00";
    const single = delivery(90, 2, null, "completed", middle);
    single.status_timestamp = "2099-01-01T00:00:00.000000+00:00";
    const bookkeeping = summary(10);
    bookkeeping.created_at = "2000-01-01T00:00:00.000000+00:00";
    const entries = groupMessages([
      single,
      delivery(12, 3, 10, "input_required", late),
      bookkeeping,
      delivery(11, 2, 10, "input_required", early),
      delivery(91, 2, null, "input_required", late),
    ]);
    expect(entries.map(ids)).toEqual([[11, 12], [90], [91]]);
    expect(entries.map(entrySortKey)).toEqual([early, middle, late]);
  });

  it("counts only fetched rows in a partial broadcast without inventing missing recipients", () => {
    const fetched = delivery(12, 3, 10, "completed");
    const bookkeeping = summary(10);
    bookkeeping.body = "Broadcast sent to 1000 recipients";
    const group = onlyBroadcast(groupMessages([bookkeeping, fetched]));
    expect(ids(group)).toEqual([12]);
    expect(group.rows).toHaveLength(1);
    expect(group.rows.filter((row) => row.status === "completed")).toHaveLength(1);
    expect(group.rows[0].to_member_name).toBe("member-3");
  });

});
