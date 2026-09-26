<script setup lang="ts">
withDefaults(defineProps<{ name: string }>(), { name: 'info' });
const paths: Record<string, string> = {
  search: 'M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14Zm5-2 5 5',
  settings: 'M6 3v7m0 4v7m6-18v3m0 4v11m6-18v11m0 4v3M3 10h6v4H3v-4Zm6-4h6v4H9V6Zm6 8h6v4h-6v-4',
  stop: 'M7 7h10v10H7V7Z', plus: 'M12 5v14M5 12h14', close: 'm6 6 12 12M18 6 6 18',
  refresh: 'M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 6M4 12l2 6a7 7 0 0 0 12-1',
  play: 'm8 5 11 7-11 7V5Z', arrow: 'M5 12h14m-6-6 6 6-6 6', info: 'M12 11v6m0-10v.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',
  moon: 'M20 15A9 9 0 0 1 9 3a9 9 0 1 0 11 12Z', home: 'm3 10 9-7 9 7M5 9v12h14V9M9 21v-8h6v8',
};
</script>
<template><svg viewBox="0 0 24 24" aria-hidden="true"><path :d="paths[name] || paths.info" /></svg></template>
