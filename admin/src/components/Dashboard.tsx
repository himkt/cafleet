import { useState, useCallback } from "react";
import type { Agent } from "../types";
import { getAgents } from "../api";
import { usePolling, POLL_INTERVAL_MS } from "../hooks/usePolling";
import AppHeader from "./AppHeader";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";

interface DashboardProps {
  fleetId: number;
  fleetLabel: string | null;
  initialAgents: Agent[];
  onBack: () => void;
}

export default function Dashboard({
  fleetId,
  fleetLabel,
  initialAgents,
  onBack,
}: DashboardProps) {
  const [agents, setAgents] = useState<Agent[]>(initialAgents);
  const [refreshKey, setRefreshKey] = useState(0);
  const [isPolling, setIsPolling] = useState(false);

  const refreshAll = useCallback(async () => {
    setIsPolling(true);
    try {
      const data = await getAgents();
      setAgents(data.agents);
    } catch {
      /* preserve last-known agent list */
    } finally {
      setIsPolling(false);
    }
    setRefreshKey((k) => k + 1);
  }, []);

  const trigger = usePolling(refreshAll, POLL_INTERVAL_MS);

  const administrator =
    agents.find((a) => a.kind === "builtin-administrator") ?? null;
  const senderId =
    administrator?.status === "active" ? administrator.agent_id : null;

  return (
    <div className="h-screen flex flex-col bg-surface">
      <AppHeader
        isPolling={isPolling}
        onRefresh={() => {
          void trigger();
        }}
        fleetLabel={fleetLabel ?? String(fleetId)}
        onBack={onBack}
        sendingAsAdministrator={senderId !== null}
      />

      <div className="flex flex-1 min-h-0">
        <Sidebar agents={agents} />
        <div className="flex flex-col flex-1 min-h-0">
          {senderId === null && (
            <div className="bg-red-50 border-b border-red-200 px-4 py-2 text-sm text-red-700">
              This fleet has no Administrator agent. Send is disabled.
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
                No agents registered in this fleet. Use the{" "}
                <code className="text-gray-500">cafleet agent register</code> CLI to
                add one.
              </p>
            </div>
          ) : (
            <Timeline agents={agents} refreshKey={refreshKey} />
          )}
          <MessageInput
            senderId={senderId}
            agents={agents}
            onSent={() => {
              void trigger();
            }}
          />
        </div>
      </div>
    </div>
  );
}
