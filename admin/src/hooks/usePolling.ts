import { useEffect, useRef } from "react";

export const POLL_INTERVAL_MS = 5000;

/**
 * Calls `callback` every `intervalMs`. Skips a tick if the previous call is
 * still pending (in-flight guard). Always polls — does NOT pause on
 * document visibility changes. Errors thrown by `callback` are swallowed
 * (caller is responsible for surfacing them through component state).
 */
export function usePolling(
  callback: () => Promise<void>,
  intervalMs: number,
): void {
  const savedCallback = useRef(callback);
  const inFlight = useRef(false);

  useEffect(() => {
    savedCallback.current = callback;
  }, [callback]);

  useEffect(() => {
    const tick = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        await savedCallback.current();
      } catch {
        /* swallow — next tick re-attempts */
      } finally {
        inFlight.current = false;
      }
    };

    const timer = setInterval(() => {
      void tick();
    }, intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
}
