<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { commandText } from '../diagnostics';
import type { Draft, ScanResult } from '../registration';
import { errorMessage } from '../model';
const emit = defineEmits<{close: []; changed: []; busy: [value: boolean]}>();
const token = ref('');
const draft = ref<Draft | null>(null), busy = ref(false), message = ref(''), finished = ref(false);
watch(busy, value => emit('busy', value));
const count = computed(() => draft.value?.candidates.filter(c => c.selected).length || 0);
async function scan() {
  if (busy.value) return;
  busy.value = true; message.value = '';
  try {
    const result = await invoke<ScanResult | null>('desktop_scan');
    if (result) { result.draft.candidates.forEach(c => c.selected = false); token.value = result.token; draft.value = result.draft; finished.value = false; }
    else if (!draft.value) emit('close');
  } catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; }
  finally { busy.value = false; }
}
async function group(index: number, merge: boolean) {
  if (!draft.value || busy.value) return;
  busy.value = true; message.value = '';
  try { const result = await invoke<ScanResult>('desktop_group', {token:token.value,index,merge}); token.value = result.token; result.draft.candidates.forEach(c => c.selected = false); draft.value = result.draft; }
  catch (error) { message.value = errorMessage(error).message; }
  finally { busy.value = false; }
}
async function save() {
  if (!draft.value || busy.value || !count.value || draft.value.groups.length || finished.value) return;
  busy.value = true; message.value = '';
  try {
    const result = await invoke<{completed: unknown[]}>('desktop_import', {token:token.value,selected:draft.value.candidates.flatMap((c,i) => c.selected ? [i] : [])});
    message.value = `已登记 ${result.completed.length} 个应用`; finished.value = true;
  } catch (error) {
    const completed = (error as {details?: {completed?: unknown[]}})?.details?.completed?.length || 0;
    message.value = `${completed ? `已登记 ${completed} 个应用；` : ''}${errorMessage(error).code} · ${errorMessage(error).message}。请重新扫描后继续。`;
    // A failed batch can have written files. Never silently repeat the old draft.
    finished.value = true;
  } finally { emit('changed'); busy.value = false; }
}
onMounted(scan);
</script>
<template>
  <aside @keydown.esc.stop="!busy && emit('close')" class="drawer import-panel" aria-labelledby="import-title" :aria-busy="busy">
    <div class="detail-heading"><h2 id="import-title">添加应用</h2><button class="icon-btn" aria-label="关闭添加应用" :disabled="busy" @click="emit('close')">×</button></div>
    <div class="detail-body">
      <div class="panel-toolbar"><span>{{ busy ? '正在处理…' : '选择并确认启动声明' }}</span><button :disabled="busy" @click="scan">选择目录</button></div>
      <p v-if="message" class="notice" role="status">{{ message }}</p>
      <template v-if="draft">
        <p class="muted path-text">{{ draft.root }}</p>
        <p v-for="(issue,i) in draft.issues" :key="i" class="notice">{{ issue.message }}</p>
        <fieldset :disabled="busy || finished" class="import-choices">
          <section v-for="(suggestion,i) in draft.groups" :key="suggestion.candidate.path" class="import-candidate">
            <strong>是否合为一个应用？</strong><p class="path-text">{{ suggestion.members.join('\n') }}</p>
            <button @click="group(i,true)">合并</button><button @click="group(i,false)">分别登记</button>
          </section>
          <section v-for="candidate in draft.candidates" :key="candidate.path" class="import-candidate">
            <label><input v-model="candidate.selected" type="checkbox" :disabled="!!draft.groups.length || !candidate.manifest?.services.length || !!candidate.issues.length">{{ candidate.human_label }}</label>
            <p class="muted path-text">{{ candidate.path }}</p>
            <p v-for="(issue,i) in candidate.issues" :key="i" class="notice">{{ issue.message }}</p>
            <template v-if="candidate.selected">
              <div v-for="service in candidate.manifest?.services" :key="service.name"><strong>{{ service.name }}</strong><pre>{{ commandText(service.command || []) }}</pre><p class="muted">工作目录：{{ service.cwd || '.' }}<span v-if="service.port"> · 端口 {{ service.port }}</span></p></div>
              <details><summary>完整清单与入口</summary><pre class="declaration-preview">{{ JSON.stringify(candidate.manifest, null, 2) }}</pre></details>
              <p v-for="(note,i) in candidate.notes" :key="i" class="notice">{{ note.message }}</p>
            </template>
            <p v-if="!candidate.manifest?.services.length || candidate.issues.length" class="muted">请先用 CLI review 补全声明，再重新扫描。</p>
          </section>
        </fieldset>
        <p v-if="!draft.candidates.length" class="muted">未发现应用，可选择具体项目目录重试。</p>
        <p class="muted">确认后登记所选应用；缺少清单时创建 apphost.toml，不会启动服务。</p>
        <button class="primary" :disabled="busy || finished || !count || !!draft.groups.length" @click="save">确认并登记{{ count ? ` ${count} 个应用` : '' }}</button>
      </template>
    </div>
  </aside>
</template>
