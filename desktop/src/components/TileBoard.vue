<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import AppTile from './AppTile.vue';
import type { Preferences } from '../preferences';
import type { SearchAction } from '../search';
import { runtimeBlocked, type AppEntry, type Snapshot } from '../model';
import type { Feedback } from '../useFeedback';
const props = defineProps<{ apps: AppEntry[]; snapshot: Snapshot; lens: string | null; preferences:Preferences; feedback:Map<string,Feedback> }>();
const emit = defineEmits<{ details: [id: string]; primary: [id: string]; preference:[value:unknown]; batch:[kind:'start'|'stop',ids:string[]]; action:[action:SearchAction,id:string]; manage:[]; dismiss:[id:string]; feedbackAction:[id:string,item:Feedback] }>();
const groups = computed(() => {
  const result = new Map<string, { name: string; apps: AppEntry[] }>();
  for (const app of props.apps) {
    const custom=props.preferences.groups.find(g=>g.members.includes(app.installation_id));
    const key=custom?.id || app.group || 'apps';
    if(!result.has(key))result.set(key,{name:custom?.name || app.group_label || '应用',apps:[]});
    result.get(key)!.apps.push(app);
  }
  for(const group of props.preferences.groups) if(!result.has(group.id)) result.set(group.id,{name:group.name,apps:[]});
  return [...result].map(([key,value])=>({key,...value}));
});
const recent = computed(() => !props.preferences.show_recent || props.snapshot.apps.length<12 ? [] : props.apps.filter(a=>props.preferences.last_used[a.installation_id]).slice().sort((a,b)=>(props.preferences.last_used[b.installation_id]||0)-(props.preferences.last_used[a.installation_id]||0)).slice(0,6));
const menuElement=ref<HTMLElement>();
let menuOrigin:HTMLElement|undefined;
const menu = ref<{id:string;x:number;y:number}>();
function context(id:string, event:MouseEvent|KeyboardEvent) {
  menuOrigin=event.currentTarget as HTMLElement;
  const rect=menuOrigin.getBoundingClientRect();
  menu.value={id,x:Math.min(event instanceof MouseEvent ? event.clientX:rect.left,window.innerWidth-230),y:Math.min(event instanceof MouseEvent?event.clientY:rect.top+20,window.innerHeight-360)};
  void nextTick(()=>menuElement.value?.querySelector<HTMLButtonElement>('button')?.focus());
}
function closeMenu(){menu.value=undefined;}
function escapeMenu(){closeMenu();menuOrigin?.focus();}
function menuKeys(event:KeyboardEvent){
  if(!['ArrowUp','ArrowDown'].includes(event.key) || !(event.target instanceof HTMLButtonElement))return;
  const buttons=[...menuElement.value?.querySelectorAll<HTMLButtonElement>('button')||[]]; const index=buttons.indexOf(event.target);
  event.preventDefault();buttons[(index+(event.key==='ArrowDown'?1:buttons.length-1))%buttons.length]?.focus();
}
function command(action:SearchAction){if(menu.value)emit('action',action,menu.value.id);closeMenu();}
function move(group:string) {if(menu.value)emit('preference',{kind:'move',installation_id:menu.value.id,group_id:group||null});closeMenu();}
function drop(event:DragEvent,key:string) {
  const id=event.dataTransfer?.getData('text/apphost-installation');
  if(id && props.apps.some(a=>a.installation_id===id) && props.preferences.groups.some(g=>g.id===key))emit('preference',{kind:'move',installation_id:id,group_id:key});
}
function navigate(event:KeyboardEvent) {
  if(!['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(event.key) || !(event.target as HTMLElement).matches('.tile-main'))return;
  const target=event.target as HTMLElement, rect=target.getBoundingClientRect();
  const horizontal=['ArrowLeft','ArrowRight'].includes(event.key), sign=['ArrowLeft','ArrowUp'].includes(event.key)?-1:1;
  let best:HTMLElement|undefined, score=Infinity;
  for(const node of board.value?.querySelectorAll<HTMLElement>('.tile-main')||[]) {
    if(node===target)continue; const r=node.getBoundingClientRect();
    const dx=r.left+r.width/2-rect.left-rect.width/2,dy=r.top+r.height/2-rect.top-rect.height/2;
    const forward=(horizontal?dx:dy)*sign, lateral=Math.abs(horizontal?dy:dx);
    if(forward>1 && forward+lateral*4<score){score=forward+lateral*4;best=node;}
  }
  if(best){event.preventDefault();best.focus();best.scrollIntoView({block:'nearest',inline:'nearest'});}
}
const board = ref<HTMLElement>();
let observer: ResizeObserver;
function layout() {
  const element = board.value; if (!element) return;
  const css = getComputedStyle(element), gap = 8;
  const free = element.clientHeight - parseFloat(css.paddingTop) - parseFloat(css.paddingBottom) - 38;
  const narrow = window.innerWidth < 700, min = narrow ? 96 : 104, max = narrow ? 120 : 148;
  const rows = Math.max(1, Math.min(3, Math.floor((free + gap) / (min + gap))));
  element.style.setProperty('--rows', String(rows));
  element.style.setProperty('--tile', `${Math.max(min, Math.min(max, Math.floor((free - (rows - 1) * gap) / rows)))}px`);
}
function scroll(event: WheelEvent) {
  if (!board.value || event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return;
  if (board.value.scrollWidth <= board.value.clientWidth) return;
  event.preventDefault(); board.value.scrollLeft += event.deltaY;
}
onMounted(() => { window.addEventListener('click',closeMenu); observer = new ResizeObserver(layout); if (board.value) observer.observe(board.value); });
onUnmounted(() => {observer?.disconnect();window.removeEventListener('click',closeMenu);});
</script>
<template>
  <div ref="board" class="board" :data-offline="!snapshot.live" :data-lens="lens" data-at-start data-at-end @wheel="scroll" @keydown="navigate">
    <div class="canvas">
      <section v-if="recent.length" class="group recent" aria-label="最近使用"><h2 class="group-title">最近使用 <button class="group-control" @click="$emit('preference',{kind:'recent',enabled:false})" aria-label="关闭最近使用组">×</button></h2><div class="group-grid"><AppTile v-for="app in recent" :key="app.installation_id" :app="app" :snapshot="snapshot" :feedback="feedback.get(app.installation_id)" @dismiss="$emit('dismiss',$event)" @feedback-action="(id,item)=>$emit('feedbackAction',id,item)" @details="$emit('details',$event)" @primary="$emit('primary',$event)" @menu="context"/></div></section>
      <section v-for="group in groups" :key="group.key" class="group" :aria-label="group.name" @dragover.prevent @drop.prevent="drop($event,group.key)">
        <h2 class="group-title" :title="group.key">{{ group.name }} <span class="group-tools"><button class="group-control" aria-label="折叠或展开分组" @click="$emit('preference',{kind:'collapse',key:group.key,collapsed:!preferences.collapsed.includes(group.key)})">{{ preferences.collapsed.includes(group.key)?'▸':'▾' }}</button><button class="group-control" aria-label="启动分组" :disabled="runtimeBlocked(snapshot)" @click="$emit('batch','start',group.apps.map(a=>a.installation_id))">▷</button><button class="group-control" aria-label="停止分组" :disabled="runtimeBlocked(snapshot)" @click="$emit('batch','stop',group.apps.map(a=>a.installation_id))">□</button></span></h2>
        <button v-if="preferences.collapsed.includes(group.key)" class="folder-tile" :aria-label="`展开 ${group.name}，${group.apps.length} 个应用`" @click="$emit('preference',{kind:'collapse',key:group.key,collapsed:false})"><span class="folder-grid"><i v-for="app in group.apps.slice(0,9)" :key="app.installation_id" :data-state="snapshot.live ? app.runtime?.status.state : undefined" :title="app.label"/></span><span>{{ group.name }}</span></button>
        <div v-else class="group-grid"><AppTile v-for="app in group.apps" :key="app.installation_id" :app="app" :snapshot="snapshot" :feedback="feedback.get(app.installation_id)" @dismiss="$emit('dismiss',$event)" @feedback-action="(id,item)=>$emit('feedbackAction',id,item)" @details="$emit('details', $event)" @primary="$emit('primary', $event)" @menu="context"/></div>
      </section>
      <button class="group-add" @click="$emit('manage')">管理分组</button>
    </div>
    <Teleport to="body"><div v-if="menu" ref="menuElement" class="tile-menu" role="menu" :style="{left:Math.max(8,menu.x)+'px',top:Math.max(8,menu.y)+'px'}" @click.stop @keydown="menuKeys" @keydown.esc.stop="escapeMenu">
      <button v-for="(title,action) in {primary:'启动 / 打开',stop:'停止',restart:'重启',details:'详情',logs:'日志',manifest:'编辑声明'}" :key="action" :disabled="runtimeBlocked(snapshot) && ['stop','restart'].includes(action)" role="menuitem" @click="command(action as SearchAction)">{{ title }}</button>
      <label>移到分组<select aria-label="移到分组" value="__choose" @change="move(($event.target as HTMLSelectElement).value)"><option value="__choose" disabled>选择分组</option><option value="">默认父目录分组</option><option v-for="group in preferences.groups" :key="group.id" :value="group.id">{{ group.name }}</option></select></label>
    </div></Teleport>
  </div>
</template>
