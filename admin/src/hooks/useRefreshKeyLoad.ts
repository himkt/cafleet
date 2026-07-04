import { useEffect, useRef } from "react";

/**
 * Runs `load` on mount and on every `refreshKey` bump (and whenever `load`
 * itself changes — e.g. AgentDetail recreates it per agent, which is what
 * triggers the agent-switch reload). A per-component-instance in-flight ref
 * guards against overlapping runs when a bump lands during a slow fetch.
 * `load` owns its own error handling and its `setLoading(false)` finalization;
 * this hook only owns the guard and the refreshKey/load-driven trigger.
 */
export function useRefreshKeyLoad(
  load: () => Promise<void>,
  refreshKey: number,
): void {
  const inFlightRef = useRef(false);

  useEffect(() => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    void load().finally(() => {
      inFlightRef.current = false;
    });
  }, [refreshKey, load]);
}
