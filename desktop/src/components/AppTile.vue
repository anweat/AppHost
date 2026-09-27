<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref }
from 'vue';
import { invoke } from '@tauri-apps/api/core';

import { feedbackPrimary, type Feedback } from '../useFeedback';
import AppIcon from './AppIcon.vue';
import { runtimeBlocked, hasLiveServices, identityHue, stateLabel, tileState, type AppEntry, type Snapshot } from '../model';
const props = defineProps<{ app: AppEntry; snapshot: Snapshot; feedback?:Feedback }>();
defineEmits<{ details: [id: string]; primary: [id: string]; menu: [id:string,event:MouseEvent|KeyboardEvent]; dismiss:[id:string]; feedbackAction:[id:string,item:Feedback] }>();
const icon=ref(''); let disposed=false;
// Follow the prototype's rounded-square outline, independent of the icon content.
const serviceRing = computed(() => {
  const runtime=props.app.runtime;
  if(!props.snapshot.live || !runtime || runtime.services.length<2 || runtime.status.state==='STOPPED') return '';
  const step=360/runtime.services.length, gap=Math.min(12,step/3);
  return `conic-gradient(${runtime.services.map((service,index)=>{
    const color=service.alive ? service.ready ? 'var(--seg-on)' : 'var(--seg-wait)' : runtime.status.state==='FAILED' ? 'var(--fail)' : 'var(--seg-off)';
    return `transparent ${index*step}deg ${index*step+gap/2}deg,${color} ${index*step+gap/2}deg ${(index+1)*step-gap/2}deg,transparent ${(index+1)*step-gap/2}deg ${(index+1)*step}deg`;
  }).join(',')})`;
});
onUnmounted(()=>{disposed=true;});
onMounted(async()=>{try {const value=await invoke<{mime:string;bytes:number[]}|null>('desktop_icon',{installationId:props.app.installation_id});
  if(value && !disposed){let binary='';for(let i=0;i<value.bytes.length;i+=8192)binary+=String.fromCharCode(...value.bytes.slice(i,i+8192));icon.value=`data:${value.mime};base64,${btoa(binary)}`;}
}catch{/* Missing/changed icons keep the stable initial fallback. */}});
const label = computed(() => [props.app.label, stateLabel(props.app, props.snapshot),
  props.app.registration_state === 'SOURCE_MISSING' ? '源码目录不可用' : props.app.requires_confirmation ? '启动声明待确认' : '',
  runtimeBlocked(props.snapshot) ? '连接异常，查看历史详情' : !props.snapshot.live ? '启动应用并连接后台' : props.app.requires_confirmation || props.app.registration_state === 'SOURCE_MISSING' ? '查看详情' : ['STARTING', 'STOPPING'].includes(props.app.runtime?.status.state || '') ? '请等待当前操作' : props.app.runtime?.status.state === 'RUNNING' ? props.app.has_entry ? '打开应用' : '查看日志' : '启动应用',
].filter(Boolean).join('，'));
</script>
<template>
  <article draggable="true" @dragstart="$event.dataTransfer?.setData('text/apphost-installation',app.installation_id)" class="tile" :style="{ '--hue': identityHue(app.project_id) }"
    :data-state="tileState(app)" :data-live="hasLiveServices(app, snapshot)"
    :data-ready="app.runtime?.services.every(service => service.ready)">
    <button class="tile-main" :aria-label="label"
      :title="label" :aria-describedby="feedback ? `feedback-${app.installation_id}-${$.uid}` : undefined"
      @click="$emit('primary', app.installation_id)"
      @contextmenu.prevent.stop="$emit('menu', app.installation_id, $event)"
      @keydown.shift.f10.prevent.stop="$emit('menu', app.installation_id, $event)">
      <span class="badge" aria-hidden="true"><img v-if="icon" :src="icon" alt="" @error="icon=''"><b v-else class="mono">{{ [...app.label.trim()][0]?.toUpperCase() || '?' }}</b><i v-if="serviceRing" class="service-ring" :style="{background:serviceRing}"></i></span>
      <span class="tile-name">{{ app.label }}</span>
      <span class="tile-flag" aria-hidden="true"></span><span class="tile-dot" aria-hidden="true"></span>
      <span v-if="!['STARTING', 'STOPPING'].includes(app.runtime?.status.state || '')" class="tile-hint" aria-hidden="true"><AppIcon :name="!snapshot.live || app.requires_confirmation || app.registration_state === 'SOURCE_MISSING' ? 'info' : app.runtime?.status.state === 'RUNNING' ? app.has_entry ? 'arrow' : 'info' : 'play'"/></span>
    </button>
    <div v-if="feedback" :id="`feedback-${app.installation_id}-${$.uid}`" class="tile-feedback" role="status" @keydown.esc.stop="$emit('dismiss',app.installation_id); ($el.querySelector('.tile-main') as HTMLElement)?.focus()">
      <button class="bubble-close" aria-label="关闭应用提示" @click="$emit('dismiss',app.installation_id)">×</button>
      <strong>{{ feedback.cleared ? '端口已释放，可以重试' : feedback.text }}</strong>
      <template v-if="feedback.persistent"><p v-if="feedback.error" class="muted">{{ feedback.error.code }}</p><p v-if="feedback.report?.advice?.impact || feedback.impact">{{ feedback.report?.advice?.impact || feedback.impact }}</p><p v-if="(feedback.report?.advice?.consecutive_same_cause || 0) > 1">与上次原因相同</p>
        <button v-if="feedback.cleared || feedbackPrimary(feedback)" :disabled="(runtimeBlocked(snapshot) || app.requires_confirmation || app.registration_state==='SOURCE_MISSING') && (feedback.cleared || ['retry','stop_owner_and_start'].includes(feedbackPrimary(feedback)?.id || ''))" @click="$emit('feedbackAction',app.installation_id,feedback)">{{ feedback.cleared ? '重新启动' : feedbackPrimary(feedback)?.title }}</button>
        <button v-if="feedback.report?.failure" @click="$emit('details',app.installation_id)">详情</button>
      </template>
    </div>
    <button class="tile-more" aria-label="应用详情与操作" @click="$emit('details', app.installation_id)">···</button>
  </article>
</template>
