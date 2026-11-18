import { useState } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { useResource } from '../useResource';
import { monitorPageSize } from './MonitorPicker';

export type ManagedResource = { id: number; target: string; name: string; engine_id: string };
type ManagedPage = { resources: ManagedResource[]; has_more: boolean };

export function useManagedResources<T>(base: string, kind: 'process' | 'docker', online: boolean, onExpired: () => void) {
  const path = base + '/managed-resources/' + kind;
  const [offset, setOffset] = useState(0);
  const [error, setError] = useState('');
  const [removing, setRemoving] = useState<number>();
  const query = '?offset=' + offset;
  const registry = useResource<ManagedPage>(path + query, onExpired, 10_000);
  const rows = registry.data?.resources ?? [];
  const snapshot = useResource<T>(online && rows.length ? path + '/live' + query : null, onExpired, 5_000);
  if (offset > 0 && registry.data && !registry.data.resources.length) setOffset(offset - monitorPageSize);

  async function remove(id: number) {
    setRemoving(id);
    setError('');
    try {
      await request(path + '/' + id, { method: 'DELETE', response: 'empty' });
      refresh();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setRemoving(undefined);
    }
  }

  function refresh() {
    registry.refresh();
    snapshot.refresh();
  }

  return { path, registry, rows, snapshot, offset, setOffset, refresh, error, removing, remove };
}
