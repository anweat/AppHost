<script setup lang="ts">
import AppIcon from './AppIcon.vue';
import type { Action } from '../useRuntime';
import { stateLabel, type AppEntry, type Snapshot } from '../model';
defineProps<{ app: AppEntry; snapshot: Snapshot; busy?: boolean }>();
defineEmits<{ close: []; action: [action: Action] }>();
</script>
<template>
  <aside class="drawer" aria-labelledby="detail-title" @keydown.esc.stop="$emit('close')">
    <div class="detail-heading"><h2 id="detail-title">{{ app.label }}</h2><button class="icon-btn" aria-label="关闭详情" @click="$emit('close')">×</button></div>
    <div class="detail-body">
      <p>{{ stateLabel(app, snapshot) }}</p>
      <div class="row-actions">
        <button class="primary" :disabled="!snapshot.live || busy || app.requires_confirmation || app.registration_state === 'SOURCE_MISSING' || ['STARTING','STOPPING','RUNNING'].includes(app.runtime?.status.state || '')" @click="$emit('action', app.runtime?.services.some(service => service.alive) ? 'restart' : 'start')"><AppIcon name="play"/>{{ app.runtime?.services.some(service => service.alive) ? '完整重启' : '启动' }}</button>
        <button :disabled="!snapshot.live || busy || !app.runtime || app.runtime.status.state === 'STOPPED'" @click="$emit('action', 'stop')"><AppIcon name="stop"/>停止</button>
        <button v-if="app.runtime?.status.state === 'RUNNING'" :disabled="!snapshot.live || busy || app.requires_confirmation" @click="$emit('action', 'restart')"><AppIcon name="refresh"/>重启</button>
      </div>
      <p v-if="app.registration_state === 'SOURCE_MISSING'" class="notice">源码目录不可用</p>
      <p v-else-if="app.requires_confirmation" class="notice">启动声明需要重新确认</p>
      <dl><dt>项目目录</dt><dd>{{ app.path }}</dd><dt>安装 ID</dt><dd>{{ app.installation_id }}</dd></dl>
      <template v-if="app.runtime">
        <p v-if="app.runtime.status.failure" class="notice">{{ app.runtime.status.failure }}</p>
        <h3>服务</h3>
        <ul class="service-list"><li v-for="service in app.runtime.services" :key="service.name">
          <span>{{ service.name }} <small v-if="service.port">:{{ service.port }}</small></span>
          <span>{{ !snapshot.live ? '历史 · ' : '' }}{{ service.alive ? service.ready ? '已就绪' : '尚未就绪' : '未运行' }}</span>
        </li></ul>
        <p class="muted">记录更新于 {{ new Date(app.runtime.updated_at_ms).toLocaleString() }}</p>
      </template>
    </div>
  </aside>
</template>
