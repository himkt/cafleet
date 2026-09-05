// @vitest-environment jsdom
import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Timeline from "../src/components/Timeline";
import MemberDetail from "../src/components/MemberDetail";
import Dashboard from "../src/components/Dashboard";
import MessageInput from "../src/components/MessageInput";
import AppHeader from "../src/components/AppHeader";
import type { FleetClient } from "../src/api";
import type { TimelineResponse, MembersResponse } from "../src/types";
import { deferred, fakeClient, members, message, timestamp } from "./step9-fixtures";
import { domFixtures, flush } from "./step9-dom";

domFixtures();
function tab(name: "Inbox" | "Sent") { fireEvent.mouseDown(screen.getByRole("tab",{name}),{button:0,ctrlKey:false}); }
function send(text: string) {fireEvent.change(screen.getByRole("textbox"),{target:{value:text}});fireEvent.click(screen.getByRole("button",{name:"Send"}));}

describe("resource error and independence UI", () => {
  it("timeline initial error is an alert with Retry, never a successful empty state", async () => {
    const pending=deferred<TimelineResponse>();
    const load=vi.fn<FleetClient["fetchTimeline"]>().mockReturnValueOnce(pending.promise).mockResolvedValue({messages:[]});
    const client=fakeClient(1,{fetchTimeline:load});
    render(<Timeline client={client} members={members()} refreshKey={0}/>);
    expect(screen.getByLabelText(/timeline/i)).toBeTruthy();
    expect(screen.queryByText(/No messages/i)).toBeNull();
    await act(async()=>pending.reject(new Error("timeline offline")));
    expect(screen.getByRole("alert").textContent).toContain("timeline offline");
    expect(screen.queryByText(/No messages/i)).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/retry timeline/i}));await flush();
    expect(screen.getByText(/No messages/i)).toBeTruthy();expect(screen.queryByRole("alert")).toBeNull();
    expect(load).toHaveBeenCalledTimes(2);
  });
  it("timeline refresh failure preserves data and identifies update failure until retry succeeds", async () => {
    const refresh=deferred<TimelineResponse>();
    const load=vi.fn<FleetClient["fetchTimeline"]>().mockResolvedValueOnce({messages:[message(1,"cached timeline")]}).mockReturnValueOnce(refresh.promise).mockResolvedValue({messages:[message(2,"fresh timeline")]});
    const client=fakeClient(1,{fetchTimeline:load});
    const view=render(<Timeline client={client} members={members()} refreshKey={0}/>);await flush();
    view.rerender(<Timeline client={client} members={members()} refreshKey={1}/>);
    expect(screen.getByText("cached timeline")).toBeTruthy();
    await act(async()=>refresh.reject(new Error("refresh offline")));
    expect(screen.getByText("cached timeline")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toMatch(/refresh|updat/i);
    expect(screen.getByRole("alert").textContent).toContain("refresh offline");
    fireEvent.click(screen.getByRole("button",{name:/retry timeline/i}));await flush();
    expect(screen.getByText("fresh timeline")).toBeTruthy();expect(screen.queryByText("cached timeline")).toBeNull();
  });
  it("inbox delay does not block sent loading or refresh; inbox Retry does not refetch sent", async () => {
    const pending=deferred<TimelineResponse>();
    const inbox=vi.fn<FleetClient["fetchInbox"]>().mockReturnValueOnce(pending.promise).mockRejectedValueOnce(new Error("inbox update failed")).mockResolvedValue({messages:[]});
    const sent=vi.fn<FleetClient["fetchSent"]>().mockResolvedValue({messages:[message(4,"sent independently")]});
    const client=fakeClient(1,{fetchInbox:inbox,fetchSent:sent});
    const view=render(<MemberDetail client={client} member={members()[1]} refreshKey={0} onClose={vi.fn()}/>);await flush();
    tab("Sent");expect(screen.getByText("sent independently")).toBeTruthy();
    view.rerender(<MemberDetail client={client} member={members()[1]} refreshKey={1} onClose={vi.fn()}/>);await flush();
    expect(inbox).toHaveBeenCalledTimes(1);expect(sent).toHaveBeenCalledTimes(2);
    await act(async()=>pending.resolve({messages:[message(5,"cached inbox")]}));
    tab("Inbox");expect(screen.getByText("cached inbox")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("inbox update failed");
    fireEvent.click(screen.getByRole("button",{name:/retry inbox/i}));await flush();
    expect(screen.getByText("No messages")).toBeTruthy();expect(inbox).toHaveBeenCalledTimes(3);expect(sent).toHaveBeenCalledTimes(2);
  });
  it("a delayed sent request does not block inbox data or its independent refresh", async () => {
    const pending=deferred<TimelineResponse>();
    const client=fakeClient(1,{fetchInbox:vi.fn().mockResolvedValue({messages:[message(9,"inbox ready")]}),fetchSent:vi.fn().mockReturnValue(pending.promise)});
    const view=render(<MemberDetail client={client} member={members()[1]} refreshKey={0} onClose={vi.fn()}/>);await flush();
    expect(screen.getByText("inbox ready")).toBeTruthy();
    view.rerender(<MemberDetail client={client} member={members()[1]} refreshKey={1} onClose={vi.fn()}/>);await flush();
    expect(client.fetchInbox).toHaveBeenCalledTimes(2);expect(client.fetchSent).toHaveBeenCalledTimes(1);
    view.unmount();expect(vi.mocked(client.fetchSent).mock.calls[0][1]?.signal?.aborted).toBe(true);
    await act(async()=>pending.resolve({messages:[]}));expect(client.fetchSent).toHaveBeenCalledTimes(1);
  });
  it.each(["inbox","sent"] as const)("%s initial failure is independently retryable and not empty success", async (which) => {
    const failed=vi.fn().mockRejectedValueOnce(new Error(`${which} unavailable`)).mockResolvedValue({messages:[]});
    const client=fakeClient(1,which==="inbox" ? {fetchInbox:failed} : {fetchSent:failed});
    render(<MemberDetail client={client} member={members()[1]} refreshKey={0} onClose={vi.fn()}/>);await flush();
    if(which==="sent")tab("Sent");
    expect(screen.getByRole("alert").textContent).toContain(`${which} unavailable`);
    expect(screen.queryByText("No messages")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:new RegExp(`retry ${which}`,"i")}));await flush();
    expect(screen.getByText("No messages")).toBeTruthy();expect(failed).toHaveBeenCalledTimes(2);
  });
  it.each([200,201])("both member tabs retain the 200-row window and overflow-only notice for %i rows", async (count) => {
    const rows=(prefix:string)=>Array.from({length:count},(_,index)=>message(index+1,`${prefix} row ${index}`));
    const client=fakeClient(1,{fetchInbox:vi.fn().mockResolvedValue({messages:rows("inbox")}),fetchSent:vi.fn().mockResolvedValue({messages:rows("sent")})});
    render(<MemberDetail client={client} member={members()[1]} refreshKey={0} onClose={vi.fn()}/>);await flush();
    for(const direction of ["inbox","sent"] as const) {
      if(direction==="sent")tab("Sent");
      expect(screen.getAllByText(new RegExp(`^${direction} row `))).toHaveLength(200);
      expect(screen.queryByText(`${direction} row 200`)).toBeNull();
      expect(screen.queryByText("Showing the 200 most recent messages")!==null).toBe(count===201);
    }
  });
  it("monitor settles independently while members load, with no empty-fleet or missing-Director assertion", async () => {
    const pending=deferred<MembersResponse>();const client=fakeClient(1,{getMembers:vi.fn().mockReturnValue(pending.promise)});
    render(<Dashboard client={client} fleetName="Fleet 1" onBack={vi.fn()}/>);await flush();
    expect(screen.getByRole("button",{name:/Monitor running/i})).toBeTruthy();
    expect(screen.getByLabelText(/members/i)).toBeTruthy();
    expect(screen.queryByText(/No members registered/i)).toBeNull();
    expect(screen.queryByText(/no active root Director/i)).toBeNull();
    expect(screen.queryByPlaceholderText("@member or @all message...")).toBeNull();
    await act(async()=>pending.resolve({members:members()}));
    expect(screen.getByPlaceholderText("@member or @all message...")).toBeTruthy();
  });
  it("members settle and poll while monitor remains pending; monitor Retry is resource-specific", async () => {
    const pending=deferred<Awaited<ReturnType<FleetClient["getMonitor"]>>>();
    const monitor=vi.fn<FleetClient["getMonitor"]>().mockReturnValueOnce(pending.promise).mockRejectedValueOnce(new Error("monitor unavailable")).mockResolvedValue({running:true,wake_interval_seconds:600});
    const client=fakeClient(1,{getMonitor:monitor});
    render(<Dashboard client={client} fleetName="Fleet 1" onBack={vi.fn()}/>);await flush();
    expect(screen.getByPlaceholderText("@member or @all message...")).toBeTruthy();
    fireEvent.click(screen.getByRole("button",{name:"Refresh"}));await flush();
    expect(client.getMembers).toHaveBeenCalledTimes(2);expect(monitor).toHaveBeenCalledTimes(1);
    await act(async()=>pending.resolve({running:true,wake_interval_seconds:600}));
    expect(screen.getByRole("alert").textContent).toContain("monitor unavailable");
    fireEvent.click(screen.getByRole("button",{name:/retry monitor/i}));await flush();
    expect(monitor).toHaveBeenCalledTimes(3);expect(client.getMembers).toHaveBeenCalledTimes(2);
  });
  it("Dashboard polls all owned resources once per five seconds even when hidden", async () => {
    vi.useFakeTimers();vi.spyOn(document,"hidden","get").mockReturnValue(true);vi.spyOn(document,"visibilityState","get").mockReturnValue("hidden");
    const client=fakeClient();const view=render(<Dashboard client={client} fleetName="Fleet 1" onBack={vi.fn()}/>);await flush();
    for(const load of [client.getMembers,client.getMonitor,client.fetchTimeline])expect(load).toHaveBeenCalledTimes(1);
    await act(async()=>vi.advanceTimersByTime(4999));
    expect(client.getMembers).toHaveBeenCalledTimes(1);
    await act(async()=>vi.advanceTimersByTime(1));
    for(const load of [client.getMembers,client.getMonitor,client.fetchTimeline])expect(load).toHaveBeenCalledTimes(2);
    view.unmount();expect(vi.getTimerCount()).toBe(0);
  });
  it.each(["send","save","wake"] as const)("successful %s refreshes members, monitor and timeline exactly once", async (operation) => {
    const client=fakeClient();render(<Dashboard client={client} fleetName="Fleet 1" onBack={vi.fn()}/>);await flush();
    if(operation==="send")send("@worker-1 hello");
    else {
      fireEvent.click(screen.getByRole("button",{name:/Monitor running/i}));
      if(operation==="save")fireEvent.change(screen.getByRole("spinbutton"),{target:{value:"30"}});
      fireEvent.click(screen.getByRole("button",{name:operation==="save" ? "Save" : "Wake now"}));
    }
    await flush();
    for(const load of [client.getMembers,client.getMonitor,client.fetchTimeline])expect(load).toHaveBeenCalledTimes(2);
    if(operation==="send")expect(client.sendMessage).toHaveBeenCalledWith(11,12,"hello",expect.objectContaining({signal:expect.any(AbortSignal)}));
    if(operation==="save")expect(client.patchMonitor).toHaveBeenCalledWith(30,expect.objectContaining({signal:expect.any(AbortSignal)}));
    if(operation==="wake")expect(client.postMonitorWake).toHaveBeenCalledTimes(1);
  });
});

describe("mutation completion belongs to its keyed fleet", () => {
  it.each(["resolve","reject"] as const)("aborts an old send and ignores late %s without clearing the new draft or resending", async (outcome) => {
    const pending=deferred<void>();const old=fakeClient(1,{sendMessage:vi.fn().mockReturnValue(pending.promise)}), next=fakeClient(2);
    const sentA=vi.fn(),sentB=vi.fn();
    const view=render(<MessageInput key={1} client={old} senderId={11} members={members(1)} onSent={sentA}/>);
    send("@worker-1 original");await flush();
    expect(old.sendMessage).toHaveBeenCalledWith(11,12,"original",expect.objectContaining({signal:expect.any(AbortSignal)}));
    const signal=vi.mocked(old.sendMessage).mock.calls[0][3]?.signal;expect(signal?.aborted).toBe(false);
    view.rerender(<MessageInput key={2} client={next} senderId={21} members={members(2)} onSent={sentB}/>);
    expect(signal?.aborted).toBe(true);expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
    fireEvent.change(screen.getByRole("textbox"),{target:{value:"@worker-2 new draft"}});
    await act(async()=>{if(outcome==="resolve")pending.resolve(undefined);else pending.reject(new Error("old send failed"));});
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("@worker-2 new draft");
    expect(screen.queryByText("old send failed")).toBeNull();expect(sentA).not.toHaveBeenCalled();expect(sentB).not.toHaveBeenCalled();
    expect(old.sendMessage).toHaveBeenCalledTimes(1);expect(next.sendMessage).not.toHaveBeenCalled();
    send("@worker-1 cross-fleet recipient");await flush();expect(next.sendMessage).not.toHaveBeenCalled();
  });
  it.each([["save","resolve"],["save","reject"],["wake","resolve"],["wake","reject"]] as const)("aborts old monitor %s and suppresses late %s callbacks and stale edit state", async (operation, outcome) => {
    const save=deferred<void>(),wake=deferred<{wake_requested_at:string}>();
    const old=fakeClient(1,{patchMonitor:vi.fn().mockReturnValue(save.promise),postMonitorWake:vi.fn().mockReturnValue(wake.promise)}),next=fakeClient(2);
    const savedA=vi.fn(),savedB=vi.fn();
    const view=render(<AppHeader key={1} client={old} isPolling={false} onRefresh={vi.fn()} monitor={{running:true,wake_interval_seconds:600}} onMonitorSaved={savedA}/>);
    fireEvent.click(screen.getByRole("button",{name:/Monitor running/i}));fireEvent.change(screen.getByRole("spinbutton"),{target:{value:"72"}});
    fireEvent.click(screen.getByRole("button",{name:operation==="save" ? "Save" : "Wake now"}));await flush();
    const signal=operation==="save" ? vi.mocked(old.patchMonitor).mock.calls[0][1]?.signal : vi.mocked(old.postMonitorWake).mock.calls[0][0]?.signal;
    expect(signal?.aborted).toBe(false);
    view.rerender(<AppHeader key={2} client={next} isPolling={false} onRefresh={vi.fn()} monitor={{running:true,wake_interval_seconds:300}} onMonitorSaved={savedB}/>);
    expect(signal?.aborted).toBe(true);expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:/Monitor running/i}));expect((screen.getByRole("spinbutton") as HTMLInputElement).value).toBe("300");
    await act(async()=>{
      if(outcome==="reject") {if(operation==="save")save.reject(new Error("old monitor failed"));else wake.reject(new Error("old monitor failed"));}
      else if(operation==="save")save.resolve(undefined);else wake.resolve({wake_requested_at:timestamp});
    });
    expect(screen.queryByText("old monitor failed")).toBeNull();
    expect(savedA).not.toHaveBeenCalled();expect(savedB).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();expect(screen.queryByText(/Wake requested/i)).toBeNull();
    expect(next.patchMonitor).not.toHaveBeenCalled();expect(next.postMonitorWake).not.toHaveBeenCalled();
  });
});
