<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import { useDetailRead } from '../useDetailRead';
import { commandText, failureEvidence, type FailureReport } from '../diagnostics';
import { errorMessage } from '../model';
import type { EditFocus } from '../manifestDiff';
import DiagnosticPreview from './DiagnosticPreview.vue';
const props = defineProps<{ installationId: string; canRetry: boolean; apps?: {installation_id:string;label:string}[] }>();
defineEmits<{ retry: []; edit: [focus: EditFocus]; environment: []; relink: []; owner: [failureId:string] }>();
const selectedFailure = ref<string>();
const { data, error, loading, refresh } = useDetailRead(() => invoke<FailureReport>('desktop_failure', { installationId:props.installationId, failureId:selectedFailure.value || null }));
const evidence = computed(() => data.value ? failureEvidence(data.value) : undefined);
const preview = ref(false), message = ref('');
const primary = computed(() => data.value?.advice?.actions.find(action => action.primary));
const failureId = computed(() => data.value?.failure?.failure_id || data.value?.history.last_failure?.failure_id);
async function copyCommand() {
  if (!primary.value) return;
  try { await invoke('desktop_copy', {text:commandText(primary.value.argv)}); message.value = '已复制建议命令，尚未执行'; }
  catch (cause) { message.value = errorMessage(cause).message; }
}
let token: string | undefined;
const condition = ref('');
function cancelCondition() { if (token) { void invoke('desktop_condition_unwatch', {token}); token = undefined; } }
async function watchCondition() {
  cancelCondition(); condition.value = '';
  if (!failureId.value || data.value?.advice?.code !== 'PORT_IN_USE' || selectedFailure.value) return;
  const current = crypto.randomUUID(); token = current;
  const channel = new Channel<{data:{state:string;failure_id:string}}>();
  channel.onmessage = event => { if (token === current && event.data.failure_id === failureId.value) condition.value = event.data.state; };
  try { await invoke('desktop_condition_watch', {installationId:props.installationId,failureId:failureId.value,token:current,channel}); }
  catch { if (token === current) condition.value = 'unavailable'; }
}
watch(data, watchCondition);
onUnmounted(cancelCondition);
async function selectFailure() { cancelCondition(); await refresh(); }
onMounted(refresh);
</script>
<template>
  <DiagnosticPreview v-if="preview && failureId" :installation-id="installationId" :failure-id="failureId" @close="preview = false"/>
  <section v-else aria-label="失败现场" :aria-busy="loading">
    <div class="panel-toolbar"><strong>失败现场</strong><button :disabled="loading" @click="refresh">刷新现场</button></div>
    <p v-if="loading" class="muted">正在读取…</p>
    <p v-if="error" class="notice" role="status">{{ error.code === 'NOT_FOUND' ? '还没有保存的失败现场。' : `${error.code} · ${error.message}` }}</p>
    <template v-if="data">
      <label v-if="data.records?.length">失败记录 <select v-model="selectedFailure" :disabled="loading" @change="selectFailure"><option :value="undefined">最近一次</option><option v-for="record in data.records" :key="record.failure_id" :value="record.failure_id">{{ new Date(record.at_ms).toLocaleString() }} · {{ record.code }}</option></select></label>
      <p v-if="selectedFailure" class="notice">正在查看历史现场；重新启动与停止占用者操作仅适用于最近一次失败。</p>
      <p v-if="data.details_expired" class="notice">失败详情已超过留存范围，仅保留历史摘要。</p>
      <template v-if="data.failure"><p class="muted">{{ new Date(data.failure.at_ms).toLocaleString() }} <span v-if="data.failure.service">· {{ data.failure.service }}</span></p>
        <p v-if="data.failure.collecting" class="notice">仍在采集输出尾部，可稍后刷新。</p>
        <p v-for="gap in data.failure.gaps" :key="gap" class="notice">现场缺口：{{ gap }}</p>
      </template>
      <template v-if="data.advice">
        <h3 class="failure-title">{{ data.advice.title }}</h3><p>{{ data.advice.impact }}</p>
        <p class="muted">{{ data.advice.code }} · 同类连续 {{ data.advice.consecutive_same_cause }} 次</p>
        <div v-if="primary" class="failure-action">
          <button v-if="primary.id === 'retry'" class="primary" :disabled="!canRetry || !!selectedFailure" @click="$emit('retry')">{{ primary.title }}</button>
          <button v-else-if="primary.id === 'stop_owner_and_start'" class="primary" :disabled="!canRetry || !failureId || !!selectedFailure" @click="$emit('owner', failureId!)">{{ primary.title }}</button>
          <button v-else-if="primary.id === 'relink'" class="primary" @click="$emit('relink')">{{ primary.title }}</button>
          <button v-else-if="primary.id === 'preview_diagnostics'" class="primary" :disabled="!failureId" @click="preview = true">{{ primary.title }}</button>
          <button v-else-if="['edit_manifest','review_manifest'].includes(primary.id)" class="primary" @click="$emit('edit', {service:data.failure?.service, field:['PORT_IN_USE','READINESS_TIMEOUT','PORT_NOT_READY'].includes(data.advice.code) ? 'port' : 'command'})">{{ primary.title }}</button>
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
      <section v-if="evidence?.owner?.port || evidence?.owner?.installation_id || evidence?.owner?.ownership" aria-label="端口占用证据"><h3>端口占用者 <span v-if="evidence.owner.port">:{{ evidence.owner.port }}</span></h3>
        <p v-if="evidence.owner.installation_id">受管应用：{{ apps?.find(a => a.installation_id === evidence?.owner?.installation_id)?.label || evidence.owner.installation_id }}</p>
        <p v-for="owner in evidence.owner.ownership?.owners" :key="owner.pid">{{ owner.name || '进程名称不可用' }} · PID {{ owner.pid }}</p>
        <p v-if="evidence.owner.ownership?.incomplete" class="muted">占用者查询不完整，可能受权限或查询期限限制。</p>
      </section>
      <section v-if="evidence?.listeners.length" aria-label="实际监听端口"><h3>实际监听端口</h3><p v-for="service in evidence.listeners" :key="service.name">{{ service.name }}：{{ service.observed_listeners?.ports?.join(', ') || '未观察到监听' }}<span v-if="service.observed_listeners?.incomplete">（观察不完整）</span></p></section>
      <p v-if="condition === 'cleared'" class="notice" role="status">端口已释放，可以重试。<button :disabled="!canRetry" @click="$emit('retry')">重新启动</button></p>
      <p v-else-if="condition === 'blocked'" class="muted">端口仍被占用，正在观察（最多 60 秒）。</p>
      <p v-else-if="['expired','unavailable','unknown'].includes(condition)" class="muted">当前未继续观察端口，刷新现场可重新检查。</p>
      <section v-if="data.changes_since_success" aria-label="自上次成功以来的变化"><h3>自上次成功以来的变化</h3><template v-if="data.changes_since_success.available"><p v-if="!data.changes_since_success.changes?.length" class="muted">未发现已记录配置的变化</p><p v-for="change in data.changes_since_success.changes" :key="change.field">{{ {branch:'Git 分支',launch_sha256:'启动声明',manifest_source:'清单来源',environment_sha256:'运行环境'}[change.field] || change.field }}已变化<span v-if="['branch','manifest_source'].includes(change.field)">：{{ change.before || '未知' }} → {{ change.after || '未知' }}</span></p></template><p v-else class="muted">{{ data.changes_since_success.reason === 'NO_SUCCESS_BASELINE' ? '尚无成功运行基线' : '当前无法读取变化证据' }}</p></section>
      <details v-if="data.failure?.stderr_tail"><summary>错误输出末尾</summary><pre>{{ data.failure.stderr_tail }}</pre></details>
      <p v-if="data.history.last_success_at_ms" class="muted">上次成功 {{ new Date(data.history.last_success_at_ms).toLocaleString() }}</p>
      <button v-if="failureId && primary?.id !== 'preview_diagnostics'" @click="preview = true">预览诊断信息</button>
    </template>
    <p class="muted" role="status">{{ message }}</p>
  </section>
</template>
