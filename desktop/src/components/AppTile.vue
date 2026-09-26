<script setup lang="ts">
import { computed } from 'vue';
import { hasLiveServices, identityHue, stateLabel, tileState, type AppEntry, type Snapshot } from '../model';
const props = defineProps<{ app: AppEntry; snapshot: Snapshot }>();
defineEmits<{ details: [id: string] }>();
const label = computed(() => [props.app.label, stateLabel(props.app, props.snapshot),
  props.app.registration_state === 'SOURCE_MISSING' ? '源码目录不可用' : props.app.requires_confirmation ? '启动声明待确认' : '',
  '查看详情',
].filter(Boolean).join('，'));
</script>
<template>
  <article class="tile" :style="{ '--hue': identityHue(app.project_id) }"
    :data-state="tileState(app)" :data-live="hasLiveServices(app, snapshot)"
    :data-ready="app.runtime?.services.every(service => service.ready)">
    <button class="tile-main" :aria-label="label"
      :title="label"
      @click="$emit('details', app.installation_id)"
      @contextmenu.prevent="$emit('details', app.installation_id)"
      @keydown.shift.f10.prevent="$emit('details', app.installation_id)">
      <span class="badge" aria-hidden="true"><b class="mono">{{ [...app.label.trim()][0]?.toUpperCase() || '?' }}</b></span>
      <span class="tile-name">{{ app.label }}</span>
      <span class="tile-flag" aria-hidden="true"></span><span class="tile-dot" aria-hidden="true"></span>
      <span class="tile-hint" aria-hidden="true">↗</span>
    </button>
  </article>
</template>
