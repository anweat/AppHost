<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { errorMessage } from '../model';
const needed=ref(false), busy=ref(false), error=ref('');
async function refresh() {
  if(!isTauri()) return;
  try {
    const result=await invoke<{choice_made:boolean}>('desktop_login_read');
    needed.value=!result.choice_made; error.value='';
  } catch(cause) {
    const e=errorMessage(cause);
    if(e.code==='UNSUPPORTED_PLATFORM') {needed.value=false;return;}
    needed.value=true; error.value=e.message;
  }
}
async function choose(enabled:boolean) {
  if(busy.value) return;
  busy.value=true; error.value='';
  try { const result=await invoke<{choice_made:boolean}>('desktop_login_save',{enabled}); needed.value=!result.choice_made; }
  catch(cause) {error.value=errorMessage(cause).message;}
  finally {busy.value=false;}
}
onMounted(refresh);
defineExpose({refresh});
</script>
<template><section v-if="needed" class="login-choice" aria-label="首次登录自启选择">
  <div><strong>登录时启动 AppHost 后台？</strong><p>只启动后台，不启动应用。以后可在“后台与网关”修改。</p><p v-if="error" role="status">{{ error }}</p></div>
  <button :disabled="busy" @click="choose(true)">开启登录自启</button><button :disabled="busy" @click="choose(false)">保持关闭</button>
</section></template>
