import { useState, useEffect, useCallback, useMemo } from "react";
import { createFleetClient, listFleets } from "./api";
import { parseHashRoute } from "./route";
import { useResource } from "./hooks/useResource";
import FleetPicker from "./components/FleetPicker";
import Dashboard from "./components/Dashboard";
import ResourceNotice from "./components/ResourceNotice";

function backToFleets() { window.location.hash = "/fleets"; }
function FleetView({ fleetId, memberId }: { fleetId: number; memberId?: string }) {
  const load = useCallback((signal: AbortSignal) => listFleets({ signal }), []);
  const { state, refresh } = useResource({ key: `fleet:${fleetId}`, load });
  const client = useMemo(() => createFleetClient(fleetId), [fleetId]);
  const fleet = state.status === "success" ? state.data.find((item) => item.fleet_id === fleetId) : undefined;
  useEffect(() => {
    if (state.status === "success" && !fleet) window.location.replace("#/fleets");
  }, [state.status, fleet]);
  if (!fleet) return <ResourceNotice state={state} name="fleets" retry={refresh} />;
  return <Dashboard key={fleetId} client={client} fleetName={fleet.name} memberId={memberId} onBack={backToFleets} />;
}

export default function App() {
  const [hash, setHash] = useState(() => window.location.hash);
  const route = parseHashRoute(hash);
  useEffect(() => {
    const changed = () => setHash(window.location.hash);
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  useEffect(() => {
    if (route.kind === "fleets" && hash !== "#/fleets") window.location.replace("#/fleets");
  }, [route.kind, hash]);
  const select = useCallback((id: number) => { window.location.hash = `/fleets/${id}/members`; }, []);
  if (route.kind === "dashboard") return <FleetView key={route.fleetId} fleetId={route.fleetId} memberId={route.memberId} />;
  return <FleetPicker onSelect={select} />;
}
