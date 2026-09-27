<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, type AppEntry } from '../model';
const props = defineProps<{apps: AppEntry[]}>();
const emit = defineEmits<{changed: []; review: [id: string]}>();
const selectedId = defineModel<string>('installationId', {default:''});
const selected = computed(() => props.apps.find(a => a.installation_id === selectedId.value));
const confirmation = ref(false), busy = ref(false), message = ref('');
const pending = ref<{id:string; operation:string; kind:string}>();
let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
onUnmounted(() => { disposed = true; clearTimeout(timer); });
watch(selectedId, () => { confirmation.value = false; if (!pending.value) message.value = ''; });
async function check() {
  if (!pending.value || disposed) return;
  const intent = pending.value;
  try {
    const value = await invoke<{completed:boolean;error?:{code:string;message:string}}>('desktop_operation', {installationId:intent.id,operationId:intent.operation});
    if (disposed || pending.value !== intent) return;
    if (value.completed) {
      pending.value = undefined; busy.value = false;
      message.value = value.error ? `${value.error.code} · ${value.error.message}` : intent.kind === 'detach' ? '已解除登记，项目文件已保留' : '已重新关联，请复核启动声明'; emit('changed');
    } else timer = setTimeout(check, 1000);
  } catch (cause) { if (!disposed) { message.value = `${errorMessage(cause).message}；不会重复提交，可核对操作结果。`; busy.value = false; if (errorMessage(cause).code === 'OPERATION_EXPIRED') { pending.value = undefined; emit('changed'); } } }
}
async function mutate(kind: 'detach' | 'relink') {
  if (!selected.value || busy.value || pending.value) return;
  const id = selected.value.installation_id;
  busy.value = true; confirmation.value = false; message.value = '';
  try {
    const result = await invoke<null | {completed?:boolean; operation_id?:string}>(`desktop_${kind}`, {installationId:id});
    if (disposed || !result) return;
    if (result.operation_id) { pending.value = {id,operation:result.operation_id,kind}; message.value = '已受理，等待服务停止并更新登记'; void check(); }
    else if (result.completed) message.value = kind === 'detach' ? '已解除登记，项目文件已保留' : '已重新关联，请复核启动声明';
    else message.value = '未收到完成确认，请刷新核对；不会重复提交。';
    emit('changed');
  } catch (cause) { message.value = `${errorMessage(cause).code} · ${errorMessage(cause).message}`; emit('changed'); }
  finally { if (!pending.value) busy.value = false; }
}
</script>
<template>
  <section aria-label="应用管理"><h1>应用管理</h1><p class="muted">登记与项目路径</p>
    <label>应用 <select :value="selected?.installation_id || ''" @change="selectedId=($event.target as HTMLSelectElement).value" :disabled="busy || !!pending"><option value="" disabled>选择应用</option><option v-for="app in apps" :key="app.installation_id" :value="app.installation_id">{{ app.label }}</option></select></label>
    <template v-if="selected"><p class="path-text">{{ selected.path }}</p>
      <div class="row-actions"><button :disabled="busy || !!pending" @click="mutate('relink')">重新关联路径…</button><button :disabled="busy || !!pending" @click="confirmation = true">解除登记…</button><button v-if="selected.requires_confirmation" @click="$emit('review', selected.installation_id)">复核启动声明</button></div>
      <p class="muted">重新关联用于已移动的项目，会核对项目身份；原目录仍存在的副本应作为新应用导入。修改前会停止该应用，完成后保持停止。</p>
      <div v-if="confirmation" class="notice" role="alertdialog" aria-label="确认解除登记"><p>解除 {{ selected.label }} 的登记？运行服务会先停止，项目文件不会删除。</p><div class="row-actions"><button @click="confirmation = false">取消</button><button class="primary" @click="mutate('detach')">停止并解除登记</button></div></div>
    </template>
    <p role="status" class="notice" v-if="message">{{ message }}</p><button v-if="pending && !busy" @click="busy = true; check()">核对操作结果</button>
  </section>
</template>
