import { useEffect, useCallback } from "react";
import { TriangleAlert, Users } from "lucide-react";
import { ApiError } from "../api";
import type { FleetClient } from "../api";
import { parsePositiveId } from "../route";
import { useResource } from "../hooks/useResource";
import { useRefreshKey } from "../hooks/useRefreshKey";
import MemberDetail from "./MemberDetail";
import AppHeader from "./AppHeader";
import EmptyState from "./EmptyState";
import Sidebar from "./Sidebar";
import Timeline from "./Timeline";
import MessageInput from "./MessageInput";
import ResourceNotice from "./ResourceNotice";

export interface DashboardProps {
  client: FleetClient;
  fleetName: string | null;
  memberId?: string;
  onBack: () => void;
}
export default function Dashboard({ client, fleetName, memberId, onBack }: DashboardProps) {
  const { fleetId } = client;
  const { refreshKey, refresh } = useRefreshKey();
  const loadMembers = useCallback((signal: AbortSignal) => client.getMembers({ signal }), [client]);
  const loadMonitor = useCallback((signal: AbortSignal) => client.getMonitor({ signal }), [client]);
  const roster = useResource({ key: `${fleetId}:members`, load: loadMembers, refreshKey });
  const monitor = useResource({ key: `${fleetId}:monitor`, load: loadMonitor, refreshKey });
  const members = roster.state.data?.members ?? [];
  const selectedId = memberId === undefined ? null : parsePositiveId(memberId);
  const detailMember = members.find((member) => member.member_id === selectedId) ?? null;
  const closeDetail = useCallback(() => { window.location.hash = `/fleets/${fleetId}/members`; }, [fleetId]);
  const selectMember = useCallback((id: number) => { window.location.hash = `/fleets/${fleetId}/members/${id}`; }, [fleetId]);
  useEffect(() => {
    if ([roster.state.error, monitor.state.error].some((error) => error instanceof ApiError && error.status === 404 && error.message === "Fleet not found")) window.location.replace("#/fleets");
  }, [roster.state.error, monitor.state.error]);
  useEffect(() => {
    if (roster.state.status === "success" && memberId !== undefined && detailMember === null) {
      window.location.replace(`#/fleets/${fleetId}/members`);
    }
  }, [roster.state.status, memberId, detailMember, fleetId]);
  const director = members.find((member) => member.kind === "director");
  const senderId = director?.status === "active" ? director.member_id : null;
  const ready = roster.state.status === "success";
  const isPolling = roster.state.status === "loading" || roster.state.refreshing || monitor.state.status === "loading" || monitor.state.refreshing;
  return <div className="h-screen flex flex-col bg-surface">
    <AppHeader client={client} isPolling={isPolling} onRefresh={refresh}
      fleetName={fleetName ?? String(fleetId)} onBack={onBack}
      sendingAsDirector={ready && senderId !== null} monitor={monitor.state.data}
      onMonitorSaved={refresh} />
    <ResourceNotice state={monitor.state} name="monitor" retry={monitor.refresh} />
    <ResourceNotice state={roster.state} name="members" retry={roster.refresh} />
    {ready && <div className="flex flex-1 min-h-0">
      <Sidebar members={members} onSelectMember={selectMember} />
      <div className="flex flex-col flex-1 min-h-0">
        {senderId === null && <div className="flex items-start gap-2 border-b border-danger/30 bg-danger-soft px-4 py-2 text-sm text-danger">
          <TriangleAlert size={16} aria-hidden="true" />
          <p>This fleet has no active root Director. Send is disabled. Every send goes out as the fleet&apos;s root Director; create a fresh fleet with <code>cafleet fleet create</code> or contact the operator.</p>
        </div>}
        {members.length === 0 ? <div className="flex flex-1 items-center justify-center">
          <EmptyState icon={Users} title="No members registered in this fleet.">
            <p className="text-xs text-text-muted">Use the <code>cafleet member create</code> CLI to add one.</p>
          </EmptyState>
        </div> : <Timeline client={client} members={members} refreshKey={refreshKey} />}
        <MessageInput client={client} senderId={senderId} members={members} onSent={refresh} />
      </div>
      {detailMember && <MemberDetail key={detailMember.member_id} client={client} member={detailMember} refreshKey={refreshKey} onClose={closeDetail} />}
    </div>}
  </div>;
}
