import { computed, ref } from 'vue';
import { errorMessage, type CoreError, type Snapshot } from './model';

export function useSnapshot(read: () => Promise<Snapshot>) {
  const snapshot = ref<Snapshot | null>(null);
  const error = ref<CoreError | null>(null);
  const loading = ref(false);
  // A failed refresh keeps the last observation, but withdraws live authority.
  const displayed = computed(() => snapshot.value && ({
    ...snapshot.value, live: snapshot.value.live && !error.value,
    runtime_error: error.value ?? snapshot.value.runtime_error,
  }));
  async function refresh() {
    if (loading.value) return;
    loading.value = true;
    try { snapshot.value = await read(); error.value = null; }
    catch (cause) { error.value = errorMessage(cause); }
    finally { loading.value = false; }
  }
  return { displayed, error, loading, refresh };
}
