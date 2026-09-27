<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { commandText } from '../diagnostics';
import type { Draft, ScanResult } from '../registration';
import { errorMessage } from '../model';
const emit = defineEmits<{close: []; changed: []; busy: [value: boolean]}>();
const token = ref(''), editable = ref<boolean[]>([]);
const editing = ref<{index: number; text: string} | null>(null);
const draft = ref<Draft | null>(null), busy = ref(false), message = ref(''), finished = ref(false);
watch(busy, value => emit('busy', value));
const count = computed(() => draft.value?.candidates.filter(c => c.selected).length || 0);
async function scan() {
  if (busy.value) return;
  busy.value = true; message.value = '';
  try {
    const result = await invoke<ScanResult | null>('desktop_scan');
    if (result) { result.draft.candidates.forEach(c => c.selected = false); token.value = result.token; editable.value = result.editable || []; draft.value = result.draft; finished.value = false; }
    else if (!draft.value) emit('close');
  } catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; }
  finally { busy.value = false; }
}
async function group(index: number, merge: boolean) {
  if (!draft.value || busy.value) return;
  busy.value = true; message.value = '';
  try { const result = await invoke<ScanResult>('desktop_group', {token:token.value,index,merge}); token.value = result.token; editable.value = result.editable || []; result.draft.candidates.forEach(c => c.selected = false); draft.value = result.draft; }
  catch (error) { message.value = errorMessage(error).message; }
  finally { busy.value = false; }
}
async function storage(index:number, external:boolean) {
  if (busy.value || finished.value) return;
  busy.value=true; message.value='';
  try { const result=await invoke<ScanResult>('desktop_candidate_external',{token:token.value,index,external}); result.draft.candidates.forEach(c => c.selected=false); token.value=result.token; editable.value=result.editable; draft.value=result.draft; }
  catch(e) {message.value=errorMessage(e).message;} finally {busy.value=false;}
}
async function edit(index: number) {
  if (busy.value || finished.value || editing.value) return;
  busy.value = true; message.value = '';
  try { editing.value = {index,text:await invoke<string>('desktop_candidate_read',{token:token.value,index})}; }
  catch (error) { message.value = `${errorMessage(error).code} · ${errorMessage(error).message}`; }
  finally { busy.value = false; }
}
async function reviewCandidate() {
  if (!editing.value || busy.value) return;
  busy.value = true; message.value = '';
  try {
    const result = await invoke<ScanResult>('desktop_candidate_review',{token:token.value,index:editing.value.index,text:editing.value.text});
    result.draft.candidates.forEach(c => c.selected = false);
    token.value = result.token; editable.value = result.editable; draft.value = result.draft; editing.value = null;
    message.value = '声明已校验，请勾选应用并确认登记。';
  } catch (error) {
    const issues = (error as {details?: {field: string; message: string}[]}).details;
    message.value = `${errorMessage(error).code} · ${errorMessage(error).message}` + (Array.isArray(issues) ? issues.map(i => `\n${i.field}：${i.message}`).join('') : '');
  } finally { busy.value = false; }
}
async function save() {
  if (!draft.value || busy.value || !count.value || draft.value.groups.length || finished.value || editing.value) return;
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
  <aside @keydown.esc.stop="!busy && !editing && emit('close')" class="drawer import-panel" aria-labelledby="import-title" :aria-busy="busy">
    <div class="detail-heading"><h2 id="import-title">添加应用</h2><button class="icon-btn" aria-label="关闭添加应用" :disabled="busy || !!editing" @click="emit('close')">×</button></div>
    <div class="detail-body">
      <div class="panel-toolbar"><span>{{ busy ? '正在处理…' : '选择并确认启动声明' }}</span><button :disabled="busy || !!editing" @click="scan">选择目录</button></div>
      <p v-if="message" class="notice path-text" role="status">{{ message }}</p>
      <template v-if="draft">
        <p class="muted path-text">{{ draft.root }}</p>
        <p v-for="(issue,i) in draft.issues" :key="i" class="notice">{{ issue.message }}</p>
        <section v-if="editing" aria-label="补全启动声明">
          <h3>{{ draft.candidates[editing.index]?.human_label }} · 启动声明</h3>
          <p class="muted">填写启动命令参数数组；工作目录相对项目根目录。多个服务可以增加 [[services]]，有多个端口时通过 [open] 指定浏览器入口。</p>
          <label class="editor-label">启动声明 TOML<textarea v-model="editing.text" class="manifest-editor" :readonly="busy" spellcheck="false" aria-label="导入启动声明"/></label>
          <details><summary>填写示例</summary><pre># 替换模板里的 command，程序和参数分别填写
command = ["uv", "run", "uvicorn", "backend.main:app", "--port", "8000"]
cwd = "."
port = 8000

# 多服务时，在文件末尾指定浏览器入口
[open]
service = "web"
path = "/"</pre><p class="muted">示例仅说明写法，请使用项目自己的启动方式。这里只校验草稿，不执行命令或写文件。</p></details>
          <div class="row-actions"><button class="primary" :disabled="busy" @click="reviewCandidate">校验并应用到草稿</button><button :disabled="busy" @click="editing = null; message = ''">放弃编辑</button></div>
        </section>
        <fieldset v-else :disabled="busy || finished" class="import-choices">
          <section v-for="(suggestion,i) in draft.groups" :key="suggestion.candidate.path" class="import-candidate">
            <strong>是否合为一个应用？</strong><p class="path-text">{{ suggestion.members.join('\n') }}</p>
            <button @click="group(i,true)">合并</button><button @click="group(i,false)">分别登记</button>
          </section>
          <section v-for="(candidate, index) in draft.candidates" :key="candidate.path" class="import-candidate">
            <label><input v-model="candidate.selected" type="checkbox" :disabled="!!draft.groups.length || !candidate.manifest?.services.length || !!candidate.issues.length">{{ candidate.human_label }}</label>
            <p class="muted path-text">{{ candidate.path }}</p>
            <p v-for="(issue,i) in candidate.issues" :key="i" class="notice">{{ issue.message }}</p>
            <template v-if="candidate.selected">
              <div v-for="service in candidate.manifest?.services" :key="service.name"><strong>{{ service.name }}</strong><pre>{{ commandText(service.command || []) }}</pre><p class="muted">工作目录：{{ service.cwd || '.' }}<span v-if="service.port"> · 端口 {{ service.port }}</span></p></div>
              <details><summary>完整清单与入口</summary><pre class="declaration-preview">{{ JSON.stringify(candidate.manifest, null, 2) }}</pre></details>
              <p v-for="(note,i) in candidate.notes" :key="i" class="notice">{{ note.message }}</p>
            </template>
            <label v-if="editable[index]">清单位置 <select :value="candidate.external ? 'external' : 'project'" :disabled="!!draft.groups.length" @change="storage(index, ($event.target as HTMLSelectElement).value === 'external')"><option value="project">项目内 apphost.toml</option><option value="external">AppHost 外置清单</option></select></label>
            <p v-if="candidate.external" class="muted">声明保存在 AppHost 数据目录，不写入项目。</p>
            <button v-if="editable[index]" :disabled="!!draft.groups.length" @click="edit(index)">{{ !candidate.manifest?.services.length || candidate.issues.length ? '补全启动声明' : '编辑启动声明' }}</button>
            <p v-else-if="!candidate.manifest?.services.length || candidate.issues.length" class="muted">已有清单存在问题，请修正项目清单后重新扫描；导入不会覆盖已有清单。</p>
          </section>
        </fieldset>
        <p v-if="!draft.candidates.length" class="muted">未发现应用，可选择具体项目目录重试。</p>
        <p class="muted">确认后登记所选应用；缺少清单时按所选位置创建声明，不会启动服务。</p>
        <button class="primary" :disabled="busy || finished || !!editing || !count || !!draft.groups.length" @click="save">确认并登记{{ count ? ` ${count} 个应用` : '' }}</button>
      </template>
    </div>
  </aside>
</template>
