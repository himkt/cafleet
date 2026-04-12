import { useState, useEffect } from "react";
import type { SessionListItem } from "../types";
import { listSessions } from "../api";

interface SessionPickerProps {
  onSelect: (sessionId: string) => void;
}

export default function SessionPicker({ onSelect }: SessionPickerProps) {
  const [sessions, setSessions] = useState<SessionListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listSessions()
      .then(setSessions)
      .catch((err) =>
        setError(err instanceof Error ? err.message : "Failed to load sessions"),
      )
      .finally(() => setLoading(false));
  }, []);

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col">
      <header className="bg-white border-b border-gray-200 px-4 py-3">
        <h1 className="text-lg font-semibold text-gray-900">
          CAFleet — Sessions
        </h1>
      </header>

      <div className="flex-1 max-w-2xl w-full mx-auto mt-4 px-4">
        {error && (
          <div className="bg-red-50 text-red-700 text-sm rounded-md px-4 py-2 mb-4">
            {error}
          </div>
        )}

        <div className="bg-white rounded-lg shadow-sm border border-gray-200 overflow-hidden">
          <div className="px-4 py-3 border-b border-gray-200">
            <h2 className="text-sm font-medium text-gray-700">
              Select a Session
            </h2>
          </div>

          {loading ? (
            <p className="text-center text-gray-400 py-8">Loading...</p>
          ) : sessions.length === 0 ? (
            <div className="text-center py-8 px-4">
              <p className="text-gray-400 text-sm">No sessions found.</p>
              <p className="text-gray-400 text-xs mt-2">
                Run{" "}
                <code className="bg-gray-100 px-1.5 py-0.5 rounded text-gray-600">
                  cafleet-registry session create
                </code>{" "}
                to create one.
              </p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200">
              {sessions.map((s) => (
                <button
                  key={s.session_id}
                  onClick={() => onSelect(s.session_id)}
                  className="w-full px-4 py-3 flex items-center justify-between gap-3 hover:bg-gray-50 text-left"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <code className="text-sm font-mono text-gray-900">
                        {s.session_id.slice(0, 8)}
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
