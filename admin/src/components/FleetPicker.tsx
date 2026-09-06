import { useCallback } from "react";
import { Boxes, ChevronRight } from "lucide-react";
import type { FleetListItem } from "../types";
import { listFleets } from "../api";
import { useRefreshKey } from "../hooks/useRefreshKey";
import { useResource } from "../hooks/useResource";
import ResourceNotice from "./ResourceNotice";
import AppHeader from "./AppHeader";
import EmptyState from "./EmptyState";
import Skeleton from "./Skeleton";

interface FleetPickerProps {
  onSelect: (fleetId: number, name: string | null) => void;
}

function FleetCard({
  fleet,
  onSelect,
}: {
  fleet: FleetListItem;
  onSelect: (fleetId: number, name: string | null) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(fleet.fleet_id, fleet.name)}
      className="group flex w-full items-center justify-between gap-3 rounded-xl border border-border bg-surface-raised px-4 py-3.5 text-left shadow-sm hover:border-accent/40 hover:shadow-md hover:ring-2 hover:ring-accent/20 focus-visible:outline-2 focus-visible:outline-accent motion-safe:transition"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-semibold">
            {fleet.name ?? `Fleet ${fleet.fleet_id}`}
          </span>
          <code className="shrink-0 rounded bg-surface-hover px-1.5 py-0.5 font-mono text-xs text-text-muted">
            #{fleet.fleet_id}
          </code>
        </div>
        <p className="mt-1 text-xs text-text-muted">
          {fleet.member_count} member{fleet.member_count !== 1 ? "s" : ""} ·
          created {new Date(fleet.created_at).toLocaleDateString()}
        </p>
      </div>
      <ChevronRight
        size={16}
        className="shrink-0 text-text-faint group-hover:translate-x-0.5 group-hover:text-accent motion-safe:transition-transform"
        aria-hidden="true"
      />
    </button>
  );
}

function FleetCardSkeleton() {
  return (
    <div className="rounded-xl border border-border bg-surface-raised px-4 py-3.5 shadow-sm">
      <Skeleton className="h-4 w-40" />
      <Skeleton className="mt-2 h-3 w-56" />
    </div>
  );
}

export default function FleetPicker({ onSelect }: FleetPickerProps) {
  const { refreshKey, refresh } = useRefreshKey();
  const load = useCallback((signal: AbortSignal) => listFleets({ signal }), []);
  const resource = useResource({ key: "fleets", load, refreshKey });
  const { state } = resource;
  const fleets = state.data ?? [];
  const loading = state.status === "loading";
  const isPolling = loading || state.refreshing;

  return (
    <div className="flex min-h-screen flex-col bg-surface">
      <AppHeader isPolling={isPolling} onRefresh={refresh} />

      <main className="mx-auto mt-8 w-full max-w-2xl flex-1 px-4 pb-8">
        <h1 className="text-xl font-semibold tracking-tight">Select a Fleet</h1>

        {!loading && <ResourceNotice state={state} name="fleets" retry={resource.refresh} />}

        {loading ? (
          <div className="mt-4 flex flex-col gap-3">
            <FleetCardSkeleton />
            <FleetCardSkeleton />
            <FleetCardSkeleton />
          </div>
        ) : state.status === "error" ? null : fleets.length === 0 ? (
          <EmptyState icon={Boxes} title="No fleets found.">
            <p className="text-xs text-text-muted">
              Run{" "}
              <code className="rounded bg-surface-hover px-1.5 py-0.5 font-mono text-text">
                cafleet fleet create
              </code>{" "}
              to create one.
            </p>
          </EmptyState>
        ) : (
          <ul className="mt-4 flex flex-col gap-3">
            {fleets.map((fleet) => (
              <li key={fleet.fleet_id}>
                <FleetCard fleet={fleet} onSelect={onSelect} />
              </li>
            ))}
          </ul>
        )}
      </main>
    </div>
  );
}
