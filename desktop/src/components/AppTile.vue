<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref }
from 'vue';
import { invoke } from '@tauri-apps/api/core';

import AppIcon from './AppIcon.vue';
import { hasLiveServices, identityHue, stateLabel, tileState, type AppEntry, type Snapshot } from '../model';
const props = defineProps<{ app: AppEntry; snapshot: Snapshot; feedback?:string }>();
defineEmits<{ details: [id: string]; primary: [id: string]; menu: [id:string,event:MouseEvent|KeyboardEvent]; dismiss:[id:string] }>();
const icon=ref(''); let disposed=false;
onUnmounted(()=>{disposed=true;});
onMounted(async()=>{try {const value=await invoke<{mime:string;bytes:number[]}|null>('desktop_icon',{installationId:props.app.installation_id});
  if(value && !disposed){let binary='';for(let i=0;i<value.bytes.length;i+=8192)binary+=String.fromCharCode(...value.bytes.slice(i,i+8192));icon.value=`data:${value.mime};base64,${btoa(binary)}`;}
}catch{/* Missing/changed icons keep the stable initial fallback. */}});
const label = computed(() => [props.app.label, stateLabel(props.app, props.snapshot),
  props.app.registration_state === 'SOURCE_MISSING' ? '源码目录不可用' : props.app.requires_confirmation ? '启动声明待确认' : '',
  !props.snapshot.live ? '启动应用并连接后台' : props.app.requires_confirmation || props.app.registration_state === 'SOURCE_MISSING' ? '查看详情' : ['STARTING', 'STOPPING'].includes(props.app.runtime?.status.state || '') ? '请等待当前操作' : props.app.runtime?.status.state === 'RUNNING' ? props.app.has_entry ? '打开应用' : '查看日志' : '启动应用',
].filter(Boolean).join('，'));
</script>
<template>
  <article draggable="true" @dragstart="$event.dataTransfer?.setData('text/apphost-installation',app.installation_id)" class="tile" :style="{ '--hue': identityHue(app.project_id) }"
    :data-state="tileState(app)" :data-live="hasLiveServices(app, snapshot)"
    :data-ready="app.runtime?.services.every(service => service.ready)">
    <button class="tile-main" :aria-label="label"
      :title="label"
      @click="$emit('primary', app.installation_id)"
      @contextmenu.prevent.stop="$emit('menu', app.installation_id, $event)"
      @keydown.shift.f10.prevent.stop="$emit('menu', app.installation_id, $event)">
      <svg v-if="app.runtime && app.runtime.services.length > 1" class="service-ring" viewBox="0 0 100 100" aria-hidden="true"><circle v-for="(service,index) in app.runtime.services" :key="service.name" cx="50" cy="50" r="46" fill="none" pathLength="100" :stroke-dasharray="`${Math.max(1,100/app.runtime.services.length-3)} ${100-Math.max(1,100/app.runtime.services.length-3)}`" :stroke-dashoffset="-index*100/app.runtime.services.length" :class="{ready:snapshot.live && service.alive && service.ready,waiting:snapshot.live && service.alive && !service.ready,failed:app.runtime.status.state==='FAILED' && !service.alive}"/></svg>
      <span class="badge" aria-hidden="true"><img v-if="icon" :src="icon" alt="" @error="icon=''"><b v-else class="mono">{{ [...app.label.trim()][0]?.toUpperCase() || '?' }}</b></span>
      <span class="tile-name">{{ app.label }}</span>
      <span class="tile-flag" aria-hidden="true"></span><span class="tile-dot" aria-hidden="true"></span>
      <span v-if="!['STARTING', 'STOPPING'].includes(app.runtime?.status.state || '')" class="tile-hint" aria-hidden="true"><AppIcon :name="!snapshot.live || app.requires_confirmation || app.registration_state === 'SOURCE_MISSING' ? 'info' : app.runtime?.status.state === 'RUNNING' ? app.has_entry ? 'arrow' : 'info' : 'play'"/></span>
    </button>
    <div v-if="feedback" class="tile-feedback" role="status">{{ feedback }}<button aria-label="关闭应用提示" @click="$emit('dismiss',app.installation_id)">×</button></div>
    <button class="tile-more" aria-label="应用详情与操作" @click="$emit('details', app.installation_id)">···</button>
  </article>
</template>
