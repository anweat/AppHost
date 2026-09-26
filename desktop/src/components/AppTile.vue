<script setup lang="ts">
import { computed } from 'vue';
import AppIcon from './AppIcon.vue';
import { hasLiveServices, identityHue, stateLabel, tileState, type AppEntry, type Snapshot } from '../model';
const props = defineProps<{ app: AppEntry; snapshot: Snapshot }>();
defineEmits<{ details: [id: string]; primary: [id: string] }>();
const label = computed(() => [props.app.label, stateLabel(props.app, props.snapshot),
  props.app.registration_state === 'SOURCE_MISSING' ? '源码目录不可用' : props.app.requires_confirmation ? '启动声明待确认' : '',
  !props.snapshot.live ? '查看连接状态' : props.app.requires_confirmation || props.app.registration_state === 'SOURCE_MISSING' ? '查看详情' : ['STARTING', 'STOPPING'].includes(props.app.runtime?.status.state || '') ? '请等待当前操作' : props.app.runtime?.status.state === 'RUNNING' ? props.app.has_entry ? '打开应用' : '查看详情' : '启动应用',
].filter(Boolean).join('，'));
</script>
<template>
  <article class="tile" :style="{ '--hue': identityHue(app.project_id) }"
    :data-state="tileState(app)" :data-live="hasLiveServices(app, snapshot)"
    :data-ready="app.runtime?.services.every(service => service.ready)">
    <button class="tile-main" :aria-label="label"
      :title="label"
      @click="$emit('primary', app.installation_id)"
      @contextmenu.prevent="$emit('details', app.installation_id)"
      @keydown.shift.f10.prevent="$emit('details', app.installation_id)">
      <span class="badge" aria-hidden="true"><b class="mono">{{ [...app.label.trim()][0]?.toUpperCase() || '?' }}</b></span>
      <span class="tile-name">{{ app.label }}</span>
      <span class="tile-flag" aria-hidden="true"></span><span class="tile-dot" aria-hidden="true"></span>
      <span v-if="!['STARTING', 'STOPPING'].includes(app.runtime?.status.state || '')" class="tile-hint" aria-hidden="true"><AppIcon :name="!snapshot.live || app.requires_confirmation || app.registration_state === 'SOURCE_MISSING' ? 'info' : app.runtime?.status.state === 'RUNNING' ? app.has_entry ? 'arrow' : 'info' : 'play'"/></span>
    </button>
    <button class="tile-more" aria-label="应用详情与操作" @click="$emit('details', app.installation_id)">···</button>
  </article>
</template>
