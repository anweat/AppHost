<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDetailRead } from '../useDetailRead';
import { errorMessage, type AppEntry } from '../model';
import { warningsText, type Baseline, type EnvironmentCheck } from '../settings';
const props = defineProps<{apps: AppEntry[]}>();
const installationId = defineModel<string>('installationId', {default:''});
const {data, error, loading, refresh} = useDetailRead(() => invoke<Baseline>('desktop_environment_read'));
const path = ref(''), writing = ref(false), checking = ref(false), captureOpen = ref(false), shell = ref('zsh');
const message = ref(''), warning = ref(''), checkError = ref(''), result = ref<EnvironmentCheck>();
const busy = computed(() => loading.value || writing.value || checking.value);
const dirty = computed(() => !!data.value && path.value !== data.value.path);
const checkable = computed(() => props.apps.some(app => app.installation_id === installationId.value));
let disposed = false;
onUnmounted(() => { disposed = true; });
watch(data, value => { if (value) path.value = value.path; });
watch([path, installationId], () => { result.value = undefined; checkError.value = ''; message.value = ''; });
watch(() => props.apps, () => { if (!checkable.value) installationId.value = ''; });
async function write(capture: boolean) {
  if (busy.value || !data.value || error.value) return;
  writing.value = true; message.value = ''; warning.value = ''; result.value = undefined; checkError.value = '';
  try {
    const saved = capture ? await invoke<Baseline>('desktop_environment_capture',{shell:shell.value}) : await invoke<Baseline>('desktop_environment_save',{path:path.value});
    if (disposed) return;
    data.value = saved; path.value = saved.path; captureOpen.value = false;
    warning.value = warningsText(saved.warnings);
    // Let buffer invalidation finish before publishing the save result.
    await nextTick(); if (disposed) return; message.value = 'PATH 已保存，下次启动生效。';
  } catch (cause) { if (!disposed) { const e = errorMessage(cause); message.value = `${e.code} · ${e.message}`; } }
  finally { writing.value = false; }
}
async function check() {
  if (busy.value || dirty.value || !checkable.value || error.value) return;
  checking.value = true; result.value = undefined; checkError.value = '';
  const id = installationId.value, baseline = path.value;
  try { const value = await invoke<EnvironmentCheck>('desktop_environment_check',{installationId:id}); if (!disposed && installationId.value === id && path.value === baseline) result.value = value; }
  catch (cause) { if (!disposed && installationId.value === id) { const e = errorMessage(cause); checkError.value = `${e.code} · ${e.message}`; } }
  finally { checking.value = false; }
}
onMounted(refresh);
</script>
<template>
  <section class="settings-page" aria-labelledby="environment-title" :aria-busy="busy">
    <div class="panel-toolbar"><h2 id="environment-title">运行环境</h2><button :disabled="busy" @click="refresh">{{ dirty ? '放弃修改并重新读取' : '重新读取' }}</button></div>
    <p v-if="loading" class="muted">正在读取…</p>
    <p v-if="error" class="notice" role="status">{{ error.code }} · {{ error.message }}</p>
    <template v-if="data">
      <label class="settings-field"><strong>启动 PATH</strong><small>按优先顺序填写绝对目录，以冒号分隔。</small><textarea v-model="path" :disabled="busy || !!error" rows="5" spellcheck="false" aria-label="启动 PATH"/></label>
      <p v-if="data.capture" class="muted">上次采集：{{ data.capture.shell }} · {{ new Date(data.capture.captured_at_ms).toLocaleString() }}</p>
      <div class="row-actions"><button class="primary" :disabled="busy || !!error || !dirty || !path" @click="write(false)">保存 PATH</button><button :disabled="busy || !!error" :aria-expanded="captureOpen" @click="captureOpen = !captureOpen">从 shell 采集…</button></div>
      <section v-if="captureOpen" class="capture-confirm" aria-label="确认主动采集">
        <label>Shell <select v-model="shell" :disabled="busy" aria-label="采集 Shell"><option value="zsh">/bin/zsh</option><option value="bash">/bin/bash</option></select></label>
        <p>将执行交互式登录启动脚本，包括主题和补全配置；成功后替换当前编辑并保存 PATH。最多等待 5 秒，失败保留原配置。</p>
        <button :disabled="busy" @click="write(true)">{{ writing ? '正在采集…' : '执行采集并保存 PATH' }}</button>
      </section>
      <p v-if="message" class="settings-message" role="status">{{ message }}</p><p v-if="warning" class="notice">{{ warning }}</p>
      <p class="note">只影响后续启动，不修改正在运行的应用。手动保存会清除旧的采集来源记录。</p>
      <div class="settings-check"><h3>检查应用</h3><p class="muted">使用已保存的 PATH 检查声明与可执行文件，不运行命令。</p>
        <label class="settings-field">应用<select v-model="installationId" :disabled="busy" aria-label="检查应用"><option value="">选择应用</option><option v-for="app in apps" :key="app.installation_id" :value="app.installation_id">{{ app.label }}</option></select></label>
        <button :disabled="busy || dirty || !checkable || !!error" @click="check">{{ checking ? '正在检查…' : '检查运行环境' }}</button>
        <p v-if="dirty" class="muted">先保存 PATH，再检查。</p><p v-if="checkError" class="notice" role="status">{{ checkError }}</p>
        <template v-if="result"><p class="settings-message" role="status">检查通过，尚未启动应用。</p><dl v-for="service in result.services" :key="service.name" class="check-service"><dt>{{ service.name }}</dt><dd>{{ service.executable }}</dd><dt>工作目录</dt><dd>{{ service.cwd }}</dd></dl></template>
      </div>
    </template>
  </section>
</template>
