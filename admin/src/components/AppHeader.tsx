import { RefreshCw } from "lucide-react";
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

function MonitorIndicator({ running }: { running: boolean | null }) {
  if (running === null) return null;
  return (
    <span
      className="flex items-center gap-1.5 text-xs text-text-muted"
      title={
        running
          ? "cafleet monitor is running for this fleet"
          : "No cafleet monitor running — start it with 'cafleet --fleet-id <id> monitor start'"
      }
    >
      <span
        aria-hidden="true"
        className={`size-2 rounded-full ${
          running ? "bg-success" : "bg-text-faint"
        }`}
      />
      Monitor {running ? "running" : "stopped"}
    </span>
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

interface AppHeaderProps {
  isPolling: boolean;
  onRefresh: () => void;
  /** Breadcrumb tail (fleet name or id). Presence switches `Fleets` into a link. */
  fleetName?: string;
  onBack?: () => void;
  sendingAsAdministrator?: boolean;
  /** Monitor liveness for the fleet; `null` until the first fetch resolves. */
  monitorRunning?: boolean | null;
}

export default function AppHeader({
  isPolling,
  onRefresh,
  fleetName,
  onBack,
  sendingAsAdministrator = false,
  monitorRunning = null,
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
        {sendingAsAdministrator && (
          <span className="hidden text-sm text-text-muted sm:inline">
            Sending as <span className="font-medium text-text">Administrator</span>
          </span>
        )}
        <MonitorIndicator running={monitorRunning} />
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
