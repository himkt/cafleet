import { useState, useEffect, useCallback } from "react";
import { TriangleAlert, Users } from "lucide-react";
import type { Agent, MonitorRuntime } from "../types";
import { getAgents, getMonitor } from "../api";
import { usePolling, POLL_INTERVAL_MS } from "../hooks/usePolling";
import AgentDetail from "./AgentDetail";
import AppHeader from "./AppHeader";
import EmptyState from "./EmptyState";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";

interface DashboardProps {
  fleetId: number;
  fleetLabel: string | null;
  agentId?: string;
  initialAgents: Agent[];
  onBack: () => void;
}

export default function Dashboard({
  fleetId,
  fleetLabel,
  agentId,
  initialAgents,
  onBack,
}: DashboardProps) {
  const [agents, setAgents] = useState<Agent[]>(initialAgents);
  const [monitor, setMonitor] = useState<MonitorRuntime | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [isPolling, setIsPolling] = useState(false);

  const refreshAll = useCallback(async () => {
    setIsPolling(true);
    // Decoupled: a transient /api/monitor failure must not block the agents
    // refresh (and vice versa). allSettled never rejects; each result is
    // applied only when fulfilled, preserving the last-known value otherwise.
    const [agentsResult, monitorResult] = await Promise.allSettled([
      getAgents(),
      getMonitor(),
    ]);
    if (agentsResult.status === "fulfilled") {
      setAgents(agentsResult.value.agents);
    }
    if (monitorResult.status === "fulfilled") {
      setMonitor(monitorResult.value);
    }
    setIsPolling(false);
    setRefreshKey((k) => k + 1);
  }, []);

  const trigger = usePolling(refreshAll, POLL_INTERVAL_MS);

  // Seed the monitor indicator immediately (agents arrive via initialAgents);
  // the periodic poll keeps both fresh thereafter.
  useEffect(() => {
    void trigger();
  }, [trigger]);

  const handleSelectAgent = useCallback(
    (selectedId: number) => {
      window.location.hash = `/fleets/${fleetId}/agents/${selectedId}`;
    },
    [fleetId],
  );

  const closeDetail = useCallback(() => {
    window.location.hash = `/fleets/${fleetId}/agents`;
  }, [fleetId]);

  const detailAgent =
    agentId !== undefined
      ? (agents.find((a) => a.agent_id === Number(agentId)) ?? null)
      : null;

  // Unknown / cross-fleet agentId: the membership check against the loaded
  // agents list covers both — redirect to the dashboard route before any
  // detail fetch happens (AgentDetail only mounts when detailAgent resolves).
  useEffect(() => {
    if (agentId !== undefined && detailAgent === null) {
      closeDetail();
    }
  }, [agentId, detailAgent, closeDetail]);

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
        monitorRunning={monitor === null ? null : monitor.running}
      />

      <div className="flex flex-1 min-h-0">
        <Sidebar agents={agents} onSelectAgent={handleSelectAgent} />
        <div className="flex flex-col flex-1 min-h-0">
          {senderId === null && (
            <div className="flex items-start gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-sm text-danger">
              <TriangleAlert size={16} className="mt-0.5 shrink-0" aria-hidden="true" />
              <p>
                This fleet has no Administrator agent. Send is disabled.
                Fleets created under an older schema lack the built-in
                Administrator; create a fresh fleet with
                <code className="mx-1 rounded bg-danger/15 px-1 font-mono">
                  cafleet fleet create
                </code>
                — upgrades preserve existing data but never backfill rows. If
                the Administrator was manually deleted, contact the operator.
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
                    cafleet member create
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
        {detailAgent !== null && (
          <AgentDetail
            agent={detailAgent}
            refreshKey={refreshKey}
            onClose={closeDetail}
            onChanged={() => {
              void trigger();
            }}
          />
        )}
      </div>
    </div>
  );
}
