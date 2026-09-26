<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDetailRead } from '../useDetailRead';
import { diagnosticText, type DiagnosticPreview } from '../diagnostics';
import { errorMessage } from '../model';
const props = defineProps<{ installationId: string; failureId: string }>();
defineEmits<{ close: [] }>();
const { data, error, loading, refresh } = useDetailRead(() => invoke<DiagnosticPreview>('desktop_diagnostics', { installationId:props.installationId, failureId:props.failureId }));
const text = computed(() => data.value ? diagnosticText(data.value) : '');
const message = ref(''), copying = ref(false);
async function copy() {
  copying.value = true;
  try { await invoke('desktop_copy', {text:text.value}); message.value = '已复制当前预览'; }
  catch (cause) { message.value = errorMessage(cause).message; }
  finally { copying.value = false; }
}
onMounted(refresh);
</script>
<template>
  <section class="diagnostic-preview" aria-label="诊断预览" :aria-busy="loading">
    <div class="panel-toolbar"><strong>诊断预览</strong><button @click="$emit('close')">返回现场</button></div>
    <p class="muted">已尽力脱敏，分享前请检查内容。不会自动发送。</p>
    <p v-if="error" class="notice" role="status">{{ error.code }} · {{ error.message }}</p>
    <p v-if="loading" class="muted">正在生成预览…</p>
    <textarea v-if="data" readonly :value="text" aria-label="待复制的诊断信息" spellcheck="false" />
    <button class="primary" :disabled="!data || loading || copying" @click="copy">复制当前预览</button>
    <p role="status" class="muted">{{ message }}</p>
  </section>
</template>
