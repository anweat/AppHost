<script setup lang="ts">
import { ref } from 'vue';
import type { Preferences } from '../preferences';
defineProps<{preferences:Preferences;busy:boolean}>();
defineEmits<{change:[value:unknown]}>();
const name=ref(''), rename=ref(''), editing=ref('');
</script>
<template><section class="settings-page"><h2>分组</h2><p class="muted">分组内始终按登记顺序排列。解散分组后，成员回到默认父目录分组。</p>
<div class="row-actions"><input v-model="name" placeholder="新分组名称" aria-label="新分组名称" maxlength="64"><button :disabled="busy || !name.trim()" @click="$emit('change',{kind:'create-group',name}); name=''">新建分组</button></div>
<ul class="service-list"><li v-for="group in preferences.groups" :key="group.id"><template v-if="editing === group.id"><input v-model="rename" aria-label="分组新名称" maxlength="64"><button :disabled="busy || !rename.trim()" @click="$emit('change',{kind:'rename-group',id:group.id,name:rename});editing=''">保存</button><button @click="editing=''">取消</button></template><template v-else><strong>{{ group.name }}</strong><div class="row-actions"><button @click="editing=group.id;rename=group.name">重命名</button><button :disabled="busy" @click="$emit('change',{kind:'dissolve-group',id:group.id})">解散</button></div></template></li></ul>
<label class="setting"><span><strong>最近使用组</strong><small>登记达到 12 个后显示最多 6 个图标，其他分组位置不变。</small></span><input type="checkbox" :checked="preferences.show_recent" :disabled="busy" @change="$emit('change',{kind:'recent',enabled:($event.target as HTMLInputElement).checked})"></label>
</section></template>
