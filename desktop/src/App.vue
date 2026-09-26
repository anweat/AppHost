<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke, isTauri } from '@tauri-apps/api/core';
import TileBoard from './components/TileBoard.vue';
import AppIcon from './components/AppIcon.vue';
import AppearanceSettings from './components/AppearanceSettings.vue';
import AppDetails from './components/AppDetails.vue';
import { type Snapshot } from './model';
import { useRuntime } from './useRuntime';
import { useSnapshot } from './useSnapshot';
const { displayed, loading, error, refresh, invalidate, connected, offline } = useSnapshot(async () => {
  if (!isTauri()) throw { code: 'DESKTOP_REQUIRED', message: '请在 AppHost 桌面窗口中查看应用。' };
  return invoke<Snapshot>('desktop_snapshot');
});
const runtime = useRuntime(displayed, invalidate, () => { void invalidate(); }, connected, offline);
const view = ref('home'), lens = ref<string | null>(null), connectionOpen = ref(false);
const theme = ref('system'), savingTheme = ref(false), preferenceError = ref('');
function applyTheme(value: string) { theme.value = value; document.documentElement.dataset.theme = value === 'system' ? '' : value; }
async function loadTheme() {
  if (!isTauri()) return;
  try { applyTheme((await invoke<{theme: string}>('desktop_preferences')).theme); }
  catch { preferenceError.value = '外观设置读取失败，已保留原文件。'; }
}
async function saveTheme(value: string) {
  savingTheme.value = true;
  try { applyTheme((await invoke<{theme: string}>('desktop_theme', { theme: value })).theme); preferenceError.value = ''; }
  catch { preferenceError.value = '外观设置保存失败，请检查数据目录。'; }
  finally { savingTheme.value = false; }
}
const counts = computed(() => ({
  running: displayed.value?.apps.filter(app => app.runtime?.status.state === 'STARTING' || app.runtime?.services.some(service => service.alive)).length || 0,
  failed: displayed.value?.apps.filter(app => app.runtime?.status.state === 'FAILED' && !app.requires_confirmation).length || 0,
  review: displayed.value?.apps.filter(app => app.requires_confirmation || app.registration_state === 'SOURCE_MISSING').length || 0,
}));
const search = ref('');
const searchInput = ref<HTMLInputElement>();
const selectedId = ref<string | null>(null);
const selected = computed(() => displayed.value?.apps.find(app => app.installation_id === selectedId.value));
const filtered = computed(() => displayed.value?.apps.filter(app => app.label.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase())) ?? []);
async function primary(id: string) {
  const app = displayed.value?.apps.find(item => item.installation_id === id); if (!app) return;
  if (!displayed.value?.live) { connectionOpen.value = true; return; }
  if (app.requires_confirmation || app.registration_state === 'SOURCE_MISSING') { void details(id); return; }
  const state = app.runtime?.status.state;
  if (state === 'STARTING' || state === 'STOPPING') return;
  if (state === 'RUNNING') {
    if (app.has_entry && app.runtime?.operation_id) await runtime.open(id, { instance: app.runtime.instance_id, operation: app.runtime.operation_id });
    else void details(id);
  } else await runtime.action(state === 'FAILED' && app.runtime?.services.some(service => service.alive) ? 'restart' : 'start', id);
}
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
onMounted(() => { void loadTheme(); void refresh(); runtime.startWatch(); window.addEventListener('keydown', keys); });
onUnmounted(() => { runtime.dispose(); window.removeEventListener('keydown', keys); });
</script>
<template>
  <div class="window">
    <header class="topbar">
      <button class="brand" aria-label="后台连接状态" :title="displayed?.live ? '后台已连接' : '后台未连接'" @click="connectionOpen = !connectionOpen">
        <span class="brand-mark" aria-hidden="true"><svg viewBox="0 0 64 64"><rect x="8" y="8" width="48" height="48" rx="15" fill="none" stroke="currentColor" stroke-width="5" pathLength="100" stroke-dasharray="9 18 73 0"/><circle class="link-dot" :class="{ off: !displayed?.live }" cx="51.6" cy="12.4" r="4.5"/><rect x="20" y="20" width="24" height="24" rx="7" fill="currentColor" stroke="none"/></svg></span><span class="brand-name">AppHost</span>
      </button>
      <label class="search-wrap"><AppIcon name="search"/><input ref="searchInput" v-model="search" type="search" placeholder="搜索" aria-label="搜索应用" @focus="view = 'home'"><kbd>⌘K</kbd></label>
      <div class="bar-status" role="toolbar" aria-label="按状态突出显示">
        <button v-for="(count, key) in counts" :key="key" class="lens" :title="{running:'运行中', failed:'失败', review:'待处理'}[key]" :aria-label="`${{running:'运行中', failed:'失败', review:'待处理'}[key]} ${count}`" :aria-pressed="lens === key" @click="lens = lens === key ? null : key"><i class="chip-dot" :class="{running:'run', failed:'fail', review:'review'}[key]"/><b>{{ count }}</b></button>
      </div>
      <button v-if="displayed?.automatic_wake_paused" class="lens paused" :disabled="!displayed.live" title="恢复自动唤醒" @click="runtime.action('resume-wake')">恢复唤醒</button>
      <button class="stop-all" :disabled="!displayed?.live || (!counts.running && runtime.busy.value.size === 0) || runtime.busy.value.has('stop-all')" @click="runtime.action('stop-all')"><AppIcon name="stop"/><span>全部停止</span></button>
      <div class="bar-actions"><button class="icon-btn" aria-label="刷新应用与状态" :disabled="loading" @click="refresh"><AppIcon name="refresh"/></button>
        <button class="icon-btn" aria-label="管理" :aria-current="view === 'manage' ? 'page' : undefined" @click="view = view === 'manage' ? 'home' : 'manage'"><AppIcon name="settings"/></button>
      </div>
      <section v-if="connectionOpen" class="connection-panel" aria-label="后台连接">
        <strong>{{ displayed?.live ? '后台已连接' : '后台未连接' }}</strong>
        <p v-if="displayed">上次读取 {{ new Date(displayed.read_at_ms).toLocaleTimeString() }}</p>
        <p v-if="error || displayed?.runtime_error">{{ error?.message || displayed?.runtime_error?.message }}</p>
        <button :disabled="runtime.connecting.value" @click="runtime.connect">{{ runtime.connecting.value ? '正在连接…' : '连接后台' }}</button>
      </section>
    </header>
    <div v-if="runtime.message.value || preferenceError || (error && error.code !== 'DAEMON_UNAVAILABLE')" class="activity-result" role="status"><span>{{ preferenceError || runtime.message.value || error?.message }}</span><button class="icon-btn" aria-label="收起提示" @click="runtime.message.value = ''; preferenceError = ''"><AppIcon name="close"/></button></div>
    <div class="workspace">
      <main :aria-busy="loading">
        <AppearanceSettings v-if="view === 'manage'" :theme="theme" :saving="savingTheme" @theme="saveTheme" @home="view = 'home'"/>
        <TileBoard v-else-if="filtered.length && displayed" :apps="filtered" :snapshot="displayed" :lens="lens" @details="details" @primary="primary"/>
        <div v-else-if="!loading" class="empty"><h2>{{ error ? '暂时无法读取应用' : displayed?.apps.length ? '没有匹配的应用' : '还没有应用' }}</h2><p>{{ error ? '检查后台或目录后重试。' : displayed?.apps.length ? '试试其他名称。' : '已登记的应用会出现在这里。' }}</p><button v-if="search" @click="search = ''">清除搜索</button><button v-else @click="refresh">重新读取</button></div>
      </main>
      <AppDetails v-if="selected && displayed && view === 'home'" :app="selected" :snapshot="displayed" :busy="runtime.busy.value.has(selected.installation_id)" @action="runtime.action($event, selected.installation_id)" @close="closeDetails" />
    </div>
  </div>
</template>
