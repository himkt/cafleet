// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useResource } from "../src/hooks/useResource";
import type { ResourceOptions } from "../src/hooks/useResource";
import { useRefreshKey, POLL_INTERVAL_MS } from "../src/hooks/useRefreshKey";
import { deferred } from "./step9-fixtures";
import { domFixtures, flush } from "./step9-dom";

domFixtures();
function useTextResource(options: ResourceOptions<string>) { return useResource(options); }
function options(load: (signal: AbortSignal) => Promise<string>,key="A",refreshKey=0) { return {key,load,refreshKey}; }

describe("real resource hook generations", () => {
  it("starts exactly once with loading state, a live signal, and stable refresh identity", async () => {
    const pending=deferred<string>(); const load=vi.fn().mockReturnValue(pending.promise);
    const view=renderHook(useTextResource,{initialProps:options(load)});
    const refresh=view.result.current.refresh;
    expect(view.result.current.state).toEqual({status:"loading",data:null,error:null,refreshing:false});
    expect(load).toHaveBeenCalledTimes(1);
    expect(load.mock.calls[0][0]).toBeInstanceOf(AbortSignal);
    expect(load.mock.calls[0][0].aborted).toBe(false);
    view.rerender(options(load)); expect(load).toHaveBeenCalledTimes(1);
    await act(async()=>pending.resolve("ready"));
    expect(view.result.current.state).toEqual({status:"success",data:"ready",error:null,refreshing:false});
    expect(view.result.current.refresh).toBe(refresh);
  });
  it("normalizes non-Error initial failures and allows retry without a stuck guard", async () => {
    const first=deferred<string>(); const load=vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue("recovered");
    const view=renderHook(useTextResource,{initialProps:options(load)});
    await act(async()=>first.reject("offline"));
    expect(view.result.current.state).toMatchObject({status:"error",data:null,refreshing:false});
    expect(view.result.current.state.error).toBeInstanceOf(Error);
    expect(view.result.current.state.error?.message).toContain("offline");
    await act(async()=>view.result.current.refresh());
    expect(load).toHaveBeenCalledTimes(2);
    expect(view.result.current.state).toMatchObject({status:"success",data:"recovered",error:null});
  });
  it("retains same-generation data during refresh and after failure, then clears the error on retry", async () => {
    const update=deferred<string>(), retry=deferred<string>();
    const load=vi.fn().mockResolvedValueOnce("cached").mockReturnValueOnce(update.promise).mockReturnValueOnce(retry.promise);
    const view=renderHook(useTextResource,{initialProps:options(load)}); await flush();
    act(()=>view.result.current.refresh());
    expect(view.result.current.state).toEqual({status:"success",data:"cached",error:null,refreshing:true});
    const error=new Error("refresh unavailable"); await act(async()=>update.reject(error));
    expect(view.result.current.state).toEqual({status:"success",data:"cached",error,refreshing:false});
    act(()=>view.result.current.refresh());
    expect(view.result.current.state).toEqual({status:"success",data:"cached",error:null,refreshing:true});
    await act(async()=>retry.resolve("new"));
    expect(view.result.current.state).toEqual({status:"success",data:"new",error:null,refreshing:false});
  });
  it("returns loading from the very first render of a new key and aborts old refresh", async () => {
    const late=deferred<string>(), next=deferred<string>();
    const loadA=vi.fn().mockResolvedValueOnce("A data").mockReturnValueOnce(late.promise);
    const loadB=vi.fn().mockReturnValue(next.promise);
    const renders=vi.fn();
    const view=renderHook(function useObserved(props:ResourceOptions<string>) {
      const resource=useResource(props); renders(props.key,resource.state); return resource;
    },{initialProps:options(loadA)});
    await flush(); act(()=>view.result.current.refresh());
    renders.mockClear(); view.rerender(options(loadB,"B"));
    expect(renders.mock.calls[0]).toEqual(["B",{status:"loading",data:null,error:null,refreshing:false}]);
    expect(loadA.mock.calls[1][0].aborted).toBe(true);
    await act(async()=>next.resolve("B data"));
    await act(async()=>late.resolve("late A"));
    expect(view.result.current.state).toMatchObject({data:"B data",error:null});
  });
  it("treats a changed load identity as a new generation even with the same key", async () => {
    const a=deferred<string>(), b=deferred<string>();
    const loadA=vi.fn().mockReturnValue(a.promise), loadB=vi.fn().mockReturnValue(b.promise);
    const view=renderHook(useTextResource,{initialProps:options(loadA)});
    view.rerender(options(loadB)); expect(loadA.mock.calls[0][0].aborted).toBe(true);
    await act(async()=>b.resolve("new loader")); await act(async()=>a.resolve("old loader"));
    expect(view.result.current.state).toMatchObject({data:"new loader"});
  });
  it.each(["resolve","reject"] as const)("ignores old %s/finally without unlocking or restarting the current request", async (outcome) => {
    const a=deferred<string>(), b=deferred<string>(), follow=deferred<string>();
    const loadA=vi.fn().mockReturnValue(a.promise), loadB=vi.fn().mockReturnValueOnce(b.promise).mockReturnValueOnce(follow.promise);
    const view=renderHook(useTextResource,{initialProps:options(loadA)});
    act(()=>view.result.current.refresh()); // obsolete generation's pending request
    view.rerender(options(loadB,"B"));
    act(()=>{view.result.current.refresh();view.result.current.refresh();});
    await act(async()=>{if(outcome==="resolve") a.resolve("old");else a.reject(new Error("old failure"));});
    expect(view.result.current.state).toMatchObject({status:"loading",data:null,error:null});
    expect(loadB).toHaveBeenCalledTimes(1);
    expect(loadB.mock.calls[0][0].aborted).toBe(false);
    await act(async()=>b.resolve("B first"));
    expect(loadB).toHaveBeenCalledTimes(2);
    expect(view.result.current.state).toMatchObject({data:"B first",refreshing:true});
    await act(async()=>follow.resolve("B latest"));
    expect(view.result.current.state).toMatchObject({data:"B latest",refreshing:false});
    expect(loadA).toHaveBeenCalledTimes(1);
  });
  it("discards the old generation pending refresh instead of replaying it in the new key", async () => {
    const a=deferred<string>(),b=deferred<string>();
    const loadA=vi.fn().mockReturnValue(a.promise),loadB=vi.fn().mockReturnValue(b.promise);
    const view=renderHook(useTextResource,{initialProps:options(loadA)});
    act(()=>view.result.current.refresh());
    view.rerender(options(loadB,"B"));
    await act(async()=>b.resolve("B"));await act(async()=>a.resolve("late A"));await flush();
    expect(loadA).toHaveBeenCalledTimes(1);expect(loadB).toHaveBeenCalledTimes(1);
    expect(view.result.current.state).toMatchObject({data:"B",error:null,refreshing:false});
  });
  it("coalesces multiple refreshKey changes and manual refreshes into one follow-up", async () => {
    const first=deferred<string>(), second=deferred<string>();
    const load=vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const view=renderHook(useTextResource,{initialProps:options(load)});
    view.rerender(options(load,"A",1));view.rerender(options(load,"A",2));
    act(()=>{view.result.current.refresh();view.result.current.refresh();});
    expect(load).toHaveBeenCalledTimes(1);
    await act(async()=>first.resolve("first")); expect(load).toHaveBeenCalledTimes(2);
    await act(async()=>second.resolve("latest")); await flush();
    expect(load).toHaveBeenCalledTimes(2);
    expect(view.result.current.state).toMatchObject({data:"latest",refreshing:false});
  });
  it.each(["error","abort"] as const)("drains a pending refresh after current %s", async (kind) => {
    const pending=deferred<string>();const load=vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue("recovered");
    const view=renderHook(useTextResource,{initialProps:options(load)});
    act(()=>view.result.current.refresh());
    await act(async()=>pending.reject(kind==="abort" ? new DOMException("cancelled","AbortError") : new Error("offline")));
    expect(load).toHaveBeenCalledTimes(2);
    expect(view.result.current.state).toMatchObject({status:"success",data:"recovered",error:null,refreshing:false});
  });
  it("current AbortError preserves the display and permits a later refresh", async () => {
    const aborted=deferred<string>();const load=vi.fn().mockResolvedValueOnce("cached").mockReturnValueOnce(aborted.promise).mockResolvedValue("retried");
    const view=renderHook(useTextResource,{initialProps:options(load)});await flush();
    act(()=>view.result.current.refresh());await act(async()=>aborted.reject(new DOMException("cancelled","AbortError")));
    expect(view.result.current.state.data).toBe("cached");expect(view.result.current.state.error).toBeNull();
    expect(load).toHaveBeenCalledTimes(2);
    await act(async()=>view.result.current.refresh());expect(view.result.current.state.data).toBe("retried");
  });
  it.each(["resolve","reject"] as const)("aborts on unmount and discards pending refresh before late %s", async (kind) => {
    const pending=deferred<string>();const load=vi.fn().mockReturnValue(pending.promise);
    const renders=vi.fn();
    const view=renderHook(function useObserved() {const resource=useResource(options(load));renders(resource.state);return resource;});
    act(()=>view.result.current.refresh());view.unmount();
    expect(load.mock.calls[0][0].aborted).toBe(true);const count=renders.mock.calls.length;
    await act(async()=>{if(kind==="resolve")pending.resolve("late");else pending.reject(new Error("late failure"));});
    expect(load).toHaveBeenCalledTimes(1);expect(renders).toHaveBeenCalledTimes(count);
  });
});

describe("refresh timer has no request state", () => {
  it("starts at zero, ticks at five seconds even hidden, and supports functional manual increments", () => {
    vi.useFakeTimers();vi.spyOn(document,"visibilityState","get").mockReturnValue("hidden");vi.spyOn(document,"hidden","get").mockReturnValue(true);
    expect(POLL_INTERVAL_MS).toBe(5000);
    const view=renderHook(useRefreshKey);const refresh=view.result.current.refresh;
    expect(view.result.current.refreshKey).toBe(0);
    act(()=>vi.advanceTimersByTime(4999));expect(view.result.current.refreshKey).toBe(0);
    act(()=>vi.advanceTimersByTime(1));expect(view.result.current.refreshKey).toBe(1);
    act(()=>{view.result.current.refresh();view.result.current.refresh();});expect(view.result.current.refreshKey).toBe(3);
    act(()=>vi.advanceTimersByTime(5000));expect(view.result.current.refreshKey).toBe(4);
    expect(view.result.current.refresh).toBe(refresh);
    view.unmount();expect(vi.getTimerCount()).toBe(0);
  });
  it("supports an explicit interval without an initial increment", () => {
    vi.useFakeTimers();const view=renderHook(function useCustomTimer(){return useRefreshKey(75);});
    expect(view.result.current.refreshKey).toBe(0);
    act(()=>vi.advanceTimersByTime(225));expect(view.result.current.refreshKey).toBe(3);
    view.unmount();expect(vi.getTimerCount()).toBe(0);
  });
});
