<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import AppTile from './AppTile.vue';
import type { AppEntry, Snapshot } from '../model';
const props = defineProps<{ apps: AppEntry[]; snapshot: Snapshot; lens: string | null }>();
defineEmits<{ details: [id: string]; primary: [id: string] }>();
const groups = computed(() => {
  const result = new Map<string, { name: string; apps: AppEntry[] }>();
  for (const app of props.apps) {
    const key = app.group || 'apps';
    if (!result.has(key)) result.set(key, { name: app.group_label || '应用', apps: [] });
    result.get(key)!.apps.push(app);
  }
  return [...result].map(([key, value]) => ({ key, ...value }));
});
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
onMounted(() => { observer = new ResizeObserver(layout); if (board.value) observer.observe(board.value); });
onUnmounted(() => observer?.disconnect());
</script>
<template>
  <div ref="board" class="board" :data-offline="!snapshot.live" :data-lens="lens" data-at-start data-at-end @wheel="scroll">
    <div class="canvas"><section v-for="group in groups" :key="group.key" class="group" :aria-label="group.name">
      <h2 class="group-title" :title="group.key">{{ group.name }}</h2>
      <div class="group-grid"><AppTile v-for="app in group.apps" :key="app.installation_id" :app="app" :snapshot="snapshot" @details="$emit('details', $event)" @primary="$emit('primary', $event)" /></div>
    </section></div>
  </div>
</template>
