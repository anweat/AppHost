<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage } from '../model';
const emit=defineEmits<{saved:[]}>();
const enabled = ref<boolean>(), loginChoice = ref(''), port = ref(18480), busy = ref(''), message = ref(''), loginError = ref('');
onMounted(async () => {
  await Promise.allSettled([
    invoke<{enabled:boolean}>('desktop_login_read').then(v => {enabled.value=v.enabled;}).catch(e => {loginError.value=errorMessage(e).message;}),
    invoke<{port:number}>('desktop_gateway_read').then(v => {port.value=v.port;}).catch(e => {message.value=errorMessage(e).message;}),
  ]);
});
async function save(kind:'login'|'gateway') {
  if (busy.value || (kind === 'login' && !loginChoice.value)) return;
  busy.value=kind; message.value='';
  try {
    if (kind === 'login') {const value=await invoke<{enabled:boolean}>('desktop_login_save',{enabled:loginChoice.value === 'on'}); enabled.value=value.enabled; emit('saved'); message.value='已保存，下次登录生效；不会自动启动应用。';}
    else {await invoke('desktop_gateway_save',{port:port.value}); message.value='网关端口已保存，现有书签需要使用新端口。';}
  } catch(e) {message.value=`${errorMessage(e).code} · ${errorMessage(e).message}`;}
  finally {busy.value='';}
}
</script>
<template><section class="settings-page"><h2>后台与网关</h2>
  <h3>登录自启</h3><p class="muted">仅在登录时启动 AppHost 后台，不启动任何应用。当前：{{ enabled === undefined ? '不可用' : enabled ? '开启' : '关闭' }}</p>
  <p v-if="loginError" class="notice">{{ loginError }}</p>
  <div class="row-actions"><label><input v-model="loginChoice" type="radio" value="on" name="login-choice">开启</label><label><input v-model="loginChoice" type="radio" value="off" name="login-choice">关闭</label><button :disabled="!!busy || !loginChoice || enabled === undefined" @click="save('login')">保存登录选择</button></div>
  <h3>网关端口</h3><p class="muted">默认 18480。端口被占用时报告冲突，不自动换端口。</p><div class="row-actions"><label>端口 <input v-model.number="port" type="number" min="1" max="65535"></label><button :disabled="!!busy || !Number.isInteger(port) || port < 1 || port > 65535" @click="save('gateway')">保存端口</button></div>
  <p v-if="message" role="status" class="notice">{{ message }}</p></section></template>
