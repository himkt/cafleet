import { useState, useCallback } from "react";
import type { Agent } from "../types";
import { getAgents } from "../api";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";
import SenderSelector from "./SenderSelector";

interface DashboardProps {
  sessionId: string;
  initialAgents: Agent[];
  onBack: () => void;
}

function getStoredSender(sessionId: string, agents: Agent[]): string | null {
  const stored = localStorage.getItem(`hikyaku.sender.${sessionId}`);
  if (stored && agents.some((a) => a.agent_id === stored && a.status === "active")) {
    return stored;
  }
  return null;
}

export default function Dashboard({
  sessionId,
  initialAgents,
  onBack,
}: DashboardProps) {
  const [agents, setAgents] = useState<Agent[]>(initialAgents);
  const [senderId, setSenderId] = useState<string | null>(() =>
    getStoredSender(sessionId, initialAgents),
  );
  const [refreshKey, setRefreshKey] = useState(0);

  const refreshAll = useCallback(async () => {
    try {
      const data = await getAgents();
      setAgents(data.agents);
    } catch {
      // keep current agents on error
    }
    setRefreshKey((k) => k + 1);
  }, []);

  const noAgents = agents.length === 0;

  return (
    <div className="h-screen flex flex-col bg-gray-50">
      <header className="bg-white border-b border-gray-200 px-4 py-2 flex items-center justify-between shrink-0">
        <h1 className="text-lg font-semibold text-gray-900">
          Hikyaku —{" "}
          <span className="font-mono text-sm text-gray-500">
            {sessionId.slice(0, 8)}
          </span>
        </h1>
        <div className="flex items-center gap-3">
          <SenderSelector
            agents={agents}
            sessionId={sessionId}
            onSelect={setSenderId}
          />
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
          {noAgents ? (
            <div className="flex-1 flex items-center justify-center">
              <p className="text-gray-400 text-sm">
                No agents registered in this session. Use the{" "}
                <code className="text-gray-500">hikyaku register</code> CLI to
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
          />
        </div>
      </div>
    </div>
  );
}
