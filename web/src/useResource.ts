import { useEffect, useState } from 'react';
import { ApiError, errorMessage, isUnauthorized, request } from './api';

type ResourceState<T> = {
  path: string | null;
  data?: T;
  error?: string;
};

export function useResource<T>(path: string | null, onExpired: () => void, interval = 0) {
  const [state, setState] = useState<ResourceState<T>>({ path });
  const [revision, setRevision] = useState(0);
  if (state.path !== path) setState({ path });
  const current = state.path === path ? state : undefined;

  useEffect(() => {
    if (!path) return;
    const activePath = path;
    const cancel = new AbortController();
    let timer: number | undefined;

    async function load() {
      try {
        const data = await request<T>(activePath, { signal: cancel.signal });
        if (!cancel.signal.aborted) {
          setState((previous) => previous.path === activePath ? { path: activePath, data } : previous);
        }
      } catch (failure) {
        if (cancel.signal.aborted) return;
        if (isUnauthorized(failure)) {
          cancel.abort();
          onExpired();
          return;
        }
        const removed = failure instanceof ApiError && [403, 404, 410].includes(failure.status);
        setState((previous) => previous.path === activePath ? {
          path: activePath,
          data: removed ? undefined : previous.data,
          error: errorMessage(failure),
        } : previous);
      } finally {
        if (interval && !cancel.signal.aborted) timer = window.setTimeout(load, interval);
      }
    }

    void load();
    return () => {
      cancel.abort();
      window.clearTimeout(timer);
    };
  }, [path, revision, interval, onExpired]);

  return {
    data: current?.data,
    error: current?.error,
    loading: Boolean(path && !current?.error && current?.data === undefined),
    refresh: () => setRevision((value) => value + 1),
  };
}
