import { onScopeDispose, ref, shallowRef } from 'vue';
import { errorMessage, type CoreError } from './model';

// Each panel owns one bounded read. Disposal prevents a late result crossing
// an installation/instance boundary; a failed refresh retains that panel's data.
export function useDetailRead<T>(read: () => Promise<T>) {
  const data = shallowRef<T | null>(null), error = ref<CoreError | null>(null), loading = ref(false);
  let disposed = false;
  async function refresh() {
    if (disposed || loading.value) return;
    loading.value = true;
    try { const result = await read(); if (!disposed) { data.value = result; error.value = null; } }
    catch (cause) { if (!disposed) error.value = errorMessage(cause); }
    finally { if (!disposed) loading.value = false; }
  }
  onScopeDispose(() => { disposed = true; });
  return { data, error, loading, refresh };
}
