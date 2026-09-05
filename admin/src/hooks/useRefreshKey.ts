import { useCallback, useEffect, useState } from "react";
export const POLL_INTERVAL_MS = 5000;
export function useRefreshKey(intervalMs = POLL_INTERVAL_MS) {
  const [refreshKey, setRefreshKey] = useState(0);
  const refresh = useCallback(() => setRefreshKey((key) => key + 1), []);
  useEffect(() => {
    const timer = setInterval(refresh, intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs, refresh]);
  return { refreshKey, refresh };
}
