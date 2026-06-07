import { useState, useEffect, useCallback } from "react";
import type { FleetListItem } from "../types";
import { listFleets } from "../api";
import { usePolling, POLL_INTERVAL_MS } from "../hooks/usePolling";

interface FleetPickerProps {
  onSelect: (fleetId: number) => void;
}

export default function FleetPicker({ onSelect }: FleetPickerProps) {
  const [fleets, setFleets] = useState<FleetListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isPolling, setIsPolling] = useState(false);

  const loadFleets = useCallback(async () => {
    setIsPolling(true);
    try {
      const data = await listFleets();
      setFleets(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load fleets");
    } finally {
      setLoading(false);
      setIsPolling(false);
    }
  }, []);

  const trigger = usePolling(loadFleets, POLL_INTERVAL_MS);

  useEffect(() => {
    void trigger();
  }, [trigger]);

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col">
      <header className="bg-white border-b border-gray-200 px-4 py-3">
        <h1 className="text-lg font-semibold text-gray-900">
          CAFleet — Fleets
        </h1>
      </header>

      <div className="flex-1 max-w-2xl w-full mx-auto mt-4 px-4">
        {error && (
          <div className="bg-red-50 text-red-700 text-sm rounded-md px-4 py-2 mb-4">
            {error}
          </div>
        )}

        <div className="bg-white rounded-lg shadow-sm border border-gray-200 overflow-hidden">
          <div className="px-4 py-3 border-b border-gray-200 flex items-center justify-between gap-3">
            <h2 className="text-sm font-medium text-gray-700">
              Select a Fleet
            </h2>
            {isPolling && (
              <span className="text-xs text-gray-400 italic">Updating…</span>
            )}
          </div>

          {loading ? (
            <p className="text-center text-gray-400 py-8">Loading...</p>
          ) : fleets.length === 0 ? (
            <div className="text-center py-8 px-4">
              <p className="text-gray-400 text-sm">No fleets found.</p>
              <p className="text-gray-400 text-xs mt-2">
                Run{" "}
                <code className="bg-gray-100 px-1.5 py-0.5 rounded text-gray-600">
                  cafleet fleet create
                </code>{" "}
                to create one.
              </p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200">
              {fleets.map((s) => (
                <button
                  key={s.fleet_id}
                  onClick={() => onSelect(s.fleet_id)}
                  className="w-full px-4 py-3 flex items-center justify-between gap-3 hover:bg-gray-50 text-left"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <code className="text-sm font-mono text-gray-900">
                        {s.fleet_id}
                      </code>
                      {s.label && (
                        <span className="text-sm text-gray-600 truncate">
                          {s.label}
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-gray-400 mt-0.5">
                      {s.agent_count} agent{s.agent_count !== 1 ? "s" : ""} |
                      Created{" "}
                      {new Date(s.created_at).toLocaleDateString()}
                    </p>
                  </div>
                  <span className="text-gray-400 text-sm shrink-0">&rarr;</span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
