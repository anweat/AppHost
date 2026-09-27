<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { declarationDiff } from '../manifestDiff';
import { commandText } from '../diagnostics';
import { errorMessage } from '../model';
const props=defineProps<{installationId:string;source:'project'|'external';disabled:boolean}>();
const emit=defineEmits<{changed:[];busy:[value:boolean]}>();
interface Review {revision:number;base_sha256:string;target_source:string;target_sha256:string|null;launch_sha256:string;target_path:string;before_text:string;after_text:string;manifest:{services:{name:string;command:string[];cwd?:string;port?:number}[]}}
const review=ref<Review>(), busy=ref(false), message=ref(''), pending=ref<string>();
watch(() => busy.value || !!pending.value, value => emit('busy',value));
let timer:ReturnType<typeof setTimeout>|undefined, disposed=false;
onUnmounted(()=>{disposed=true;clearTimeout(timer);emit('busy',false);});
async function inspect() {
  busy.value=true; message.value='';
  try {review.value=await invoke<Review>('desktop_manifest_switch_review',{installationId:props.installationId,targetSource:props.source === 'project'?'external':'project'});}
  catch(e){message.value=errorMessage(e).message;} finally {busy.value=false;}
}
async function check() {
  if(!pending.value || disposed)return;
  try {const value=await invoke<{completed:boolean;error?:{code:string;message:string}}>('desktop_operation',{installationId:props.installationId,operationId:pending.value});
    if(disposed)return;
    if(value.completed){pending.value=undefined;busy.value=false;review.value=undefined;message.value=value.error?`${value.error.code} · ${value.error.message}`:'清单来源已切换；应用保持停止';emit('changed');}
    else timer=setTimeout(check,1000);
  }catch(e){if(!disposed){message.value=`${errorMessage(e).message}；结果未确认，不会重复提交。`;busy.value=false;if(errorMessage(e).code==='OPERATION_EXPIRED'){pending.value=undefined;review.value=undefined;}}}
}
async function save() {
  const r=review.value;if(!r || busy.value || props.disabled)return;
  busy.value=true;
  try {const value=await invoke<{completed:boolean;operation_id?:string}>('desktop_manifest_switch',{installationId:props.installationId,request:{revision:r.revision,base_sha256:r.base_sha256,target_source:r.target_source,target_sha256:r.target_sha256,confirmed_launch_sha256:r.launch_sha256}});
    if(disposed)return;
    if(value.operation_id){pending.value=value.operation_id;message.value='已受理，等待停止并切换';void check();}
    else if(value.completed){review.value=undefined;message.value='清单来源已切换，原文件已保留';emit('changed');}
  }catch(e){message.value=`${errorMessage(e).code} · ${errorMessage(e).message}`;review.value=undefined;}finally{if(!pending.value)busy.value=false;}
}
</script>
<template><section aria-label="清单来源"><p class="muted">当前使用{{ source === 'project'?'项目内':'AppHost 外置' }}清单 <button :disabled="disabled || busy || !!pending" @click="inspect">切换为{{ source === 'project'?'外置':'项目内' }}清单…</button></p>
  <div v-if="review" class="notice"><p>目标：{{ review.target_path }}</p><p>原文件保留。运行中的应用先停止；保存后不会自动启动。</p><pre>{{ declarationDiff(review.before_text,review.after_text) }}</pre><div v-for="service in review.manifest.services" :key="service.name"><strong>{{ service.name }}</strong><pre>{{ commandText(service.command) }}</pre><p>工作目录 {{ service.cwd || '.' }} · 端口 {{ service.port || '未声明' }}</p></div><details><summary>完整目标声明</summary><pre>{{ review.after_text }}</pre></details><div class="row-actions"><button :disabled="disabled || busy || !!pending" @click="save">保存并确认启动声明</button><button :disabled="busy || !!pending" @click="review=undefined">取消</button></div></div>
  <p v-if="message" class="notice" role="status">{{ message }}</p><button v-if="pending && !busy" @click="busy=true;check()">核对切换结果</button></section></template>
