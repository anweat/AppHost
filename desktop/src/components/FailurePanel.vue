<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useDetailRead } from '../useDetailRead';
import { commandText, type FailureReport } from '../diagnostics';
import { errorMessage } from '../model';
import DiagnosticPreview from './DiagnosticPreview.vue';
const props = defineProps<{ installationId: string; canRetry: boolean }>();
defineEmits<{ retry: []; edit: []; environment: [] }>();
const { data, error, loading, refresh } = useDetailRead(() => invoke<FailureReport>('desktop_failure', { installationId:props.installationId }));
const preview = ref(false), message = ref('');
const primary = computed(() => data.value?.advice?.actions.find(action => action.primary));
const failureId = computed(() => data.value?.failure?.failure_id || data.value?.history.last_failure?.failure_id);
async function copyCommand() {
  if (!primary.value) return;
  try { await invoke('desktop_copy', {text:commandText(primary.value.argv)}); message.value = '已复制建议命令，尚未执行'; }
  catch (cause) { message.value = errorMessage(cause).message; }
}
onMounted(refresh);
</script>
<template>
  <DiagnosticPreview v-if="preview && failureId" :installation-id="installationId" :failure-id="failureId" @close="preview = false"/>
  <section v-else aria-label="失败现场" :aria-busy="loading">
    <div class="panel-toolbar"><strong>失败现场</strong><button :disabled="loading" @click="refresh">刷新现场</button></div>
    <p v-if="loading" class="muted">正在读取…</p>
    <p v-if="error" class="notice" role="status">{{ error.code === 'NOT_FOUND' ? '还没有保存的失败现场。' : `${error.code} · ${error.message}` }}</p>
    <template v-if="data">
      <p v-if="data.details_expired" class="notice">失败详情已超过留存范围，仅保留历史摘要。</p>
      <template v-if="data.failure"><p class="muted">{{ new Date(data.failure.at_ms).toLocaleString() }} <span v-if="data.failure.service">· {{ data.failure.service }}</span></p>
        <p v-if="data.failure.collecting" class="notice">仍在采集输出尾部，可稍后刷新。</p>
        <p v-for="gap in data.failure.gaps" :key="gap" class="notice">现场缺口：{{ gap }}</p>
      </template>
      <template v-if="data.advice">
        <h3 class="failure-title">{{ data.advice.title }}</h3><p>{{ data.advice.impact }}</p>
        <p class="muted">{{ data.advice.code }} · 同类连续 {{ data.advice.consecutive_same_cause }} 次</p>
        <div v-if="primary" class="failure-action">
          <button v-if="primary.id === 'retry'" class="primary" :disabled="!canRetry" @click="$emit('retry')">{{ primary.title }}</button>
          <button v-else-if="primary.id === 'preview_diagnostics'" class="primary" :disabled="!failureId" @click="preview = true">{{ primary.title }}</button>
          <button v-else-if="['edit_manifest','review_manifest'].includes(primary.id)" class="primary" @click="$emit('edit')">{{ primary.title }}</button>
          <button v-else-if="primary.id === 'view_environment'" class="primary" @click="$emit('environment')">{{ primary.title }}</button>
          <button v-else-if="primary.copy_only" class="primary" @click="copyCommand">{{ primary.title }}</button>
          <a v-else-if="primary.id === 'view_reason'" class="primary evidence-link" href="#failure-evidence">{{ primary.title }}</a>
          <template v-else><strong>{{ primary.title }}</strong><p class="muted">此操作请在命令行中完成。</p></template>
          <template v-if="primary.argv.length && primary.id !== 'retry' && primary.id !== 'preview_diagnostics' && primary.id !== 'view_reason'">
            <pre class="command-preview">{{ commandText(primary.argv) }}</pre><p v-if="primary.cwd" class="muted">服务工作目录：{{ primary.cwd }}</p>
            <button v-if="!primary.copy_only" @click="copyCommand">复制建议命令</button>
          </template>
        </div>
        <div id="failure-evidence" tabindex="-1"><h3>判断依据</h3><p class="muted">{{ data.advice.evidence.inferred ? '依据输出推断' : '运行检查结果' }} · {{ data.advice.evidence.rule_id }}<span v-if="data.advice.evidence.line"> · 第 {{ data.advice.evidence.line }} 行</span></p>
          <pre v-if="data.advice.evidence.excerpt">{{ data.advice.evidence.excerpt }}</pre>
          <p v-for="hint in data.advice.hints" :key="hint.code" class="notice">{{ hint.title }}</p>
        </div>
      </template>
      <details v-if="data.failure?.stderr_tail"><summary>错误输出末尾</summary><pre>{{ data.failure.stderr_tail }}</pre></details>
      <p v-if="data.history.last_success_at_ms" class="muted">上次成功 {{ new Date(data.history.last_success_at_ms).toLocaleString() }}</p>
      <button v-if="failureId && primary?.id !== 'preview_diagnostics'" @click="preview = true">预览诊断信息</button>
    </template>
    <p class="muted" role="status">{{ message }}</p>
  </section>
</template>
