import { useState, useCallback } from "react";
import type { Agent } from "../types";
import { getAgents } from "../api";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";

interface DashboardProps {
  sessionId: string;
  initialAgents: Agent[];
  onBack: () => void;
}

export default function Dashboard({
  sessionId,
  initialAgents,
  onBack,
}: DashboardProps) {
  const [agents, setAgents] = useState<Agent[]>(initialAgents);
  const [refreshKey, setRefreshKey] = useState(0);

  const refreshAll = useCallback(async () => {
    try {
      const data = await getAgents();
      setAgents(data.agents);
    } catch {
      /* preserve last-known agent list */
    }
    setRefreshKey((k) => k + 1);
  }, []);

  const administrator =
    agents.find((a) => a.kind === "builtin-administrator") ?? null;
  const senderId =
    administrator?.status === "active" ? administrator.agent_id : null;

  return (
    <div className="h-screen flex flex-col bg-gray-50">
      <header className="bg-white border-b border-gray-200 px-4 py-2 flex items-center justify-between shrink-0">
        <h1 className="text-lg font-semibold text-gray-900">
          CAFleet —{" "}
          <span className="font-mono text-sm text-gray-500">
            {sessionId.slice(0, 8)}
          </span>
        </h1>
        <div className="flex items-center gap-3">
          {senderId !== null && (
            <span className="text-sm text-gray-700">
              Sending as{" "}
              <span className="font-medium text-gray-900">Administrator</span>
            </span>
          )}
          <button
            onClick={refreshAll}
            className="text-xs text-gray-500 hover:text-gray-700"
          >
            Refresh
          </button>
          <button
            onClick={onBack}
            className="text-sm text-gray-500 hover:text-gray-700"
          >
            Back to Sessions
          </button>
        </div>
      </header>

      <div className="flex flex-1 min-h-0">
        <Sidebar agents={agents} />
        <div className="flex flex-col flex-1 min-h-0">
          {senderId === null && (
            <div className="bg-red-50 border-b border-red-200 px-4 py-2 text-sm text-red-700">
              This session has no Administrator agent. Send is disabled.
              If you just upgraded, run
              <code className="mx-1 bg-red-100 px-1 rounded">
                cafleet db init
              </code>
              to apply the backfill migration. If the Administrator was manually
              deleted, contact the operator —
              <code className="mx-1 bg-red-100 px-1 rounded">db init</code>
              will not re-seed it.
            </div>
          )}
          {agents.length === 0 ? (
            <div className="flex-1 flex items-center justify-center">
              <p className="text-gray-400 text-sm">
                No agents registered in this session. Use the{" "}
                <code className="text-gray-500">cafleet register</code> CLI to
                add one.
              </p>
            </div>
          ) : (
            <Timeline agents={agents} refreshKey={refreshKey} />
          )}
          <MessageInput
            senderId={senderId}
            agents={agents}
            onSent={refreshAll}
            disabled={senderId === null}
          />
        </div>
      </div>
    </div>
  );
}
