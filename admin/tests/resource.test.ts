import { setImmediate as flush } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createResource } from "../src/resource";
import type { Resource, ResourceState } from "../src/resource";
import { deferred } from "./step9-fixtures";

type Load = (signal: AbortSignal) => Promise<string>;
const resources: Resource<string>[] = [];
function resource(load: Load) {
  const result = createResource(load);
  resources.push(result);
  return result;
}
afterEach(() => {
  for (const current of resources.splice(0)) current.stop();
});
const loading: ResourceState<string> = {status:"loading",data:null,error:null,refreshing:false};

describe("production resource lifecycle and snapshots", () => {
  it("construction, reads and subscriptions have no fetch or notification side effect", async () => {
    const load = vi.fn<Load>();
    const current = resource(load);
    const first = current.getSnapshot();
    expect(first).toEqual(loading);
    const listener = vi.fn();
    const unsubscribe = current.subscribe(listener);
    await flush();
    expect(current.getSnapshot()).toBe(first);
    expect(load).not.toHaveBeenCalled();
    expect(listener).not.toHaveBeenCalled();
    unsubscribe(); unsubscribe();
    expect(current.getSnapshot()).toBe(first);
  });

  it("first start fetches once, repeated active starts do nothing before and after settlement", async () => {
    const pending = deferred<string>();
    const load = vi.fn<Load>().mockReturnValue(pending.promise);
    const current = resource(load);
    current.start(); current.start(); await flush();
    expect(load).toHaveBeenCalledTimes(1);
    expect(load.mock.calls[0][0]).toBeInstanceOf(AbortSignal);
    expect(load.mock.calls[0][0].aborted).toBe(false);
    const initial = current.getSnapshot();
    expect(initial).toEqual(loading);
    current.start(); expect(current.getSnapshot()).toBe(initial);
    pending.resolve("ready"); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"ready",error:null,refreshing:false});
    const success = current.getSnapshot();
    current.start(); await flush();
    expect(load).toHaveBeenCalledTimes(1); expect(current.getSnapshot()).toBe(success);
  });

  it("publishes the actual snapshot before notifying and keeps references stable between changes", async () => {
    const pending = deferred<string>();
    const current = resource(vi.fn<Load>().mockReturnValue(pending.promise));
    const observed: ResourceState<string>[] = [];
    const unsubscribe = current.subscribe(() => observed.push(current.getSnapshot()));
    current.start(); await flush(); observed.length = 0;
    pending.resolve("published"); await flush();
    const snapshot = current.getSnapshot();
    expect(observed).toContain(snapshot);
    expect(snapshot).toEqual({status:"success",data:"published",error:null,refreshing:false});
    expect(current.getSnapshot()).toBe(snapshot);
    await flush(); expect(current.getSnapshot()).toBe(snapshot);
    unsubscribe();
  });

  it("unsubscribe suppresses future notifications without unsubscribing other listeners", async () => {
    const first = deferred<string>(), second = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const current = resource(load);
    const removed = vi.fn(), retained = vi.fn();
    const unsubscribe = current.subscribe(removed);
    current.subscribe(retained); current.start(); await flush();
    unsubscribe(); const removedCount = removed.mock.calls.length;
    first.resolve("first"); await flush();
    expect(removed).toHaveBeenCalledTimes(removedCount);
    expect(retained).toHaveBeenCalled(); retained.mockClear();
    current.refresh(); await flush(); second.resolve("second"); await flush();
    expect(removed).toHaveBeenCalledTimes(removedCount);
    expect(retained).toHaveBeenCalled();
    expect(current.getSnapshot().data).toBe("second");
  });

  it("stop has no notification, invalidates before abort callbacks settle, and ignores refresh while stopped", async () => {
    const pending = deferred<string>();
    const load = vi.fn<Load>((signal) => {
      signal.addEventListener("abort", () => pending.resolve("settled synchronously by abort"), {once:true});
      return pending.promise;
    });
    const current = resource(load), listener = vi.fn();
    current.subscribe(listener); current.start(); await flush(); current.refresh();
    const count = listener.mock.calls.length;
    current.stop();
    expect(listener).toHaveBeenCalledTimes(count);
    expect(load.mock.calls[0][0].aborted).toBe(true);
    const stopped = current.getSnapshot();
    current.stop(); current.refresh(); current.refresh(); await flush();
    expect(load).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledTimes(count);
    expect(current.getSnapshot()).toBe(stopped);
  });

  it("refresh before the first start is a no-op rather than a deferred initial request", async () => {
    const load = vi.fn<Load>().mockResolvedValue("one start");
    const current = resource(load);
    current.refresh(); current.refresh(); await flush(); expect(load).not.toHaveBeenCalled();
    current.start(); await flush();
    expect(load).toHaveBeenCalledTimes(1); expect(current.getSnapshot().data).toBe("one start");
  });

  it("restarting a successful resource returns loading and obtains a fresh signal and value", async () => {
    const first = deferred<string>(), second = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const current = resource(load);
    current.start(); await flush(); first.resolve("old data"); await flush();
    current.stop(); current.start(); await flush();
    expect(current.getSnapshot()).toEqual(loading);
    expect(load).toHaveBeenCalledTimes(2);
    expect(load.mock.calls[1][0]).not.toBe(load.mock.calls[0][0]);
    expect(load.mock.calls[1][0].aborted).toBe(false);
    second.resolve("new data"); await flush(); expect(current.getSnapshot().data).toBe("new data");
  });
});

describe("production resource failure and retry", () => {
  it("initial failure retains its Error and a refresh retries with a free request guard", async () => {
    const pending = deferred<string>(), retry = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(pending.promise).mockReturnValueOnce(retry.promise);
    const current = resource(load); current.start(); await flush();
    const error = new Error("offline"); pending.reject(error); await flush();
    expect(current.getSnapshot()).toEqual({status:"error",data:null,error,refreshing:false});
    current.refresh(); await flush(); expect(load).toHaveBeenCalledTimes(2);
    retry.resolve("recovered"); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"recovered",error:null,refreshing:false});
  });

  it.each(["offline", null, undefined, {reason:"unknown failure"}])("normalizes rejected non-Error value %j", async (reason) => {
    const load = vi.fn<Load>().mockRejectedValueOnce(reason).mockResolvedValue("recovered");
    const current = resource(load); current.start(); await flush();
    expect(current.getSnapshot()).toMatchObject({status:"error",data:null,refreshing:false});
    expect(current.getSnapshot().error).toBeInstanceOf(Error);
    if (typeof reason === "string") expect(current.getSnapshot().error?.message).toContain(reason);
    current.refresh(); await flush(); expect(current.getSnapshot().data).toBe("recovered");
  });

  it("synchronous loader throw enters the same failure path without escaping start or blocking retry", async () => {
    const error = new Error("synchronous failure");
    const load = vi.fn<Load>().mockImplementationOnce(() => { throw error; }).mockResolvedValue("retried");
    const current = resource(load);
    expect(() => current.start()).not.toThrow(); await flush();
    expect(current.getSnapshot()).toEqual({status:"error",data:null,error,refreshing:false});
    current.refresh(); await flush(); expect(load).toHaveBeenCalledTimes(2);
    expect(current.getSnapshot()).toEqual({status:"success",data:"retried",error:null,refreshing:false});
  });

  it("refresh retains data, exposes its failure, and clears that failure when retry starts", async () => {
    const update = deferred<string>(), retry = deferred<string>();
    const load = vi.fn<Load>().mockResolvedValueOnce("cached").mockReturnValueOnce(update.promise).mockReturnValueOnce(retry.promise);
    const current = resource(load); current.start(); await flush();
    current.refresh(); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"cached",error:null,refreshing:true});
    const error = new Error("update failed"); update.reject(error); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"cached",error,refreshing:false});
    current.refresh(); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"cached",error:null,refreshing:true});
    retry.resolve("fresh"); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"fresh",error:null,refreshing:false});
  });

  it("initial AbortError stays loading without an error and allows a later refresh", async () => {
    const pending = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(pending.promise).mockResolvedValue("retried");
    const current = resource(load); current.start(); await flush();
    pending.reject(new DOMException("cancelled","AbortError")); await flush();
    expect(current.getSnapshot()).toEqual(loading);
    expect(load).toHaveBeenCalledTimes(1);
    current.refresh(); await flush(); expect(load).toHaveBeenCalledTimes(2);
    expect(current.getSnapshot().data).toBe("retried");
  });

  it("AbortError during refresh keeps cached data and clears refreshing with no new error", async () => {
    const pending = deferred<string>();
    const load = vi.fn<Load>().mockResolvedValueOnce("cached").mockReturnValueOnce(pending.promise).mockResolvedValue("retried");
    const current = resource(load); current.start(); await flush(); current.refresh(); await flush();
    pending.reject(new DOMException("cancelled","AbortError")); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"cached",error:null,refreshing:false});
    current.refresh(); await flush(); expect(current.getSnapshot().data).toBe("retried");
  });
});

describe("production coalescing and obsolete completion guards", () => {
  it("coalesces arbitrarily many refreshes into one latest follow-up", async () => {
    const first = deferred<string>(), second = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const current = resource(load); current.start(); await flush();
    const snapshot = current.getSnapshot();
    for (let index=0; index<20; index++) current.refresh();
    await flush(); expect(load).toHaveBeenCalledTimes(1); expect(current.getSnapshot()).toBe(snapshot);
    first.resolve("first"); await flush();
    expect(load).toHaveBeenCalledTimes(2);
    expect(current.getSnapshot()).toEqual({status:"success",data:"first",error:null,refreshing:true});
    second.resolve("latest"); await flush();
    expect(load).toHaveBeenCalledTimes(2);
    expect(current.getSnapshot()).toEqual({status:"success",data:"latest",error:null,refreshing:false});
  });

  it.each(["error","abort"] as const)("settlement by %s still starts one queued refresh", async (kind) => {
    const first = deferred<string>(), second = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const current = resource(load); current.start(); await flush(); current.refresh(); current.refresh();
    first.reject(kind === "error" ? new Error("failed") : new DOMException("cancelled","AbortError")); await flush();
    expect(load).toHaveBeenCalledTimes(2); expect(load.mock.calls[1][0].aborted).toBe(false);
    second.resolve("recovered"); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"recovered",error:null,refreshing:false});
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("an earlier request finally cannot unlock its same-generation pending successor", async () => {
    const first = deferred<string>(), second = deferred<string>(), third = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockReturnValueOnce(third.promise);
    const current = resource(load); current.start(); await flush(); current.refresh();
    first.resolve("first"); await flush(); expect(load).toHaveBeenCalledTimes(2);
    current.refresh(); current.refresh(); await flush();
    expect(load).toHaveBeenCalledTimes(2); expect(load.mock.calls[1][0].aborted).toBe(false);
    second.resolve("second"); await flush(); expect(load).toHaveBeenCalledTimes(3);
    third.resolve("third"); await flush(); expect(load).toHaveBeenCalledTimes(3);
    expect(current.getSnapshot()).toEqual({status:"success",data:"third",error:null,refreshing:false});
  });

  it.each(["resolve","reject"] as const)("stopped request ignoring abort cannot %s into state, listeners or queued work", async (kind) => {
    const pending = deferred<string>(); const load = vi.fn<Load>().mockReturnValue(pending.promise);
    const current = resource(load), listener = vi.fn(); current.subscribe(listener);
    current.start(); await flush(); current.refresh(); current.stop();
    const snapshot = current.getSnapshot(), count = listener.mock.calls.length;
    expect(load.mock.calls[0][0].aborted).toBe(true);
    if (kind === "resolve") pending.resolve("late"); else pending.reject(new Error("late failure"));
    await flush();
    expect(current.getSnapshot()).toBe(snapshot); expect(listener).toHaveBeenCalledTimes(count);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it.each(["resolve","reject"] as const)("old %s/finally cannot unlock or restart the new start generation", async (kind) => {
    const old = deferred<string>(), currentRequest = deferred<string>(), latest = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(old.promise).mockReturnValueOnce(currentRequest.promise).mockReturnValueOnce(latest.promise);
    const current = resource(load), listener = vi.fn(); current.subscribe(listener);
    current.start(); await flush(); current.refresh(); current.stop(); current.start(); await flush();
    current.refresh(); current.refresh();
    const snapshot = current.getSnapshot(), count = listener.mock.calls.length;
    if (kind === "resolve") old.resolve("old"); else old.reject(new Error("old failure"));
    await flush();
    expect(current.getSnapshot()).toBe(snapshot); expect(listener).toHaveBeenCalledTimes(count);
    expect(load).toHaveBeenCalledTimes(2); expect(load.mock.calls[1][0].aborted).toBe(false);
    current.refresh(); await flush();
    expect(load).toHaveBeenCalledTimes(2);
    currentRequest.resolve("new generation"); await flush(); expect(load).toHaveBeenCalledTimes(3);
    expect(current.getSnapshot()).toMatchObject({data:"new generation",refreshing:true});
    latest.resolve("latest"); await flush();
    expect(current.getSnapshot()).toEqual({status:"success",data:"latest",error:null,refreshing:false});
    expect(load).toHaveBeenCalledTimes(3);
  });

  it("stop discards old pending work instead of replaying it after restart", async () => {
    const old = deferred<string>(), next = deferred<string>();
    const load = vi.fn<Load>().mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    const current = resource(load); current.start(); await flush(); current.refresh(); current.stop();
    current.start(); await flush(); next.resolve("new"); await flush(); old.resolve("old"); await flush();
    expect(load).toHaveBeenCalledTimes(2);
    expect(current.getSnapshot()).toEqual({status:"success",data:"new",error:null,refreshing:false});
  });

  it("independent resources settle and refresh without sharing state, controllers or pending flags", async () => {
    const slow = deferred<string>(), fastUpdate = deferred<string>();
    const loadA = vi.fn<Load>().mockReturnValue(slow.promise);
    const loadB = vi.fn<Load>().mockResolvedValueOnce("B ready").mockReturnValueOnce(fastUpdate.promise);
    const a = resource(loadA), b = resource(loadB);
    a.start(); b.start(); await flush();
    expect(a.getSnapshot()).toEqual(loading); expect(b.getSnapshot().data).toBe("B ready");
    expect(loadA.mock.calls[0][0]).not.toBe(loadB.mock.calls[0][0]);
    b.refresh(); await flush(); expect(loadA).toHaveBeenCalledTimes(1); expect(loadB).toHaveBeenCalledTimes(2);
    slow.reject(new Error("A unavailable")); await flush();
    expect(a.getSnapshot().status).toBe("error"); expect(b.getSnapshot()).toMatchObject({data:"B ready",error:null,refreshing:true});
    const bSnapshot = b.getSnapshot(); a.stop(); await flush();
    expect(b.getSnapshot()).toBe(bSnapshot); expect(loadB.mock.calls[1][0].aborted).toBe(false);
    fastUpdate.resolve("B updated"); await flush(); expect(b.getSnapshot().data).toBe("B updated");
  });

  it("a newly constructed resource never inherits another instance's cached data", async () => {
    const a = resource(vi.fn<Load>().mockResolvedValue("A data")); a.start(); await flush();
    const loadB = vi.fn<Load>(), b = resource(loadB);
    expect(a.getSnapshot().data).toBe("A data"); expect(b.getSnapshot()).toEqual(loading);
    expect(loadB).not.toHaveBeenCalled();
  });
});
