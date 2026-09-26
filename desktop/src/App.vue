<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke, isTauri } from '@tauri-apps/api/core';
import AppTile from './components/AppTile.vue';
import AppDetails from './components/AppDetails.vue';
import { type Snapshot } from './model';
import { useSnapshot } from './useSnapshot';
const { displayed, loading, error, refresh } = useSnapshot(async () => {
  if (!isTauri()) throw { code: 'DESKTOP_REQUIRED', message: '请在 AppHost 桌面窗口中查看应用。' };
  return invoke<Snapshot>('desktop_snapshot');
});
const search = ref('');
const searchInput = ref<HTMLInputElement>();
const selectedId = ref<string | null>(null);
const selected = computed(() => displayed.value?.apps.find(app => app.installation_id === selectedId.value));
const filtered = computed(() => displayed.value?.apps.filter(app => app.label.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase())) ?? []);
let returnFocus: HTMLElement | null = null;
async function details(id: string) {
  returnFocus = document.activeElement as HTMLElement;
  selectedId.value = id;
  await nextTick();
  document.querySelector<HTMLButtonElement>('.drawer button')?.focus();
}
async function closeDetails() {
  selectedId.value = null;
  await nextTick();
  if (returnFocus?.isConnected) returnFocus.focus();
}
function keys(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault(); searchInput.value?.focus();
  }
  if (event.key === 'Escape' && !selectedId.value) search.value = '';
}
onMounted(() => { void refresh(); window.addEventListener('keydown', keys); });
onUnmounted(() => window.removeEventListener('keydown', keys));
</script>
<template>
  <div class="window">
    <header class="topbar">
      <h1 class="brand"><span class="brand-mark" aria-hidden="true"><svg viewBox="0 0 64 64"><rect x="8" y="8" width="48" height="48" rx="15" fill="none" stroke="currentColor" stroke-width="5" pathLength="100" stroke-dasharray="9 18 73 0"/><circle class="link-dot" :class="{ off: !displayed?.live }" cx="51.6" cy="12.4" r="4.5"/><rect x="20" y="20" width="24" height="24" rx="7" fill="currentColor" stroke="none"/></svg></span>AppHost</h1>
      <label class="search-wrap"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14Zm5-2 5 5"/></svg><input ref="searchInput" v-model="search" type="search" placeholder="搜索" aria-label="搜索应用"></label>
      <button class="refresh icon-btn" :disabled="loading" aria-label="刷新应用与状态" title="刷新应用与状态" @click="refresh">↻</button>
    </header>
    <div class="observation" role="status" aria-live="polite">
      <span v-if="loading">正在读取…</span>
      <span v-else-if="error">{{ error.message }} <small>{{ error.code }}</small></span>
      <span v-else-if="displayed?.runtime_error">{{ displayed.runtime_error.message }} <small>{{ displayed.runtime_error.code }}</small></span>
      <span v-else-if="displayed">{{ displayed.live ? '状态快照' : '后台未连接 · 历史记录' }} · {{ new Date(displayed.read_at_ms).toLocaleTimeString() }}</span>
      <small v-if="displayed && (error || displayed.runtime_error)"> · 上次读取 {{ new Date(displayed.read_at_ms).toLocaleTimeString() }}</small>
    </div>
    <div class="workspace">
      <main :aria-busy="loading">
        <div v-if="displayed?.apps.length" class="board" :data-offline="!displayed.live" data-at-start data-at-end>
          <div v-if="filtered.length" class="canvas"><section class="group" aria-labelledby="all-apps"><h2 id="all-apps" class="group-title">应用</h2><div class="group-grid">
            <AppTile v-for="app in filtered" :key="app.installation_id" :app="app" :snapshot="displayed" @details="details" />
          </div></section></div>
          <div v-else class="empty"><h2>没有匹配的应用</h2><button @click="search = ''">清除搜索</button></div>
        </div>
        <div v-else-if="!loading" class="empty">
          <h2>{{ error ? '暂时无法读取应用' : '还没有应用' }}</h2>
          <p>{{ error ? '检查后台或目录后重试。' : '已登记的应用会出现在这里。' }}</p>
          <button @click="refresh">重新读取</button>
        </div>
      </main>
      <AppDetails v-if="selected && displayed" :app="selected" :snapshot="displayed" @close="closeDetails" />
    </div>
  </div>
</template>
