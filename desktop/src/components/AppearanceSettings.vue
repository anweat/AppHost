<script setup lang="ts">
defineProps<{ theme: string; saving: boolean }>();
defineEmits<{ theme: [value: string] }>();
</script>
<template>
    <section class="settings-page"><h2>外观</h2>
      <label class="setting"><span><strong>主题</strong><small>默认跟随系统外观。</small></span>
        <select :value="theme" :disabled="saving" aria-label="主题" @change="$emit('theme', ($event.target as HTMLSelectElement).value)">
          <option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option>
        </select>
      </label>
      <p class="note">关闭窗口后，后台和已启动的应用继续运行。需要结束应用时，请使用“全部停止”。</p>
    </section>
</template>
