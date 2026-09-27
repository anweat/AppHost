<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke, isTauri } from '@tauri-apps/api/core';
import ImportPanel from './components/ImportPanel.vue';
import TileBoard from './components/TileBoard.vue';
import AppIcon from './components/AppIcon.vue';
import ManagementPanel from './components/ManagementPanel.vue';
import AppDetails from './components/AppDetails.vue';
import FirstRunLogin from './components/FirstRunLogin.vue';
import type { Feedback } from './useFeedback';
import DiagnosticPreview from './components/DiagnosticPreview.vue';
import {commandText} from './diagnostics';
import { runtimeBlocked, type Snapshot } from './model';
import { defaults, type Preferences } from './preferences';
import { searchApps, type SearchAction } from './search';
import { useRuntime } from './useRuntime';
import { useSnapshot } from './useSnapshot';
const { displayed, loading, error, refresh, invalidate, connected, offline } = useSnapshot(async () => {
  if (!isTauri()) throw { code: 'DESKTOP_REQUIRED', message: '请在 AppHost 桌面窗口中查看应用。' };
  return invoke<Snapshot>('desktop_snapshot');
});
const runtime = useRuntime(displayed, invalidate, () => { void invalidate(); }, connected, offline);
const diagnostic=ref<{id:string;failureId:string}>();
const loginPrompt = ref<InstanceType<typeof FirstRunLogin>>();
const view = ref('home'), lens = ref<string | null>(null), connectionOpen = ref(false);
const managementOpened = ref(false), managementSection = ref('appearance'), environmentApp = ref('');
function manage(section = managementSection.value, id = environmentApp.value) { managementOpened.value = true; managementSection.value = section; environmentApp.value = id; view.value = 'manage'; }
const preferences = ref<Preferences>(defaults());
const theme = ref('system'), savingTheme = ref(false), preferenceError = ref('');
function applyTheme(value: string) { theme.value = value; document.documentElement.dataset.theme = value === 'system' ? '' : value; }
async function loadTheme() {
  if (!isTauri()) return;
  try { preferences.value = {...defaults(),...await invoke<Preferences>('desktop_preferences')}; applyTheme(preferences.value.theme); }
  catch { preferenceError.value = '外观设置读取失败，已保留原文件。'; }
}
async function saveTheme(value: string) {
  savingTheme.value = true;
  try { preferences.value=await invoke<Preferences>('desktop_theme', { theme: value }); applyTheme(preferences.value.theme); preferenceError.value = ''; }
  catch { preferenceError.value = '外观设置保存失败，请检查数据目录。'; }
  finally { savingTheme.value = false; }
}
async function editPreference(change: unknown) {
  if (savingTheme.value) return;
  savingTheme.value=true;
  try {preferences.value=await invoke<Preferences>('desktop_ui_edit',{change});preferenceError.value='';}
  catch(e) {preferenceError.value='布局或搜索设置未保存，请刷新后重试。';}
  finally {savingTheme.value=false;}
}
const counts = computed(() => ({
  running: displayed.value?.apps.filter(app => app.runtime?.status.state === 'STARTING' || app.runtime?.services.some(service => service.alive)).length || 0,
  failed: displayed.value?.apps.filter(app => app.runtime?.status.state === 'FAILED' && !app.requires_confirmation).length || 0,
  review: displayed.value?.apps.filter(app => app.requires_confirmation || app.registration_state === 'SOURCE_MISSING').length || 0,
}));
const activityOpen=ref(false), clock=ref(Date.now());
let clockTimer:ReturnType<typeof setInterval>|undefined;
watch(()=>displayed.value?.live && displayed.value.apps.some(a=>a.runtime?.status.state==='STARTING'),active=>{clearInterval(clockTimer);if(active)clockTimer=setInterval(()=>clock.value=Date.now(),1000);});
const startups=computed(()=>displayed.value?.live ? displayed.value.apps.filter(a=>a.runtime?.status.state==='STARTING' && clock.value-(a.runtime.created_at_ms||clock.value)>5000):[]);
watch(()=>startups.value.length, count=>{if(count)activityOpen.value=true;});
const search = ref('');
const searchInput = ref<HTMLInputElement>();
const selectedId = ref<string | null>(null);
const detailTab = ref('overview');
const importing = ref(false), importBusy = ref(false), editorDirty = ref(false), discardPrompt = ref(false);
let afterDiscard: (() => void) | undefined;
function guard(change: () => void) {
  if (editorDirty.value) { afterDiscard = change; discardPrompt.value = true; }
  else change();
}
function discard() { editorDirty.value = false; discardPrompt.value = false; afterDiscard?.(); afterDiscard = undefined; }
function add() { if (importing.value) return; guard(() => { selectedId.value = null; importing.value = true; view.value = 'home'; }); }
const selected = computed(() => displayed.value?.apps.find(app => app.installation_id === selectedId.value));
const results = computed(() => searchApps(displayed.value?.apps || [],search.value,preferences.value));
const filtered = computed(() => results.value.apps);
async function primary(id: string) {
  const app = displayed.value?.apps.find(item => item.installation_id === id); if (!app) return;
  if (runtimeBlocked(displayed.value) || app.requires_confirmation || app.registration_state === 'SOURCE_MISSING') { void details(id); return; }
  void editPreference({kind:'touch',installation_id:id});
  const state = displayed.value?.live ? app.runtime?.status.state : 'STOPPED';
  if (state === 'STARTING' || state === 'STOPPING') return;
  if (state === 'RUNNING') {
    if (app.has_entry && app.runtime?.operation_id) await runtime.open(id, { instance: app.runtime.instance_id, operation: app.runtime.operation_id });
    else void details(id, 'logs');
  } else await runtime.action(state === 'FAILED' && app.runtime?.services.some(service => service.alive) ? 'restart' : 'start', id);
}
async function feedbackAction(id:string, item:Feedback) {
  const action=item.cleared ? 'retry' : item.report?.advice?.actions.find(a=>a.primary)?.id;
  if(action==='retry') await runtime.action('restart',id);
  else if(action==='stop_owner_and_start') await runtime.action('stop-owner-and-start',id,item.report?.failure?.failure_id);
  else if(action==='relink') manage('applications',id);
  else if(action==='view_environment') manage('environment',id);
  else if(action==='preview_diagnostics' && item.report?.failure) diagnostic.value={id,failureId:item.report.failure.failure_id};
  else if(action==='inspect_daemon') connectionOpen.value=true;
  else if(item.report?.advice?.actions.find(a=>a.primary)?.copy_only) {
    try { await invoke('desktop_copy',{text:commandText(item.report.advice.actions.find(a=>a.primary)!.argv)}); }
    catch {runtime.message.value='建议命令未能复制，请在失败详情中查看。';}
  } else await details(id,['edit_manifest','review_manifest'].includes(action||'')?'manifest':'failure');
}
async function searchExecute(action:SearchAction, id:string) {
  if (action === 'primary') await primary(id);
  else if (['start','stop','restart'].includes(action)) await runtime.action(action as 'start'|'stop'|'restart', id);
  else await details(id, action === 'details'?'overview':action);
  search.value='';
}
function searchEnter(event:KeyboardEvent) {
  if(event.isComposing || event.keyCode===229) return;
  const first=filtered.value[0]; if(!first)return;
  event.preventDefault();
  const normal=preferences.value.search.enter === 'details'?'details':'primary';
  const action=results.value.action || (results.value.onlyActions ? results.value.actions[0]! : event.shiftKey ? normal === 'details'?'primary':'details' : normal);
  void searchExecute(action,first.installation_id);
}
async function batch(kind:'start'|'stop', ids:string[]) {
  const epoch=runtime.intentEpoch(), versions=new Map(ids.map(id=>[id,runtime.intentVersion(id)]));
  for(const id of ids) {
    if(kind==='start' && runtime.intentEpoch()!==epoch)break;
    if(kind==='start' && runtime.intentVersion(id)!==versions.get(id))continue;
    const app=displayed.value?.apps.find(a=>a.installation_id===id); if(!app)continue;
    if(kind==='start' && (app.requires_confirmation || app.registration_state==='SOURCE_MISSING')) {runtime.message.value=`${app.label} · 已跳过，请先处理启动声明或项目路径`;continue;}
    await runtime.action(kind,id,undefined,true);
  }
}
let returnFocus: HTMLElement | null = null;
async function details(id: string, tab = 'overview') {
  if (importing.value && importBusy.value) return;
  if (editorDirty.value && selectedId.value !== id) { guard(() => { void details(id, tab); }); return; }
  importing.value = false;
  returnFocus = document.activeElement as HTMLElement;
  selectedId.value = id;
  detailTab.value = tab;
  await nextTick();
  document.querySelector<HTMLButtonElement>('.drawer button')?.focus();
}
async function closeDetails() {
  if (editorDirty.value) { guard(() => { void closeDetails(); }); return; }
  selectedId.value = null;
  await nextTick();
  if (returnFocus?.isConnected) returnFocus.focus();
}
function keys(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault(); searchInput.value?.focus();
  }
  const target=event.target as HTMLElement;
  const canvasTyping = view.value==='home' && !selectedId.value && !importing.value && !event.metaKey && !event.ctrlKey && !event.altKey && !target.closest('input,textarea,select,[role=menu],[contenteditable=true]');
  // macOS IME's first key can be 229 before isComposing becomes true.
  if (event.keyCode === 229 || event.isComposing) { if(canvasTyping) searchInput.value?.focus(); return; }

  if (view.value==='home' && !selectedId.value && !importing.value && !event.metaKey && !event.ctrlKey && !event.altKey && event.key.length===1 && event.key!==' ' && !event.isComposing && !target.closest('input,textarea,select,[role=menu],[contenteditable=true]')) {event.preventDefault();search.value+=event.key;searchInput.value?.focus();}
  if (event.key === 'Escape') {
    if (discardPrompt.value) { discardPrompt.value = false; afterDiscard = undefined; return; }
    // macOS WebView buttons can leave focus on the page after a panel changes.
    // Escape must still close the visible non-modal details and restore its tile.
    if (selectedId.value) void closeDetails();
    else if (importing.value && !importBusy.value) importing.value = false;
    else search.value = '';
    connectionOpen.value = false;
  }
}
onMounted(() => { void loadTheme(); void refresh(); runtime.startWatch(); window.addEventListener('keydown', keys); });
onUnmounted(() => { runtime.dispose(); clearInterval(clockTimer); window.removeEventListener('keydown', keys); });
</script>
<template>
  <div class="window">
    <header class="topbar">
      <button class="brand" aria-label="后台连接状态" :title="displayed?.live ? '后台已连接' : '后台未连接'" @click="connectionOpen = !connectionOpen">
        <span class="brand-mark" aria-hidden="true"><svg viewBox="0 0 64 64"><rect x="8" y="8" width="48" height="48" rx="15" fill="none" stroke="currentColor" stroke-width="5" pathLength="100" stroke-dasharray="9 18 73 0"/><circle class="link-dot" :class="{ off: !displayed?.live }" cx="51.6" cy="12.4" r="4.5"/><rect x="20" y="20" width="24" height="24" rx="7" fill="currentColor" stroke="none"/></svg></span><span class="brand-name">AppHost</span>
      </button>
      <label class="search-wrap"><AppIcon name="search"/><input ref="searchInput" v-model="search" type="search" placeholder="搜索" aria-label="搜索应用" @focus="view = 'home'" @keydown.enter="searchEnter"><kbd>⌘K</kbd></label>
      <div class="bar-status" role="toolbar" aria-label="按状态突出显示">
        <button v-for="(count, key) in counts" :key="key" class="lens" :title="{running:'运行中', failed:'失败', review:'待处理'}[key]" :aria-label="`${{running:'运行中', failed:'失败', review:'待处理'}[key]} ${count}`" :aria-pressed="lens === key" @click="lens = lens === key ? null : key"><i class="chip-dot" :class="{running:'run', failed:'fail', review:'review'}[key]"/><b>{{ count }}</b></button>
      </div>
      <button v-if="displayed?.automatic_wake_paused" class="lens paused" :disabled="!displayed.live" title="恢复自动唤醒" @click="activityOpen=true; runtime.action('resume-wake')">恢复唤醒</button>
      <button class="stop-all" :disabled="!runtime.canStopAll.value || runtime.busy.value.has('stop-all')" @click="activityOpen=true; runtime.action('stop-all')"><AppIcon name="stop"/><span>全部停止</span></button>
      <button class="lens" :aria-expanded="activityOpen" @click="activityOpen=!activityOpen">活动 {{ runtime.activities.value.size || startups.length || '' }}</button>
      <section v-if="activityOpen" class="activity-panel" aria-label="活动区"><div class="panel-toolbar"><strong>当前操作</strong><button @click="activityOpen=false">收起</button></div><p v-if="runtime.message.value" class="muted">{{ runtime.message.value }}</p><p v-for="item in runtime.activities.value.values()" :key="item.id">{{ item.label }} · {{ item.text }}</p><p v-for="app in startups" :key="app.installation_id">{{ app.label }} · {{ app.runtime?.started_at_ms ? `已等待 ${Math.max(0,Math.floor((clock-app.runtime.started_at_ms)/1000))} 秒` : '正在准备启动' }}<span v-for="service in app.runtime?.services.filter(s=>!s.ready)" :key="service.name"> · {{ service.name }} 期限 {{ (service.startup_timeout_ms||0)/1000 }} 秒</span></p><p v-if="!runtime.activities.value.size && !startups.length" class="muted">暂无操作</p><button @click="manage('journal');activityOpen=false">查看操作记录</button></section>
      <div class="bar-actions"><button class="icon-btn" aria-label="添加应用" @click="add">＋</button><button class="icon-btn" aria-label="刷新应用与状态" :disabled="loading" @click="refresh"><AppIcon name="refresh"/></button>
        <button class="icon-btn" aria-label="管理" :aria-current="view === 'manage' ? 'page' : undefined" @click="view === 'manage' ? view = 'home' : manage()"><AppIcon name="settings"/></button>
      </div>
      <section v-if="connectionOpen" class="connection-panel" aria-label="后台连接">
        <strong>{{ displayed?.live ? '后台已连接' : '后台未连接' }}</strong>
        <p v-if="displayed">上次读取 {{ new Date(displayed.read_at_ms).toLocaleTimeString() }}</p>
        <p v-if="error || displayed?.runtime_error">{{ error?.message || displayed?.runtime_error?.message }}</p>
        <button :disabled="runtime.connecting.value" @click="runtime.connect">{{ runtime.connecting.value ? '正在连接…' : '连接后台' }}</button>
      </section>
    </header>
    <div v-if="(runtime.message.value && !runtime.activities.value.size) || preferenceError || (error && error.code !== 'DAEMON_UNAVAILABLE')" class="activity-result" role="status"><span>{{ preferenceError || runtime.message.value || error?.message }}</span><button class="icon-btn" aria-label="收起提示" @click="runtime.message.value = ''; preferenceError = ''"><AppIcon name="close"/></button></div>
    <div v-if="discardPrompt" class="discard-prompt" role="alertdialog" aria-label="未保存的启动声明"><p>启动声明尚未保存，丢弃编辑？</p><button @click="discard">丢弃编辑</button><button class="primary" @click="discardPrompt = false; afterDiscard = undefined">继续编辑</button></div>
    <FirstRunLogin ref="loginPrompt"/>
    <DiagnosticPreview v-if="diagnostic" :installation-id="diagnostic.id" :failure-id="diagnostic.failureId" @close="diagnostic=undefined"/>
    <div class="workspace">
      <main :aria-busy="loading">
        <ManagementPanel v-if="managementOpened" v-show="view === 'manage'" v-model:section="managementSection" v-model:installation-id="environmentApp" :apps="displayed?.apps || []" :theme="theme" :saving="savingTheme" :preferences="preferences" @preference="editPreference" @theme="saveTheme" @home="view = 'home'" @changed="invalidate(); runtime.resumeWatch(); loginPrompt?.refresh()" @review="view = 'home'; details($event, 'manifest')"/>
        <section v-if="view === 'home' && search && results.showActions" class="search-actions" aria-label="操作搜索结果"><div v-for="app in filtered.slice(0,12)" :key="app.installation_id"><button v-for="action in results.actions" :key="action" :disabled="runtimeBlocked(displayed) && ['start','stop','restart'].includes(action)" @click="searchExecute(action,app.installation_id)">{{ {start:'启动',stop:'停止',restart:'重启',logs:'日志',details:'详情',failure:'失败现场',manifest:'声明',primary:'打开'}[action] }} {{ app.label }}</button></div></section>
        <TileBoard v-if="view === 'home' && filtered.length && displayed && !(results.onlyActions && results.showActions)" :apps="filtered" :snapshot="displayed" :lens="lens" :preferences="preferences" :feedback="runtime.feedback.value" @dismiss="runtime.dismiss" @feedback-action="feedbackAction" @preference="editPreference" @batch="batch" @action="searchExecute" @manage="manage('groups')" @details="details" @primary="primary"/>
        <div v-else-if="view === 'home' && !loading && !filtered.length" class="empty"><h2>{{ error ? '暂时无法读取应用' : displayed?.apps.length ? '没有匹配的应用' : '还没有应用' }}</h2><p>{{ error ? '检查后台或目录后重试。' : displayed?.apps.length ? '试试其他名称。' : '选择项目目录，确认后加入磁贴。' }}</p><button v-if="search" @click="search = ''">清除搜索</button><button v-else-if="!error" class="primary" @click="add">＋ 添加应用</button><button v-else @click="refresh">重新读取</button></div>
      </main>
      <AppDetails v-if="selected && displayed" v-show="view === 'home'" :key="selected.installation_id" v-model:tab="detailTab" :app="selected" :snapshot="displayed" :busy="runtime.busy.value.has(selected.installation_id)" @action="runtime.action($event, selected.installation_id)" @close="closeDetails" @changed="runtime.message.value = ''; invalidate(); if (!displayed?.live) runtime.resumeWatch()" @dirty="editorDirty = $event" @environment="manage('environment', selected.installation_id)" @relink="manage('applications', selected.installation_id)" @owner="runtime.action('stop-owner-and-start', selected.installation_id, $event)" />
      <ImportPanel v-if="importing" @close="importing = false" @busy="importBusy = $event" @changed="invalidate"/>
    </div>
  </div>
</template>
