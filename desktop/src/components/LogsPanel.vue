<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import AppIcon from './AppIcon.vue';
import { useDetailRead } from '../useDetailRead';
import { logEntries, logWarnings, type LogResult } from '../logs';
const props = defineProps<{ installationId: string; instanceId: string; live: boolean }>();
const { data, error, loading, refresh } = useDetailRead(() => invoke<LogResult>('desktop_logs', { installationId: props.installationId, instanceId: props.instanceId }));
const entries = computed(() => data.value ? logEntries(data.value) : []);
onMounted(refresh);
</script>
<template>
  <section aria-label="应用日志" :aria-busy="loading">
    <div class="panel-toolbar"><span class="muted">{{ !live || error ? '上次日志 · 未同步' : '日志快照' }}</span><button :disabled="loading || !live" @click="refresh"><AppIcon name="refresh"/>刷新日志</button></div>
    <p class="muted">最近五分钟 · 最多 64 KiB · 手动刷新</p>
    <p v-if="error" class="notice" role="status">{{ error.code }} · {{ error.message }}</p>
    <p v-else-if="!live" class="notice">后台未连接，无法刷新日志。</p>
    <template v-if="data">
      <p v-for="warning in logWarnings(data)" :key="warning" class="notice">{{ warning }}</p>
      <div class="log-output" tabindex="0" aria-label="日志内容"><div v-for="entry in entries" :key="entry.sequence" class="log-entry"><small>{{ new Date(entry.observed_ms).toLocaleTimeString() }} · {{ entry.service }} · {{ entry.stream }}</small><pre>{{ entry.text }}</pre></div><p v-if="!entries.length" class="muted">这个时间窗口没有输出。</p></div>
      <p class="muted">读取于 {{ new Date(data.read_at_ms).toLocaleTimeString() }}</p>
    </template>
    <p v-else-if="loading" class="muted">正在读取…</p>
  </section>
</template>
