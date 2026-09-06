import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { createResource } from "../resource";
import type { Resource } from "../resource";
export type { ResourceState } from "../resource";

export interface ResourceOptions<T> {
  key: string;
  load: (signal: AbortSignal) => Promise<T>;
  refreshKey?: number;
}
export function useResource<T>({ key, load, refreshKey = 0 }: ResourceOptions<T>) {
  const { resource } = useMemo(() => ({ key, resource: createResource(load) }), [key, load]);
  const state = useSyncExternalStore(resource.subscribe, resource.getSnapshot, resource.getSnapshot);
  const current = useRef<Resource<T> | null>(null);
  const previous = useRef<{ resource: Resource<T>; key: number } | null>(null);
  useEffect(() => {
    current.current = resource;
    resource.start();
    return () => {
      current.current = null;
      resource.stop();
    };
  }, [resource]);
  useEffect(() => {
    if (previous.current?.resource === resource && previous.current.key !== refreshKey) resource.refresh();
    previous.current = { resource, key: refreshKey };
  }, [resource, refreshKey]);
  const refresh = useCallback(() => current.current?.refresh(), []);
  return { state, refresh };
}
