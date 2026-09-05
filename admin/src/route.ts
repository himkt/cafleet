export type Route = { kind: "fleets" } | {
  kind: "dashboard"; fleetId: number; memberId?: string;
};

export function parsePositiveId(token: string): number | null {
  if (!token || /[^0-9]/.test(token)) return null;
  const value = Number(token);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

export function parseHashRoute(hash: string): Route {
  const path = hash.replace(/^#\/?/, "");
  const match = path.match(/^fleets\/([^/]+)\/members(?:\/([^/]+))?$/);
  if (!match || match[0] !== path) return { kind: "fleets" };
  const fleetId = parsePositiveId(match[1]);
  if (fleetId === null) return { kind: "fleets" };
  return match[2] === undefined ? { kind: "dashboard", fleetId }
    : { kind: "dashboard", fleetId, memberId: match[2] };
}
