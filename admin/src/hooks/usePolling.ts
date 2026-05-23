import { useCallback, useEffect, useRef } from "react";

export const POLL_INTERVAL_MS = 5000;

/**
 * Calls `callback` every `intervalMs` and also returns a guarded `trigger()`
 * callers can invoke for one-off refreshes (initial load, manual Refresh,
 * post-send, refreshKey bumps). All invocations — timer ticks AND `trigger`
 * calls — share a single in-flight ref, so concurrent calls to `callback`
 * never overlap regardless of which path initiated them. Errors thrown by
 * `callback` are swallowed (caller surfaces them through component state).
 * Polling does NOT pause on document visibility changes.
 */
export function usePolling(
  callback: () => Promise<void>,
  intervalMs: number,
): () => Promise<void> {
  const savedCallback = useRef(callback);
  const inFlight = useRef(false);

  useEffect(() => {
    savedCallback.current = callback;
  }, [callback]);

  const trigger = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      await savedCallback.current();
    } catch {
      /* swallow — next tick re-attempts */
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    const timer = setInterval(() => {
      void trigger();
    }, intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs, trigger]);

  return trigger;
}
