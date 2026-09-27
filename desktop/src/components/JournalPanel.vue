<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, type AppEntry } from '../model';
defineProps<{apps:AppEntry[]}>();
interface Event {event_id:string;at_ms:number;action:string;actor:string;installation_id?:string;outcome:string;code?:string}
const events = ref<Event[]>([]), cursor = ref<string>(), busy = ref(false), message = ref('');
const outcomes:Record<string,string> = {accepted:'已受理',succeeded:'已完成',failed:'失败',canceled:'已取消',observed:'已观察'};
const actions:Record<string,string> = {start:'启动',restart:'重启',stop:'停止','stop-all':'全部停止',detach:'解除登记',relink:'重新关联','daemon-start':'启动后台','manifest-save':'保存声明',import:'导入','stop-owner-and-start':'停止占用应用并启动'};
async function read(more = false) {
  if (busy.value) return; busy.value = true;
  try { const page = await invoke<{events:Event[];next_cursor?:string;has_more:boolean}>('desktop_journal', {cursor:more ? cursor.value : null}); events.value = more ? [...events.value,...page.events] : page.events; cursor.value = page.has_more ? page.next_cursor : undefined; message.value = ''; }
  catch (cause) { message.value = errorMessage(cause).message; }
  finally { busy.value = false; }
}
onMounted(() => read());
</script>
<template><section aria-label="操作记录"><div class="panel-toolbar"><h1>操作记录</h1><button :disabled="busy" @click="read()">刷新</button></div><p class="muted">AppHost 刚才做了什么</p>
  <p v-if="message" class="notice" role="status">{{ message }}</p><p v-if="!events.length && !busy">暂无记录</p>
  <ol class="journal-list"><li v-for="event in events" :key="event.event_id"><strong>{{ actions[event.action] || event.action }} · {{ outcomes[event.outcome] || event.outcome }}</strong><p>{{ apps.find(a => a.installation_id === event.installation_id)?.label || event.installation_id || 'AppHost' }}<span v-if="event.code"> · {{ event.code }}</span></p><small class="muted">{{ new Date(event.at_ms).toLocaleString() }} · {{ {human:'用户',agent:'Agent',system:'系统'}[event.actor] || event.actor }}</small></li></ol>
  <button v-if="cursor" :disabled="busy" @click="read(true)">后续记录</button></section></template>
