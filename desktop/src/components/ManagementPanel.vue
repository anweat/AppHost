<script setup lang="ts">
import { reactive, watch } from 'vue';
import AppIcon from './AppIcon.vue';
import AppearanceSettings from './AppearanceSettings.vue';
import EnvironmentSettings from './EnvironmentSettings.vue';
import RetentionSettings from './RetentionSettings.vue';
import type { AppEntry } from '../model';
defineProps<{theme: string; saving: boolean; apps: AppEntry[]}>();
defineEmits<{theme: [value: string]; home: []}>();
const section = defineModel<string>('section', {default:'appearance'});
const installationId = defineModel<string>('installationId', {default:''});
const visited = reactive(new Set<string>());
// Navigation keeps edited buffers alive; opening management does not read or
// execute unrelated environment operations.
watch(section, value => visited.add(value), {immediate:true});
</script>
<template>
  <div class="management">
    <nav class="sidebar" aria-label="管理导航"><h2>管理工作区</h2>
      <button v-for="(label, key) in {appearance:'外观', environment:'运行环境', retention:'日志保留'}" :key="key" :class="{active:section === key}" :aria-current="section === key ? 'page' : undefined" @click="section = key"><AppIcon :name="{appearance:'moon',environment:'settings',retention:'info'}[key]"/>{{ label }}</button>
      <button @click="$emit('home')"><AppIcon name="home"/>返回应用</button>
    </nav>
    <div class="manage-content">
      <AppearanceSettings v-show="section === 'appearance'" :theme="theme" :saving="saving" @theme="$emit('theme', $event)"/>
      <EnvironmentSettings v-if="visited.has('environment')" v-show="section === 'environment'" v-model:installation-id="installationId" :apps="apps"/>
      <RetentionSettings v-if="visited.has('retention')" v-show="section === 'retention'"/>
    </div>
  </div>
</template>
