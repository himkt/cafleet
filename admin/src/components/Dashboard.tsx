import { useState, useCallback } from "react";
import { TriangleAlert, Users } from "lucide-react";
import type { Agent } from "../types";
import { getAgents } from "../api";
import { usePolling, POLL_INTERVAL_MS } from "../hooks/usePolling";
import AppHeader from "./AppHeader";
import EmptyState from "./EmptyState";
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

  const handleSelectAgent = useCallback(
    (agentId: number) => {
      window.location.hash = `/fleets/${fleetId}/agents/${agentId}`;
    },
    [fleetId],
  );

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
        <Sidebar agents={agents} onSelectAgent={handleSelectAgent} />
        <div className="flex flex-col flex-1 min-h-0">
          {senderId === null && (
            <div className="flex items-start gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-sm text-danger">
              <TriangleAlert size={16} className="mt-0.5 shrink-0" aria-hidden="true" />
              <p>
                This fleet has no Administrator agent. Send is disabled.
                If you just upgraded, run
                <code className="mx-1 rounded bg-danger/15 px-1 font-mono">
                  cafleet db init
                </code>
                to apply the backfill migration. If the Administrator was manually
                deleted, contact the operator —
                <code className="mx-1 rounded bg-danger/15 px-1 font-mono">db init</code>
                will not re-seed it.
              </p>
            </div>
          )}
          {agents.length === 0 ? (
            <div className="flex flex-1 items-center justify-center">
              <EmptyState
                icon={Users}
                title="No agents registered in this fleet."
              >
                <p className="text-xs text-text-muted">
                  Use the{" "}
                  <code className="rounded bg-surface-hover px-1.5 py-0.5 font-mono text-text">
                    cafleet agent register
                  </code>{" "}
                  CLI to add one.
                </p>
              </EmptyState>
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
