import { afterEach, beforeEach, vi } from "vitest";
import { act, cleanup } from "@testing-library/react";

export function domFixtures() {
  let scroll: PropertyDescriptor | undefined;
  beforeEach(() => {
    scroll = Object.getOwnPropertyDescriptor(Element.prototype,"scrollIntoView");
    Object.defineProperty(Element.prototype,"scrollIntoView",{configurable:true,value:vi.fn()});
    vi.stubGlobal("matchMedia",vi.fn((query: string) => ({matches:false,media:query,onchange:null,
      addListener:vi.fn(),removeListener:vi.fn(),addEventListener:vi.fn(),removeEventListener:vi.fn(),dispatchEvent:vi.fn(()=>true)})));
    window.history.replaceState(null,"","/#/fleets");
    localStorage.clear();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    if (scroll) Object.defineProperty(Element.prototype,"scrollIntoView",scroll);
    else Reflect.deleteProperty(Element.prototype,"scrollIntoView");
  });
}
export async function flush() { await act(async () => {}); }
export async function navigate(hash: string) {
  await act(async () => {
    window.history.pushState(null,"",`/${hash}`);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}
