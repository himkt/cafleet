import type { ResourceState } from "../resource";
import Skeleton from "./Skeleton";

export default function ResourceNotice<T>({ state, name, retry }: {
  state: ResourceState<T>; name: string; retry: () => void;
}) {
  if (state.status === "loading") return <div aria-label={`Loading ${name}`} className="p-4"><Skeleton className="h-4 w-40" /></div>;
  if (!state.error) return null;
  return <div role="alert" className="border-b border-danger/30 bg-danger-soft p-3 text-sm text-danger">
    <p>{state.status === "success" ? "Update failed: " : ""}{state.error.message}</p>
    <button type="button" aria-label={`Retry ${name}`} onClick={retry}
      className="mt-2 rounded border border-current px-2 py-1 focus-visible:outline-2">Retry</button>
  </div>;
}
