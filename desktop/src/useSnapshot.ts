import { computed, ref } from 'vue';
import { errorMessage, type CoreError, type Snapshot } from './model';
export function useSnapshot(read: () => Promise<Snapshot>) {
  const snapshot = ref<Snapshot | null>(null), error = ref<CoreError | null>(null), loading = ref(false);
  const disconnectError=ref<CoreError|null>(null);
  let dirty = false, disconnected = false, flight: Promise<void> | null = null;
  const displayed = computed(() => snapshot.value && ({
    ...snapshot.value, live: snapshot.value.live && !error.value && !disconnected,
    runtime_error: error.value ?? disconnectError.value ?? snapshot.value.runtime_error,
  }));
  function refresh(): Promise<void> {
    if (flight) return flight;
    loading.value = true;
    flight = (async () => {
      do {
        dirty = false;
        try { snapshot.value = await read(); error.value = null; if(!snapshot.value.live && !snapshot.value.runtime_error) disconnectError.value=null; }
        catch (cause) { error.value = errorMessage(cause); }
      } while (dirty);
    })().finally(() => { loading.value = false; flight = null; });
    return flight;
  }
  function invalidate() { dirty = true; return refresh(); }
  function connected() { disconnected = false; disconnectError.value=null; void invalidate(); }
  function offline(cause: unknown) { disconnected = true; error.value = errorMessage(cause); disconnectError.value=error.value; }
  return { displayed, error, loading, refresh, invalidate, connected, offline };
}
