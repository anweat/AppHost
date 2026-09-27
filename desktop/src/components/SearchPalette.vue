<script setup lang="ts">
import type { SearchAction } from '../search';
type Option={key:string;label:string;section:string;hint?:string;id?:string;action?:SearchAction;global?:string;disabled?:boolean};
defineProps<{items:Option[];active:number}>();
defineEmits<{pick:[item:Option];hover:[index:number];insert:[text:string]}>();
</script>
<template>
  <section class="search-palette" aria-label="搜索与操作" @pointerdown.prevent>
    <div id="search-palette" role="listbox" aria-label="搜索结果" class="palette-results">
      <template v-for="(item,index) in items" :key="item.key">
        <p v-if="!index || item.section!==items[index-1]?.section" class="palette-heading" role="presentation">{{ item.section }}</p>
        <div :id="'search-option-'+index" role="option" :aria-selected="active===index" :aria-disabled="item.disabled || undefined" class="palette-option" @mouseenter="$emit('hover',index)" @click="!item.disabled && $emit('pick',item)"><span>{{ item.label }}</span><small v-if="item.hint">{{ item.hint }}</small></div>
      </template>
      <p v-if="!items.length" class="palette-empty">没有匹配结果</p>
    </div>
    <div class="palette-syntax"><button @click="$emit('insert','is:')"><code>is:</code> 状态</button><button @click="$emit('insert','in:')"><code>in:</code> 分组</button><button @click="$emit('insert','>')"><code>&gt;</code> 操作</button></div>
    <div class="palette-help">↑↓ 选择 · 回车执行 · Shift+回车 备选 · Esc 返回</div>
  </section>
</template>
