<script setup lang="ts">
import type { Preferences } from '../preferences';
const props=defineProps<{preferences:Preferences;busy:boolean}>();
const emit=defineEmits<{change:[value:unknown]}>();
function change(key:string,value:unknown){emit('change',{kind:'search',search:{...props.preferences.search,[key]:value}});}
</script>
<template><section class="settings-page"><h2>搜索与操作索引</h2>
  <label class="setting"><span>回车默认操作</span><select :value="preferences.search.enter" :disabled="busy" @change="change('enter',($event.target as HTMLSelectElement).value)"><option value="primary">启动或打开应用</option><option value="details">查看详情</option></select></label>
  <label class="setting"><span>匹配项目路径</span><input type="checkbox" :checked="preferences.search.match_path" :disabled="busy" @change="change('match_path',($event.target as HTMLInputElement).checked)"></label>
  <label class="setting"><span>显示操作结果</span><input type="checkbox" :checked="preferences.search.show_actions" :disabled="busy" @change="change('show_actions',($event.target as HTMLInputElement).checked)"></label>
  <p class="muted">is:running、is:failed、in:分组、&gt; 仅搜索操作；例如“停止 notes”。Shift+回车执行应用的备选操作。</p>
</section></template>
