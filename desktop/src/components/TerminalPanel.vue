<script setup lang="ts">
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage } from '../model';
defineProps<{ installationId: string; path: string; missing: boolean }>();
const busy = ref(false), message = ref('');
async function open(id: string) {
  if (busy.value) return;
  busy.value = true;
  try { await invoke('desktop_terminal', {installationId:id}); message.value = '已请求系统终端打开项目目录'; }
  catch (cause) { message.value = errorMessage(cause).message; }
  finally { busy.value = false; }
}
</script>
<template>
  <section aria-label="项目命令行"><p class="muted">项目目录</p><pre>{{ path }}</pre><button :disabled="busy || missing" @click="open(installationId)">打开系统终端 ↗</button><p class="note">仅打开目录，不自动执行启动或修复命令。</p><p role="status" class="muted">{{ message }}</p></section>
</template>
