import { useState, useEffect, useCallback } from "react";
import { TriangleAlert, Users } from "lucide-react";
import type { Member, MonitorRuntime } from "../types";
import { getMembers, getMonitor } from "../api";
import { usePolling, POLL_INTERVAL_MS } from "../hooks/usePolling";
import MemberDetail from "./MemberDetail";
import AppHeader from "./AppHeader";
import EmptyState from "./EmptyState";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";

interface DashboardProps {
  fleetId: number;
  fleetName: string | null;
  memberId?: string;
  initialMembers: Member[];
  onBack: () => void;
}

export default function Dashboard({
  fleetId,
  fleetName,
  memberId,
  initialMembers,
  onBack,
}: DashboardProps) {
  const [members, setMembers] = useState<Member[]>(initialMembers);
  const [monitor, setMonitor] = useState<MonitorRuntime | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [isPolling, setIsPolling] = useState(false);

  const refreshAll = useCallback(async () => {
    setIsPolling(true);
    // Decoupled: a transient /api/monitor failure must not block the members
    // refresh (and vice versa). allSettled never rejects; each result is
    // applied only when fulfilled, preserving the last-known value otherwise.
    const [membersResult, monitorResult] = await Promise.allSettled([
      getMembers(),
      getMonitor(),
    ]);
    if (membersResult.status === "fulfilled") {
      setMembers(membersResult.value.members);
    }
    if (monitorResult.status === "fulfilled") {
      setMonitor(monitorResult.value);
    }
    setIsPolling(false);
    setRefreshKey((k) => k + 1);
  }, []);

  const trigger = usePolling(refreshAll, POLL_INTERVAL_MS);

  // Seed the monitor indicator immediately (members arrive via initialMembers);
  // the periodic poll keeps both fresh thereafter.
  useEffect(() => {
    void trigger();
  }, [trigger]);

  const handleSelectMember = useCallback(
    (selectedId: number) => {
      window.location.hash = `/fleets/${fleetId}/members/${selectedId}`;
    },
    [fleetId],
  );

  const closeDetail = useCallback(() => {
    window.location.hash = `/fleets/${fleetId}/members`;
  }, [fleetId]);

  const detailMember =
    memberId !== undefined
      ? (members.find((m) => m.member_id === Number(memberId)) ?? null)
      : null;

  // Unknown / cross-fleet memberId: the membership check against the loaded
  // members list covers both — redirect to the dashboard route before any
  // detail fetch happens (MemberDetail only mounts when detailMember resolves).
  useEffect(() => {
    if (memberId !== undefined && detailMember === null) {
      closeDetail();
    }
  }, [memberId, detailMember, closeDetail]);

  const director = members.find((m) => m.kind === "director") ?? null;
  const senderId = director?.status === "active" ? director.member_id : null;

  return (
    <div className="h-screen flex flex-col bg-surface">
      <AppHeader
        isPolling={isPolling}
        onRefresh={() => {
          void trigger();
        }}
        fleetName={fleetName ?? String(fleetId)}
        onBack={onBack}
        sendingAsDirector={senderId !== null}
        monitorRunning={monitor === null ? null : monitor.running}
      />

      <div className="flex flex-1 min-h-0">
        <Sidebar members={members} onSelectMember={handleSelectMember} />
        <div className="flex flex-col flex-1 min-h-0">
          {senderId === null && (
            <div className="flex items-start gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-sm text-danger">
              <TriangleAlert size={16} className="mt-0.5 shrink-0" aria-hidden="true" />
              <p>
                This fleet has no active root Director. Send is disabled.
                Every send goes out as the fleet&apos;s root Director, so a
                fleet whose Director row is missing or deregistered cannot
                message; create a fresh fleet with
                <code className="mx-1 rounded bg-danger/15 px-1 font-mono">
                  cafleet fleet create
                </code>
                or contact the operator.
              </p>
            </div>
          )}
          {members.length === 0 ? (
            <div className="flex flex-1 items-center justify-center">
              <EmptyState
                icon={Users}
                title="No members registered in this fleet."
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
            <Timeline members={members} refreshKey={refreshKey} />
          )}
          <MessageInput
            senderId={senderId}
            members={members}
            onSent={() => {
              void trigger();
            }}
          />
        </div>
        {detailMember !== null && (
          <MemberDetail
            member={detailMember}
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
