export type ResourceState<T> =
  | { status: "loading"; data: null; error: null; refreshing: false }
  | { status: "error"; data: null; error: Error; refreshing: false }
  | { status: "success"; data: T; error: Error | null; refreshing: boolean };

export interface Resource<T> {
  getSnapshot(): ResourceState<T>;
  subscribe(listener: () => void): () => void;
  start(): void;
  refresh(): void;
  stop(): void;
}

const loading = <T>(): ResourceState<T> => ({
  status: "loading", data: null, error: null, refreshing: false,
});

/** Owns one resource's requests; React only subscribes and connects its lifetime. */
export function createResource<T>(load: (signal: AbortSignal) => Promise<T>): Resource<T> {
  let snapshot = loading<T>();
  const listeners = new Set<() => void>();
  let active = false;
  let generation = 0;
  let request: AbortController | null = null;
  let pending = false;
  const publish = (next: ResourceState<T>) => {
    snapshot = next;
    listeners.forEach((listener) => listener());
  };
  const refresh = () => {
    if (!active) return;
    if (request) { pending = true; return; }
    const controller = new AbortController();
    const epoch = generation;
    request = controller;
    const current = () => active && generation === epoch && request === controller;
    publish(snapshot.status === "success"
      ? { ...snapshot, error: null, refreshing: true }
      : loading<T>());
    void (async () => {
      try {
        if (!current()) return;
        const data = await load(controller.signal);
        if (current()) publish({ status: "success", data, error: null, refreshing: false });
      } catch (reason) {
        if (!current()) return;
        const error = reason instanceof Error ? reason : new Error(String(reason));
        if (error.name === "AbortError") {
          publish(snapshot.status === "success" ? { ...snapshot, error: null, refreshing: false } : loading<T>());
        } else {
          publish(snapshot.status === "success"
            ? { ...snapshot, error, refreshing: false }
            : { status: "error", data: null, error, refreshing: false });
        }
      } finally {
        if (current()) {
          request = null;
          if (pending) { pending = false; refresh(); }
        }
      }
    })();
  };
  return {
    getSnapshot: () => snapshot,
    subscribe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    start() {
      if (active) return;
      active = true;
      generation++;
      snapshot = loading<T>();
      refresh();
    },
    refresh,
    stop() {
      active = false;
      generation++;
      pending = false;
      const previous = request;
      request = null;
      previous?.abort();
    },
  };
}
