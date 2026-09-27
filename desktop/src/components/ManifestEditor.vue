<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { declarationDiff, declarationSelection, type EditFocus } from '../manifestDiff';
import ManifestSource from './ManifestSource.vue';
import { commandText } from '../diagnostics';
import type { AppEntry } from '../model';
import { errorMessage } from '../model';
import type { ManifestRead, ManifestReview, SaveReceipt } from '../registration';
const props = defineProps<{app: AppEntry; live: boolean; focus?:EditFocus}>();
const emit = defineEmits<{changed: []; dirty: [value: boolean]}>();
const source = ref<ManifestRead>(), text = ref(''), review = ref<ManifestReview>(), busy = ref(false), message = ref('');
const editor = ref<HTMLTextAreaElement>(), sourceBusy = ref(false);
async function focusRelated() { if (!props.focus) return; await nextTick(); const selection=declarationSelection(text.value,props.focus); editor.value?.focus(); editor.value?.setSelectionRange(selection.start,selection.end); }
watch(() => props.focus, focusRelated);
const pending = ref<string>(), reloadPrompt = ref(false);
let disposed = false;
onUnmounted(() => { disposed = true; });
const dirty = computed(() => !!source.value && text.value !== source.value.text);
watch(dirty, value => emit('dirty', value));
watch(text, () => { review.value = undefined; if (dirty.value) message.value = ''; });
async function read() {
  busy.value = true; reloadPrompt.value = false;
  try {
    const result = await invoke<ManifestRead>('desktop_manifest_read', {installationId:props.app.installation_id});
    if (disposed) return false;
    source.value = result; text.value = result.text; review.value = undefined; pending.value = undefined; void focusRelated(); return true;
  } catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; return false; }
  finally { busy.value = false; }
}
async function inspect() {
  if (busy.value || pending.value) return;
  busy.value = true; message.value = ''; review.value = undefined;
  const buffer = text.value;
  try {
    const result = await invoke<ManifestReview>('desktop_manifest_review', {installationId:props.app.installation_id,text:buffer});
    if (disposed || text.value !== buffer) return;
    if (source.value?.base_sha256 !== result.base_sha256 || source.value?.revision !== result.revision)
      throw {code:'EDIT_CONFLICT',message:'文件或登记已被修改。已保留编辑内容，请先复制需要保留的内容，再重新读取。'};
    review.value = result;
  } catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; }
  finally { busy.value = false; }
}
async function completed(error?: {code: string; message: string} | null) {
  pending.value = undefined;
  review.value = undefined;
  if (error) message.value = `${error.code} · ${error.message}。编辑内容已保留。`;
  else if (await read()) message.value = '启动声明已保存并确认';
  emit('changed');
}
async function save() {
  if (!review.value || busy.value || pending.value) return;
  const confirmed = review.value;
  busy.value = true; message.value = '';
  try {
    const receipt = await invoke<SaveReceipt>('desktop_manifest_save', {installationId:props.app.installation_id,request:{revision:confirmed.revision,base_sha256:confirmed.base_sha256,confirmed_launch_sha256:confirmed.launch_sha256,text:confirmed.after_text}});
    if (disposed) return;
    if (receipt.confirmed) await completed();
    else if (receipt.operation_id) { pending.value = receipt.operation_id; message.value = '已提交，等待后台停止并保存'; emit('changed'); observe(); }
    else throw {code:'SAVE_UNCONFIRMED',message:'未收到保存确认，请刷新状态核对；不会自动重试。'};
  } catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; review.value = undefined; emit('changed'); }
  finally { busy.value = false; }
}
function observe() {
  const runtime = props.app.runtime;
  if (!pending.value || !props.live || runtime?.operation_id !== pending.value || !runtime.operation_done) return;
  void completed(runtime.operation_error);
}
watch(() => [props.app.runtime, props.live], observe);
onMounted(read);
</script>
<template>
  <section aria-label="启动声明" :aria-busy="busy">
    <div class="panel-toolbar"><strong>启动声明</strong><button :disabled="busy || !!pending || sourceBusy" @click="dirty ? reloadPrompt = true : read()">重新读取</button></div>
    <p v-if="message" class="notice" role="status">{{ message }}</p>
    <div v-if="reloadPrompt" class="notice">丢弃未保存的编辑并重新读取？<div class="row-actions"><button @click="read">丢弃并读取</button><button @click="reloadPrompt = false">继续编辑</button></div></div>
    <template v-if="source">
      <button v-if="pending" :disabled="busy" @click="reloadPrompt = true">结束等待，重新读取</button>
      <p class="muted path-text">{{ source.manifest_path }}</p>
      <ManifestSource v-if="source.manifest_source" :installation-id="app.installation_id" :source="source.manifest_source" :disabled="dirty || busy || !!pending" @busy="sourceBusy=$event" @changed="read(); emit('changed')"/>
      <label class="editor-label">清单内容<textarea ref="editor" v-model="text" class="manifest-editor" :readonly="busy || !!pending || sourceBusy" spellcheck="false" aria-label="清单内容"/></label>
      <p v-if="pending" class="muted">{{ live ? '等待操作完成，可刷新应用状态核对。' : '后台未连接，结果尚未确认。重新连接后核对，不会重复提交。' }}</p>
      <template v-if="review">
        <h3>确认变更</h3>
        <details><summary>上次确认的启动声明</summary><pre>{{ JSON.stringify(review.previous_confirmation.manifest || review.previous_confirmation.services, null, 2) }}</pre></details>
        <details><summary>当前文件</summary><pre>{{ review.before_text }}</pre></details>
        <h3>变更差异</h3><pre class="declaration-preview">{{ declarationDiff(review.before_text, review.after_text) }}</pre>
        <h3>执行摘要</h3><div v-for="service in review.manifest?.services" :key="service.name"><strong>{{ service.name }}</strong><pre>{{ commandText(service.command) }}</pre><p class="muted">工作目录：{{ app.path }}/{{ service.cwd || '.' }} · 端口：{{ service.port || '未声明' }}</p><details v-if="Object.keys(service.env || {}).length"><summary>环境变量</summary><pre>{{ JSON.stringify(service.env, null, 2) }}</pre></details></div>
        <details><summary>本次完整声明与入口</summary><pre class="declaration-preview">{{ review.after_text }}</pre></details>
        <p v-for="(warning,i) in review.warnings" :key="i" class="notice">{{ warning.message }}</p>
        <p class="muted">运行中的应用会先停止；保存后保持停止，需手动启动。</p>
        <button class="primary" :disabled="busy || !!pending || sourceBusy || (!!app.runtime && !live)" @click="save">保存并确认启动声明</button>
      </template>
      <button v-else class="primary" :disabled="busy || !!pending || sourceBusy" @click="inspect">复核启动声明</button>
      <p v-if="app.runtime && !live" class="muted">保存前请先连接后台。</p>
    </template>
  </section>
</template>
