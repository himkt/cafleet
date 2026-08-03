import { useState, useEffect, useCallback } from "react";
import type { Member } from "./types";
import { setFleetId, getMembers, listFleets } from "./api";
import FleetPicker from "./components/FleetPicker";
import Dashboard from "./components/Dashboard";
import Skeleton from "./components/Skeleton";

interface Route {
  kind: "fleets" | "dashboard";
  fleetId?: string;
  memberId?: string;
}

function parseHash(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  const match = hash.match(/^fleets\/([^/]+)\/members(?:\/([^/]+))?/);
  if (match) {
    return { kind: "dashboard", fleetId: match[1], memberId: match[2] };
  }
  return { kind: "fleets" };
}

function navigate(hash: string): void {
  window.location.hash = hash;
}

export default function App() {
  const [route, setRoute] = useState<Route>(parseHash);
  const [members, setMembers] = useState<Member[]>([]);
  const [fleetName, setFleetName] = useState<string | null>(null);
  const [loadedFleetId, setLoadedFleetId] = useState<string | null>(null);

  // Derived, never set in an effect: a dashboard route whose fleet has not
  // loaded yet shows the skeleton, so stale members from a previous fleet can
  // never render against the new fleet id.
  const loading =
    route.kind === "dashboard" && !!route.fleetId
      ? loadedFleetId !== route.fleetId
      : false;

  useEffect(() => {
    const onHashChange = () => setRoute(parseHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  useEffect(() => {
    if (route.kind !== "dashboard" || !route.fleetId) {
      return;
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
        setFleetName(fleet.name);

        setFleetId(Number(route.fleetId));
        const data = await getMembers();
        if (cancelled) return;
        setMembers(data.members);
        setLoadedFleetId(route.fleetId ?? null);
      } catch {
        if (!cancelled) {
          navigate("/fleets");
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [route.kind, route.fleetId]);

  const handleSelectFleet = useCallback(
    async (sid: number, name: string | null) => {
      setFleetId(sid);
      try {
        const data = await getMembers();
        setMembers(data.members);
        setFleetName(name);
        setLoadedFleetId(String(sid));
        navigate(`/fleets/${sid}/members`);
      } catch {
        setFleetId(null);
      }
    },
    [],
  );

  const handleBack = useCallback(() => {
    setFleetId(null);
    setMembers([]);
    setLoadedFleetId(null);
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
        fleetName={fleetName}
        memberId={route.memberId}
        initialMembers={members}
        onBack={handleBack}
      />
    );
  }

  return <FleetPicker onSelect={handleSelectFleet} />;
}
