<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDetailRead } from '../useDetailRead';
import { errorMessage } from '../model';
import { validRetention, warningsText, type Retention } from '../settings';
const {data, error, loading, refresh} = useDetailRead(() => invoke<Retention>('desktop_retention_read'));
const days = ref<number | string>(''), mib = ref<number | string>(''), saving = ref(false), message = ref(''), warning = ref('');
const busy = computed(() => loading.value || saving.value);
const valid = computed(() => validRetention(days.value,mib.value));
const dirty = computed(() => !!data.value && (days.value !== data.value.days || mib.value !== data.value.max_mib));
let disposed = false; onUnmounted(() => { disposed = true; });
watch(data, value => { if (value) { days.value = value.days; mib.value = value.max_mib; } });
watch([days,mib], () => { message.value = ''; });
async function save() {
  if (busy.value || !valid.value || !dirty.value || error.value) return;
  saving.value = true; message.value = ''; warning.value = '';
  try {
    const saved = await invoke<Retention>('desktop_retention_save',{days:days.value,maxMib:mib.value});
    if (disposed) return;
    data.value = saved; warning.value = warningsText(saved.warnings);
    await nextTick(); if (disposed) return; message.value = '已保存，下次启动或完整重启清理归档时生效。';
  } catch (cause) { if (!disposed) { const e = errorMessage(cause); message.value = `${e.code} · ${e.message}`; } }
  finally { saving.value = false; }
}
onMounted(refresh);
</script>
<template>
  <section class="settings-page" aria-labelledby="retention-title" :aria-busy="busy">
    <div class="panel-toolbar"><h2 id="retention-title">日志保留</h2><button :disabled="busy" @click="refresh">{{ dirty ? '放弃修改并重新读取' : '重新读取' }}</button></div>
    <p v-if="loading" class="muted">正在读取…</p><p v-if="error" class="notice" role="status">{{ error.code }} · {{ error.message }}</p>
    <template v-if="data">
      <label class="setting"><span><strong>保留天数</strong><small>1–365 天</small></span><input v-model.number="days" type="number" min="1" max="365" step="1" :disabled="busy || !!error" aria-label="日志保留天数"></label>
      <label class="setting"><span><strong>每个应用的容量上限</strong><small>10–10240 MiB，默认 100 MiB</small></span><input v-model.number="mib" type="number" min="10" max="10240" step="1" :disabled="busy || !!error" aria-label="每个应用日志容量 MiB"></label>
      <p v-if="!valid" class="notice" role="status">请输入范围内的整数。</p>
      <p class="note">时间或容量先达到时清理较早的普通归档。降低上限不会立即删除日志，在下次启动或完整重启清理时生效；活动日志、失败现场和操作记录沿用各自规则。</p>
      <button class="primary" :disabled="busy || !valid || !dirty || !!error" @click="save">保存日志保留设置</button>
      <p v-if="message" class="settings-message" role="status">{{ message }}</p><p v-if="warning" class="notice">{{ warning }}</p>
    </template>
  </section>
</template>
