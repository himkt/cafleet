import { useState, useEffect, useCallback, useRef } from "react";
import type { Agent } from "./types";
import { setFleetId, getAgents, listFleets } from "./api";
import FleetPicker from "./components/FleetPicker";
import Dashboard from "./components/Dashboard";
import Skeleton from "./components/Skeleton";

interface Route {
  kind: "fleets" | "dashboard";
  fleetId?: string;
  agentId?: string;
}

function parseHash(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  const match = hash.match(/^fleets\/([^/]+)\/agents(?:\/([^/]+))?/);
  if (match) {
    return { kind: "dashboard", fleetId: match[1], agentId: match[2] };
  }
  return { kind: "fleets" };
}

function navigate(hash: string): void {
  window.location.hash = hash;
}

export default function App() {
  const [route, setRoute] = useState<Route>(parseHash);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [fleetLabel, setFleetLabel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const loadedFleetIdRef = useRef<string | null>(null);

  useEffect(() => {
    const onHashChange = () => setRoute(parseHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    if (route.kind !== "dashboard" || !route.fleetId) {
      loadedFleetIdRef.current = null;
      setLoading(false);
      return;
    }

    // Entering a different fleet's dashboard: drop the previous fleet's
    // state before any network await so it can never render against the new
    // fleet id. Same-fleet route changes (e.g. opening the agent detail
    // panel) keep their data and skip the skeleton.
    if (loadedFleetIdRef.current !== route.fleetId) {
      setAgents([]);
      setFleetLabel(null);
      setLoading(true);
    }

    let cancelled = false;

    (async () => {
      try {
        const fleets = await listFleets();
        if (cancelled) return;

        const fleet = fleets.find((s) => s.fleet_id === Number(route.fleetId));
        if (!fleet) {
          navigate("/fleets");
          return;
        }
        setFleetLabel(fleet.label);

        setFleetId(Number(route.fleetId));
        const data = await getAgents();
        if (cancelled) return;
        setAgents(data.agents);
        loadedFleetIdRef.current = route.fleetId ?? null;
      } catch {
        if (!cancelled) {
          navigate("/fleets");
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [route.kind, route.fleetId]);

  const handleSelectFleet = useCallback(
    async (sid: number, label: string | null) => {
      setFleetId(sid);
      try {
        const data = await getAgents();
        setAgents(data.agents);
        setFleetLabel(label);
        loadedFleetIdRef.current = String(sid);
        navigate(`/fleets/${sid}/agents`);
      } catch {
        setFleetId(null);
      }
    },
    [],
  );

  const handleBack = useCallback(() => {
    setFleetId(null);
    setAgents([]);
    navigate("/fleets");
  }, []);

  if (loading && route.kind === "dashboard") {
    return (
      <div className="flex h-screen flex-col bg-surface">
        <div className="border-b border-border bg-surface-raised px-4 py-3">
          <Skeleton className="h-5 w-40" />
        </div>
        <div className="flex flex-1">
          <div className="w-60 shrink-0 border-r border-border p-3">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="mt-3 h-4 w-28" />
          </div>
          <div className="flex-1 p-4">
            <Skeleton className="h-4 w-64" />
            <Skeleton className="mt-3 h-4 w-48" />
          </div>
        </div>
      </div>
    );
  }

  if (route.kind === "dashboard" && route.fleetId) {
    return (
      <Dashboard
        fleetId={Number(route.fleetId)}
        fleetLabel={fleetLabel}
        agentId={route.agentId}
        initialAgents={agents}
        onBack={handleBack}
      />
    );
  }

  return <FleetPicker onSelect={handleSelectFleet} />;
}
