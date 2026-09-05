import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { LoaderCircle, RefreshCw } from "lucide-react";
import type { MonitorRuntime } from "../types";
import { ApiError } from "../api";
import type { FleetClient } from "../api";
import ThemeToggle from "./ThemeToggle";

function BrandMark() {
  return (
    <svg
      viewBox="0 0 48 46"
      className="size-5 text-accent"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M25.946 44.938c-.664.845-2.021.375-2.021-.698V33.937a2.26 2.26 0 0 0-2.262-2.262H10.287c-.92 0-1.456-1.04-.92-1.788l7.48-10.471c1.07-1.497 0-3.578-1.842-3.578H1.237c-.92 0-1.456-1.04-.92-1.788L10.013.474c.214-.297.556-.474.92-.474h28.894c.92 0 1.456 1.04.92 1.788l-7.48 10.471c-1.07 1.498 0 3.579 1.842 3.579h11.377c.943 0 1.473 1.088.89 1.83L25.947 44.94z" />
    </svg>
  );
}

function parseInterval(draft: string): number | null {
  const trimmed = draft.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return Number.isSafeInteger(value) ? value : null;
}

function MonitorIndicator({
  client,
  monitor,
  onSaved,
}: {
  client: FleetClient;
  monitor: MonitorRuntime | null;
  onSaved: () => void;
}) {
  const [missingFleet, setMissingFleet] = useState(false);
  useEffect(() => {
    if (missingFleet) window.location.replace("#/fleets");
  }, [missingFleet]);
  const lifetime = useRef<AbortController | null>(null);
  useLayoutEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => { controller.abort(); lifetime.current = null; };
  }, [client]);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [waking, setWaking] = useState(false);
  const [wakeRequested, setWakeRequested] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const container = containerRef.current;
      if (container && !container.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  if (monitor === null) return null;
  const running = monitor.running;
  const parsed = parseInterval(draft);

  const toggleOpen = () => {
    if (!open) {
      setDraft(
        monitor.wake_interval_seconds === null
          ? ""
          : String(monitor.wake_interval_seconds),
      );
      setError(null);
      setWakeRequested(false);
    }
    setOpen((wasOpen) => !wasOpen);
  };

  const save = async () => {
    const controller = lifetime.current;
    if (!controller || controller.signal.aborted) return;
    if (!running || parsed === null) return;
    setSaving(true);
    setError(null);
    try {
      await client.patchMonitor(parsed, { signal: controller.signal });
      if (controller.signal.aborted) return;
    } catch (err) {
      if (controller.signal.aborted) return;
      if (err instanceof ApiError && err.status === 404 && err.message === "Fleet not found") {
        setMissingFleet(true);
        return;
      }
      setError(err instanceof Error ? err.message : "Save failed");
      return;
    } finally {
      if (!controller.signal.aborted) setSaving(false);
    }
    setOpen(false);
    onSaved();
  };

  const wake = async () => {
    const controller = lifetime.current;
    if (!controller || controller.signal.aborted) return;
    if (!running || waking) return;
    setWaking(true);
    setError(null);
    setWakeRequested(false);
    try {
      await client.postMonitorWake({ signal: controller.signal });
      if (controller.signal.aborted) return;
    } catch (err) {
      if (controller.signal.aborted) return;
      if (err instanceof ApiError && err.status === 404 && err.message === "Fleet not found") {
        setMissingFleet(true);
        return;
      }
      setError(err instanceof Error ? err.message : "Wake failed");
      return;
    } finally {
      if (!controller.signal.aborted) setWaking(false);
    }
    setWakeRequested(true);
    onSaved();
  };

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={toggleOpen}
        aria-haspopup="dialog"
        aria-expanded={open}
        className="flex items-center gap-1.5 rounded text-xs text-text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        title={
          running
            ? "cafleet monitor is running for this fleet"
            : "No cafleet monitor running — start it with 'cafleet monitor <fleet-id>'"
        }
      >
        <span
          aria-hidden="true"
          className={`size-2 rounded-full ${
            running ? "bg-success" : "bg-text-faint"
          }`}
        />
        Monitor {running ? "running" : "stopped"}
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Director wake interval"
          className="absolute right-0 top-full z-10 mt-1 w-72 rounded-lg border border-border bg-surface-raised p-3 shadow-lg"
        >
          <label className="block text-xs font-medium">
            Director wake interval (seconds)
            <input
              type="number"
              min={0}
              step={1}
              value={draft}
              onChange={(e) => {
                setDraft(e.target.value);
                setError(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") void save();
              }}
              disabled={!running || saving}
              className="mt-1.5 w-full rounded-lg border border-border bg-surface px-2 py-1.5 font-mono text-sm outline-none focus:border-accent focus:ring-2 focus:ring-accent/30 disabled:opacity-50"
            />
          </label>
          {running && parsed === 0 && (
            <p className="mt-1.5 text-[11px] text-text-muted">
              0 disables the Director wake while the loop keeps running.
            </p>
          )}
          {!running && (
            <p className="mt-1.5 text-[11px] text-text-muted">
              Monitor not running — the interval is re-stamped from the
              CLI/env when the monitor starts, so there is nothing durable
              to edit.
            </p>
          )}
          {error && <p className="mt-1.5 text-xs text-danger">{error}</p>}
          <button
            type="button"
            onClick={() => void save()}
            disabled={!running || saving || parsed === null}
            className="mt-2 flex w-full items-center justify-center gap-1.5 rounded-lg bg-accent px-2 py-1.5 text-xs font-medium text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            {saving && (
              <LoaderCircle
                size={12}
                className="motion-safe:animate-spin"
                aria-hidden="true"
              />
            )}
            Save
          </button>
          <button
            type="button"
            onClick={() => void wake()}
            disabled={!running || waking}
            className="mt-1.5 flex w-full items-center justify-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5 text-xs font-medium hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            {waking && (
              <LoaderCircle
                size={12}
                className="motion-safe:animate-spin"
                aria-hidden="true"
              />
            )}
            Wake now
          </button>
          {wakeRequested && (
            <p className="mt-1.5 text-[11px] text-text-muted">
              Wake requested — fires within one tick.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function LiveIndicator({ isPolling }: { isPolling: boolean }) {
  return (
    <span
      className="flex items-center gap-1.5 text-xs text-text-muted"
      title="Auto-refreshes every 5 seconds"
    >
      {isPolling ? (
        <RefreshCw
          size={12}
          className="text-success motion-safe:animate-spin"
          aria-hidden="true"
        />
      ) : (
        <span
          className="size-2 rounded-full bg-success motion-safe:animate-pulse-dot"
          aria-hidden="true"
        />
      )}
      Live
    </span>
  );
}

export interface AppHeaderProps {
  client?: FleetClient;
  isPolling: boolean;
  onRefresh: () => void;
  /** Breadcrumb tail (fleet name or id). Presence switches `Fleets` into a link. */
  fleetName?: string;
  onBack?: () => void;
  sendingAsDirector?: boolean;
  /** The polled monitor payload; `null` until the first fetch resolves. */
  monitor?: MonitorRuntime | null;
  /** The dashboard refresh trigger, fired after a successful interval save. */
  onMonitorSaved?: () => void;
}

export default function AppHeader({
  client,
  isPolling,
  onRefresh,
  fleetName,
  onBack,
  sendingAsDirector = false,
  monitor = null,
  onMonitorSaved = () => {},
}: AppHeaderProps) {
  return (
    <header className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-surface-raised px-4 py-2">
      <div className="flex min-w-0 items-center gap-2.5">
        <BrandMark />
        <span className="text-base font-semibold tracking-tight">CAFleet</span>
        <nav
          aria-label="Breadcrumb"
          className="flex min-w-0 items-center gap-1.5 text-sm"
        >
          {fleetName !== undefined && onBack ? (
            <>
              <button
                type="button"
                onClick={onBack}
                className="rounded text-text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
              >
                Fleets
              </button>
              <span className="text-text-faint" aria-hidden="true">
                /
              </span>
              <span className="truncate font-medium">{fleetName}</span>
            </>
          ) : (
            <span className="font-medium text-text-muted">Fleets</span>
          )}
        </nav>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {sendingAsDirector && (
          <span className="hidden text-sm text-text-muted sm:inline">
            Sending as <span className="font-medium text-text">Director</span>
          </span>
        )}
        {client && <MonitorIndicator key={client.fleetId} client={client} monitor={monitor} onSaved={onMonitorSaved} />}
        <LiveIndicator isPolling={isPolling} />
        <button
          type="button"
          onClick={onRefresh}
          aria-label="Refresh"
          title="Refresh"
          className="rounded-lg p-2 text-text-muted hover:bg-surface-hover hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <RefreshCw size={18} aria-hidden="true" />
        </button>
        <ThemeToggle />
      </div>
    </header>
  );
}
