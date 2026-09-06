import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, createFleetClient, listFleets } from "../src/api";
import type { FleetClient } from "../src/api";
import { deferred } from "./step9-fixtures";

afterEach(() => vi.unstubAllGlobals());
const methods: { name: string; path: string; verb: string; body?: unknown; response: unknown; result: unknown;
  call: (client: FleetClient, signal: AbortSignal) => Promise<unknown> }[] = [
  { name: "members", path: "/members", verb: "GET", response: {members: []}, result: {members: []}, call: (c,s) => c.getMembers({signal:s}) },
  { name: "timeline", path: "/timeline", verb: "GET", response: {messages: []}, result: {messages: []}, call: (c,s) => c.fetchTimeline({signal:s}) },
  { name: "inbox", path: "/members/42/inbox", verb: "GET", response: {messages: []}, result: {messages: []}, call: (c,s) => c.fetchInbox(42,{signal:s}) },
  { name: "sent", path: "/members/42/sent", verb: "GET", response: {messages: []}, result: {messages: []}, call: (c,s) => c.fetchSent(42,{signal:s}) },
  { name: "monitor", path: "/monitor", verb: "GET", response: {running:true,wake_interval_seconds:600}, result: {running:true,wake_interval_seconds:600}, call: (c,s) => c.getMonitor({signal:s}) },
  { name: "send", path: "/messages/send", verb: "POST", body: {from_member_id:11,to_member_id:12,text:"雪\nmessage"}, response: {message_id:7}, result: undefined, call: (c,s) => c.sendMessage(11,12,"雪\nmessage",{signal:s}) },
  { name: "patch", path: "/monitor", verb: "PATCH", body: {wake_interval_seconds:0}, response: {ok:true}, result: undefined, call: (c,s) => c.patchMonitor(0,{signal:s}) },
  { name: "wake", path: "/monitor/wake", verb: "POST", response: {wake_requested_at:"exact timestamp"}, result: {wake_requested_at:"exact timestamp"}, call: (c,s) => c.postMonitorWake({signal:s}) },
];
describe("explicit immutable fleet client", () => {
  it.each(methods)("$name preserves request and response contracts", async (entry) => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(new Response(JSON.stringify(entry.response)));
    vi.stubGlobal("fetch", fetch);
    const controller = new AbortController();
    const result = await entry.call(createFleetClient(7),controller.signal);
    expect(result).toEqual(entry.result);
    expect(fetch).toHaveBeenCalledTimes(1);
    const [path, options] = fetch.mock.calls[0];
    expect(path).toBe(`/api${entry.path}`);
    expect(options?.method ?? "GET").toBe(entry.verb);
    expect(options?.signal).toBe(controller.signal);
    expect(new Headers(options?.headers).get("X-Fleet-Id")).toBe("7");
    expect(new Headers(options?.headers).get("Content-Type")).toBe(entry.body === undefined ? null : "application/json");
    expect(options?.body).toBe(entry.body === undefined ? undefined : JSON.stringify(entry.body));
  });
  it("listFleets is always unscoped and forwards the exact signal", async () => {
    const client = createFleetClient(9);
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValueOnce(new Response('{"members":[]}')).mockResolvedValueOnce(new Response("[]"));
    vi.stubGlobal("fetch",fetch);
    await client.getMembers();
    const signal = new AbortController().signal;
    expect(await listFleets({signal})).toEqual([]);
    const [url, options] = fetch.mock.calls[1];
    expect(url).toBe("/api/fleets");
    expect(new Headers(options?.headers).has("X-Fleet-Id")).toBe(false);
    expect(options?.signal).toBe(signal);
  });
  it("A and B clients retain their IDs through concurrent reverse completions", async () => {
    const a = createFleetClient(1), b = createFleetClient(2);
    const first = deferred<Response>(), second = deferred<Response>();
    const fetch = vi.fn<typeof globalThis.fetch>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockResolvedValue(new Response("{}"));
    vi.stubGlobal("fetch",fetch);
    const pa = a.getMembers(), pb = b.getMembers();
    second.resolve(new Response('{"members":["B"]}'));
    expect(await pb).toEqual({members:["B"]});
    first.resolve(new Response('{"members":["A"]}'));
    expect(await pa).toEqual({members:["A"]});
    await a.sendMessage(11,"*","broadcast");
    expect(fetch.mock.calls.map(([,o]) => new Headers(o?.headers).get("X-Fleet-Id"))).toEqual(["1","2","1"]);
    expect(fetch.mock.calls[2][1]?.body).toBe('{"from_member_id":11,"to_member_id":"*","text":"broadcast"}');
    expect(a.fleetId).toBe(1); expect(b.fleetId).toBe(2);
  });
  it("rejects invalid fleet numbers before performing any fetch", () => {
    const fetch = vi.fn(); vi.stubGlobal("fetch",fetch);
    for (const id of [0,-1,0.5,NaN,Infinity,-Infinity,Number.MAX_SAFE_INTEGER+1]) {
      expect(() => createFleetClient(id)).toThrow(new RangeError("Invalid fleet ID"));
    }
    expect(createFleetClient(Number.MAX_SAFE_INTEGER).fleetId).toBe(Number.MAX_SAFE_INTEGER);
    expect(fetch).not.toHaveBeenCalled();
  });
  it.each([
    [404,{error:"Fleet not found",detail:"ignored"},"Fleet not found"],
    [404,{detail:"Member not found"},"Member not found"],
    [422,{error:"",detail:"invalid interval"},"invalid interval"],
    [503,{},"HTTP 503"],
    [500,"not JSON","HTTP 500"],
  ] as const)("preserves HTTP status %i and error/detail/fallback precedence", async (status,body,text) => {
    vi.stubGlobal("fetch",vi.fn().mockResolvedValue(new Response(typeof body === "string" ? body : JSON.stringify(body),{status})));
    const error = await createFleetClient(1).getMembers().catch((error: unknown) => error);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({status,message:text});
  });
  it("passes network and AbortError objects through without wrapping or retrying", async () => {
    for (const error of [new TypeError("offline"),new DOMException("aborted","AbortError")]) {
      const fetch=vi.fn().mockRejectedValue(error); vi.stubGlobal("fetch",fetch);
      await expect(createFleetClient(1).getMembers()).rejects.toBe(error);
      expect(fetch).toHaveBeenCalledTimes(1);
    }
  });
  it("does not misclassify a successful HTTP response with invalid JSON as a missing fleet", async () => {
    vi.stubGlobal("fetch",vi.fn().mockResolvedValue(new Response("invalid JSON")));
    const error=await createFleetClient(1).getMembers().catch((error:unknown)=>error);
    expect(error).toBeInstanceOf(SyntaxError);
    expect(error).not.toBeInstanceOf(ApiError);
  });
});
