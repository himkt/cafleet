import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchInbox, fetchSent, setFleetId } from "../src/api";
import type { TimelineResponse } from "../src/types";

const endpoints = [
  ["inbox", fetchInbox],
  ["sent", fetchSent],
] as const;

beforeEach(() => setFleetId(null));
afterEach(() => {
  vi.unstubAllGlobals();
  setFleetId(null);
});

describe("bounded member history requests", () => {
  it.each(endpoints)("fetches %s with limit 201 and the selected fleet", async (endpoint, load) => {
    const payload: TimelineResponse = {
      messages: [{
        message_id: 9,
        from_member_id: 1,
        from_member_name: "Director",
        to_member_id: 42,
        to_member_name: "Worker",
        type: "unicast",
        status: "completed",
        created_at: "raw-created",
        status_timestamp: "raw-status",
        origin_message_id: null,
        body: "unchanged body",
      }],
    };
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(payload)));
    vi.stubGlobal("fetch", fetch);
    setFleetId(7);
    expect(await load(42)).toEqual(payload);
    expect(fetch).toHaveBeenCalledExactlyOnceWith(
      `/api/members/42/${endpoint}?limit=201`,
      { headers: { "X-Fleet-Id": "7" } },
    );
  });

  it.each(endpoints)("preserves %s response errors and omits an unset fleet header", async (_endpoint, load) => {
    const fetch = vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ detail: "limit must be an integer between 1 and 1000" }),
      { status: 422 },
    ));
    vi.stubGlobal("fetch", fetch);
    await expect(load(42)).rejects.toThrow("limit must be an integer between 1 and 1000");
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch.mock.calls[0][1]).toEqual({ headers: {} });
  });
});
