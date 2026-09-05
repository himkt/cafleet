// @vitest-environment jsdom
import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { deferred, fleets, members, message } from "./step9-fixtures";
import { domFixtures, flush, navigate } from "./step9-dom";

domFixtures();
interface Call { url:string; fleet:string|null; method:string; signal:AbortSignal|null|undefined; body:BodyInit|null|undefined }
function json(data:unknown,status=200) {return new Response(JSON.stringify(data),{status});}
function network(override:(call:Call)=>Promise<Response>|undefined=()=>undefined) {
  const calls:Call[]=[];
  vi.stubGlobal("fetch",vi.fn<typeof globalThis.fetch>((input,options)=>{
    const url=typeof input==="string" ? input : input instanceof URL ? input.toString() : input.url;
    const call:Call={url,fleet:new Headers(options?.headers).get("X-Fleet-Id"),method:options?.method??"GET",signal:options?.signal,body:options?.body};
    calls.push(call);
    const result=override(call);if(result)return result;
    const fleet=Number(call.fleet);
    if(url==="/api/fleets")return Promise.resolve(json(fleets()));
    if(url==="/api/members")return Promise.resolve(json({members:members(fleet)}));
    if(url==="/api/monitor")return Promise.resolve(json({running:true,wake_interval_seconds:600}));
    if(url==="/api/timeline")return Promise.resolve(json({messages:[message(1,`timeline fleet ${fleet}`,fleet)]}));
    if(/\/members\/\d+\/(inbox|sent)\?limit=201$/.test(url))return Promise.resolve(json({messages:[message(2,`history fleet ${fleet}`,fleet)]}));
    throw new Error(`unexpected request ${call.method} ${url}`);
  }));
  return calls;
}
function start(hash:string) {window.history.replaceState(null,"",`/${hash}`);render(<App/>);}
async function ready(fleet:number) {await screen.findByPlaceholderText("@member or @all message...");expect(screen.getByText(`Fleet ${fleet}`)).toBeTruthy();}
function rosterCalls(calls:Call[],fleet:number) {return calls.filter(c=>c.url==="/api/members"&&c.fleet===String(fleet));}

describe("App uses URL authority and a single roster owner", () => {
  it("picker selection navigates and performs only one initial dashboard roster fetch", async () => {
    const calls=network();render(<App/>);
    fireEvent.click(await screen.findByRole("button",{name:/Fleet 1/}));
    await ready(1);expect(window.location.hash).toBe("#/fleets/1/members");
    expect(rosterCalls(calls,1)).toHaveLength(1);
    expect(calls.filter(c=>c.url==="/api/fleets").every(c=>c.fleet===null)).toBe(true);
    expect(calls.filter(c=>c.url==="/api/timeline")).toHaveLength(1);
  });
  it("StrictMode has only one non-aborted initial roster attempt", async () => {
    const calls=network();window.history.replaceState(null,"","/#/fleets/1/members");
    render(<StrictMode><App/></StrictMode>);await ready(1);
    expect(rosterCalls(calls,1).filter(call=>!call.signal?.aborted)).toHaveLength(1);
  });
  it("deep links and member changes preserve Dashboard and avoid repeated fleet or roster prefetch", async () => {
    const calls=network();start("#/fleets/1/members/00012");await ready(1);
    await screen.findByRole("button",{name:"Close"});
    expect(calls.filter(c=>c.url==="/api/members/12/inbox?limit=201")).toHaveLength(1);
    await navigate("#/fleets/1/members/13");await flush();
    expect(rosterCalls(calls,1)).toHaveLength(1);
    expect(calls.filter(c=>c.url==="/api/fleets")).toHaveLength(1);
    expect(calls.filter(c=>c.url==="/api/members/13/inbox?limit=201")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button",{name:"Close"}));
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets/1/members"));
    expect(rosterCalls(calls,1)).toHaveLength(1);
  });
  it("Back and Forward re-establish the URL fleet without stale selection state", async () => {
    const calls=network();start("#/fleets/1/members/12");await ready(1);
    await navigate("#/fleets/2/members");await ready(2);
    act(()=>window.history.back());
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets/1/members/12"));await ready(1);
    act(()=>window.history.forward());
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets/2/members"));await ready(2);
    expect(rosterCalls(calls,1)).toHaveLength(2);expect(rosterCalls(calls,2)).toHaveLength(2);
  });
  it("invalid fleet and unknown path are normalized to the picker without scoped requests", async () => {
    const calls=network();start("#/fleets/1e0/members");
    await screen.findByRole("heading",{name:"Select a Fleet"});
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets"));
    expect(calls.some(c=>c.fleet!==null)).toBe(false);
    await navigate("#/unknown/path");await waitFor(()=>expect(window.location.hash).toBe("#/fleets"));
    expect(calls.some(c=>c.fleet!==null)).toBe(false);
  });
  it.each(["0","+12","1.2e1","12.0"," 12","9007199254740992","999"])("invalid or foreign member %s returns to its fleet dashboard without history requests", async (member) => {
    const calls=network();start(`#/fleets/1/members/${member}`);await ready(1);
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets/1/members"));
    expect(calls.some(c=>/\/(inbox|sent)\?/.test(c.url))).toBe(false);
  });
  it("does not redirect a deep-linked member while roster is loading or failed", async () => {
    const pending=deferred<Response>();let first=true;
    const calls=network(c=>{if(c.url==="/api/members"&&first){first=false;return pending.promise;}});
    start("#/fleets/1/members/12");await flush();
    expect(window.location.hash).toBe("#/fleets/1/members/12");
    expect(calls.some(c=>/\/(inbox|sent)\?/.test(c.url))).toBe(false);
    await act(async()=>pending.reject(new TypeError("members offline")));
    expect(screen.getByRole("alert").textContent).toContain("members offline");
    expect(window.location.hash).toBe("#/fleets/1/members/12");
    expect(screen.queryByText(/No members registered/i)).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/retry members/i}));await ready(1);
    await screen.findByRole("button",{name:"Close"});
    expect(calls.filter(c=>c.url==="/api/members/12/inbox?limit=201")).toHaveLength(1);
  });
  it.each(["network","parse","server"] as const)("fleet confirmation %s failure keeps the deep link and exposes Retry", async (kind) => {
    let first=true;network(c=>{
      if(c.url!=="/api/fleets"||!first)return;first=false;
      if(kind==="network")return Promise.reject(new TypeError("fleet list offline"));
      if(kind==="parse")return Promise.resolve(new Response("not JSON"));
      return Promise.resolve(json({error:"fleet list unavailable"},503));
    });
    start("#/fleets/1/members/12");await screen.findByRole("alert");
    expect(window.location.hash).toBe("#/fleets/1/members/12");
    expect(screen.queryByText("No fleets found.")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/retry fleets/i}));await ready(1);
  });
  it("a successful fleet list without the requested fleet returns to the picker", async () => {
    let first=true;const calls=network(c=>{if(c.url==="/api/fleets"&&first){first=false;return Promise.resolve(json([]));}});
    start("#/fleets/1/members");await screen.findByRole("heading",{name:"Select a Fleet"});
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets"));expect(rosterCalls(calls,1)).toHaveLength(0);
  });
  it("current scoped 404 Fleet not found returns to picker after deletion", async () => {
    network(c=>c.url==="/api/members" ? Promise.resolve(json({error:"Fleet not found"},404)) : undefined);
    start("#/fleets/1/members");await screen.findByRole("heading",{name:"Select a Fleet"});
    await waitFor(()=>expect(window.location.hash).toBe("#/fleets"));
  });
  it.each([
    ["/api/members",404,"Member not found","members"],
    ["/api/monitor",404,"Monitor not found","monitor"],
    ["/api/members",500,"Fleet not found","members"],
  ] as const)("%s HTTP %i %s is a resource error, not fleet deletion", async (url,status,error,resource) => {
    network(c=>c.url===url ? Promise.resolve(json({error},status)) : undefined);
    start("#/fleets/1/members");await screen.findByRole("button",{name:new RegExp(`retry ${resource}`,"i")});
    expect(window.location.hash).toBe("#/fleets/1/members");
    expect(screen.queryByRole("heading",{name:"Select a Fleet"})).toBeNull();
  });
  it("reverse fleet-list responses cannot restore an old fleet name or start its roster", async () => {
    const a=deferred<Response>(),b=deferred<Response>();let lists=0;
    const calls=network(c=>{if(c.url==="/api/fleets")return ++lists===1 ? a.promise : b.promise;});
    start("#/fleets/1/members");await waitFor(()=>expect(lists).toBe(1));
    await navigate("#/fleets/2/members");await waitFor(()=>expect(lists).toBe(2));
    expect(calls.filter(c=>c.url==="/api/fleets")[0].signal?.aborted).toBe(true);
    await act(async()=>b.resolve(json(fleets())));await ready(2);
    await act(async()=>a.resolve(json(fleets())));
    expect(window.location.hash).toBe("#/fleets/2/members");expect(screen.getByText("Fleet 2")).toBeTruthy();
    expect(screen.queryByText("Fleet 1")).toBeNull();expect(rosterCalls(calls,1)).toHaveLength(0);expect(rosterCalls(calls,2)).toHaveLength(1);
  });
  it("ignores a late fleet-deleted 404 after switching to a healthy fleet", async () => {
    const pending=deferred<Response>();const calls=network(c=>c.url==="/api/members"&&c.fleet==="1" ? pending.promise : undefined);
    start("#/fleets/1/members");await waitFor(()=>expect(rosterCalls(calls,1)).toHaveLength(1));
    await navigate("#/fleets/2/members");await ready(2);
    expect(rosterCalls(calls,1)[0].signal?.aborted).toBe(true);
    await act(async()=>pending.resolve(json({error:"Fleet not found"},404)));
    expect(window.location.hash).toBe("#/fleets/2/members");expect(screen.getByText("Fleet 2")).toBeTruthy();expect(rosterCalls(calls,2)).toHaveLength(1);
  });
  it("clears old fleet name, recipient candidates and draft while the new roster loads", async () => {
    const pending=deferred<Response>();network(c=>c.url==="/api/members"&&c.fleet==="2" ? pending.promise : undefined);
    start("#/fleets/1/members");await ready(1);
    fireEvent.change(screen.getByRole("textbox"),{target:{value:"@worker-1 old draft"}});
    await navigate("#/fleets/2/members");await flush();
    expect(screen.queryByText("Fleet 1")).toBeNull();expect(screen.queryByText("Worker 1")).toBeNull();
    expect(screen.queryByDisplayValue("@worker-1 old draft")).toBeNull();
    expect(screen.queryByPlaceholderText("@member or @all message...")).toBeNull();
    await act(async()=>pending.resolve(json({members:members(2)})));await ready(2);
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
    fireEvent.change(screen.getByRole("textbox"),{target:{value:"@",selectionStart:1}});
    expect(screen.queryByText("@worker-1")).toBeNull();expect(screen.getByText("@worker-2")).toBeTruthy();
  });
  it("old send keeps its original fleet/header/body and cannot refresh the new fleet on completion", async () => {
    const pending=deferred<Response>();const calls=network(c=>c.url==="/api/messages/send" ? pending.promise : undefined);
    start("#/fleets/1/members");await ready(1);
    fireEvent.change(screen.getByRole("textbox"),{target:{value:"@worker-1 hello"}});fireEvent.click(screen.getByRole("button",{name:"Send"}));await flush();
    const send=calls.find(c=>c.url==="/api/messages/send");expect(send).toMatchObject({fleet:"1",method:"POST",body:'{"from_member_id":11,"to_member_id":12,"text":"hello"}'});
    await navigate("#/fleets/2/members");await ready(2);expect(send?.signal?.aborted).toBe(true);
    await act(async()=>pending.resolve(json({message_id:5})));
    expect(rosterCalls(calls,2)).toHaveLength(1);expect(calls.filter(c=>c.url==="/api/messages/send")).toHaveLength(1);
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
  });
  it("picker initial failure does not also show an empty success state and can retry", async () => {
    let first=true;network(c=>{if(c.url==="/api/fleets"&&first){first=false;return Promise.reject(new Error("picker offline"));}});
    render(<App/>);await screen.findByRole("alert");expect(screen.queryByText("No fleets found.")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/retry fleets/i}));await screen.findByRole("button",{name:/Fleet 1/});
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
