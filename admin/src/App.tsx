import { useState, useEffect, useCallback } from "react";
import type { Agent } from "./types";
import { setFleetId, getAgents, listFleets } from "./api";
import FleetPicker from "./components/FleetPicker";
import Dashboard from "./components/Dashboard";

interface Route {
  kind: "fleets" | "dashboard";
  fleetId?: string;
}

function parseHash(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  const match = hash.match(/^fleets\/([^/]+)\/agents/);
  if (match) {
    return { kind: "dashboard", fleetId: match[1] };
  }
  return { kind: "fleets" };
}

function navigate(hash: string): void {
  window.location.hash = hash;
}

export default function App() {
  const [route, setRoute] = useState<Route>(parseHash);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const onHashChange = () => setRoute(parseHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    if (route.kind !== "dashboard" || !route.fleetId) {
      setLoading(false);
      return;
    }

    let cancelled = false;

    (async () => {
      try {
        const fleets = await listFleets();
        if (cancelled) return;

        const found = fleets.some((s) => s.fleet_id === Number(route.fleetId));
        if (!found) {
          navigate("/fleets");
          return;
        }

        setFleetId(Number(route.fleetId));
        const data = await getAgents();
        if (cancelled) return;
        setAgents(data.agents);
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
  }, [route]);

  const handleSelectFleet = useCallback(async (sid: number) => {
    setFleetId(sid);
    try {
      const data = await getAgents();
      setAgents(data.agents);
      navigate(`/fleets/${sid}/agents`);
    } catch {
      setFleetId(null);
    }
  }, []);

  const handleBack = useCallback(() => {
    setFleetId(null);
    setAgents([]);
    navigate("/fleets");
  }, []);

  if (loading && route.kind === "dashboard") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <p className="text-gray-400">Loading...</p>
      </div>
    );
  }

  if (route.kind === "dashboard" && route.fleetId) {
    return (
      <Dashboard
        fleetId={Number(route.fleetId)}
        initialAgents={agents}
        onBack={handleBack}
      />
    );
  }

  return <FleetPicker onSelect={handleSelectFleet} />;
}
