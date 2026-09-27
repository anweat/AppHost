import { ref, watch, type Ref } from 'vue';
import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Snapshot } from './model';
import { errorMessage } from './model';
export type Action = 'start' | 'stop' | 'restart' | 'stop-all' | 'resume-wake' | 'stop-owner-and-start';
interface Receipt { installation_id: string; instance_id?: string; operation_id?: string; operation_done?: boolean; error?: { code: string; message: string }; results?: Receipt[] }
export interface Activity {id:string;label:string;action:string;text:string;done:boolean;canceled?:boolean}
interface Intent { instance: string; operation: string }

export function useRuntime(snapshot: Ref<Snapshot | null>, refresh: () => Promise<void>, invalidate: () => void, connected: () => void, offline: (error: unknown) => void) {
  const activities = ref(new Map<string,Activity>()), feedback = ref(new Map<string,string>());
  const bubbleTimers = new Map<string,ReturnType<typeof setTimeout>>();
  function bubble(id:string,text:string) {feedback.value.set(id,text);clearTimeout(bubbleTimers.get(id));bubbleTimers.set(id,setTimeout(()=>{feedback.value.delete(id);bubbleTimers.delete(id);},8000));}
  function dismiss(id:string){feedback.value.delete(id);clearTimeout(bubbleTimers.get(id));bubbleTimers.delete(id);}
  const connecting = ref(false), busy = ref(new Set<string>()), message = ref('');
  const pending = new Map<string, Intent>(), observed = new Map<string, string>();
  const versions = new Map<string, number>();
  let epoch = 0, watching = false, disposed = false;
  let watchTask: Promise<void> | undefined;
  function startWatch() {
    if (watching || disposed || !isTauri()) return;
    watching = true;
    const channel = new Channel<{kind:string}>();
    channel.onmessage = event => { if (!disposed) event.kind === 'connected' ? connected() : invalidate(); };
    watchTask = invoke<void>('desktop_watch', { channel }).catch(cause => {
      if (!disposed) { pending.clear(); offline(cause); }
    }).finally(() => { watching = false; });
  }
  async function resumeWatch() {
    if (watching) { await invoke('desktop_unwatch'); await watchTask; }
    if (!disposed) { connected(); startWatch(); }
  }
  async function connect() {
    if (connecting.value) return;
    connecting.value = true;
    try {
      await invoke('desktop_connect');
      // Join the old subscription before replacing it, including a disconnect
      // racing this explicit reconnect. Its late error must not mark us offline.
      await resumeWatch();
    }
    catch (cause) { offline(cause); }
    finally { connecting.value = false; }
  }
  async function open(id: string, intent: Intent) {
    try { await invoke('desktop_open', { installationId: id, instanceId: intent.instance, operationId: intent.operation }); }
    catch (cause) { message.value = errorMessage(cause).message; }
  }
  async function action(kind: Action, id?: string, failureId?: string) {
    const wasOffline = !snapshot.value?.live;
    const key = id || kind;
    if (busy.value.has(key)) return;
    if (kind === 'stop-all') { epoch++; pending.clear(); observed.clear(); }
    if (id) {pending.delete(id);observed.delete(id);activities.value.set(id,{id,label:snapshot.value?.apps.find(a=>a.installation_id===id)?.label||id,action:kind,text:'正在提交',done:false});}
    const version = (versions.get(key) || 0) + 1; versions.set(key, version);
    const canceling=new Set(snapshot.value?.apps.filter(a=>a.runtime?.status.state==='STARTING').map(a=>a.installation_id));
    const startedEpoch = epoch;
    busy.value.add(key);
    try {
      const receipt = await invoke<Receipt>('desktop_action', { action: kind, installationId: id || null, ...(failureId ? {failureId} : {}) });
      if (disposed || startedEpoch !== epoch || versions.get(key) !== version) return;
      if (wasOffline) await resumeWatch();
      const receipts = receipt.results || [receipt];
      const failures = receipts.filter(item => item.error);
      for (const item of receipts) {
        if (!item.installation_id) continue;
        if (item.operation_id) observed.set(item.installation_id, item.operation_id);
        const canceled=kind==='stop-all' && canceling.has(item.installation_id);
        const text=item.error?`${item.error.code} · ${item.error.message}`:item.operation_done?'已完成':canceled?'已取消启动，正在停止':'已受理';
        activities.value.set(item.installation_id,{id:item.installation_id,label:snapshot.value?.apps.find(a=>a.installation_id===item.installation_id)?.label||item.installation_id,action:kind,text,done:!!item.error||!!item.operation_done,canceled});
        if(id)bubble(id,text);
      }
      message.value = failures.length ? `${failures.length} 项操作未接受：${failures[0]?.error?.message}` : kind === 'stop-all' ? '正在全部停止 · 自动唤醒已暂停' : kind === 'resume-wake' ? '自动唤醒已恢复' : '操作已接受';
      await refresh();
      if (disposed || startedEpoch !== epoch || versions.get(key) !== version) return;
      if (id && ['start','restart','stop-owner-and-start'].includes(kind) && receipt.instance_id && receipt.operation_id && snapshot.value?.apps.find(app => app.installation_id === id)?.has_entry) {
        pending.set(id, { instance: receipt.instance_id, operation: receipt.operation_id });
      }
      observe(snapshot.value);
    } catch (cause) { if (startedEpoch === epoch && !disposed) { message.value = `${errorMessage(cause).code} · ${errorMessage(cause).message}`; if(id){bubble(id,message.value);const item=activities.value.get(id);if(item){item.text=message.value;item.done=true;}} await refresh(); } }
    finally { busy.value.delete(key); }
  }
  function observe(value: Snapshot | null) {
    if (!value?.live) { pending.clear(); return; }
    for (const app of value.apps) {
      const runtime = app.runtime; if (!runtime) continue;
      const intent = pending.get(app.installation_id);
      if (intent) {
        if (runtime.operation_id !== intent.operation || runtime.instance_id !== intent.instance || ['STOPPING','STOPPED','FAILED'].includes(runtime.status.state)) pending.delete(app.installation_id);
        else if (runtime.status.state === 'RUNNING' && runtime.services.every(service => service.ready && service.alive)) {
          // Consume first: repeated snapshots and reconnections cannot open again.
          pending.delete(app.installation_id); void open(app.installation_id, intent);
        }
      }
      if (runtime.operation_id === observed.get(app.installation_id) && runtime.operation_done) {
        observed.delete(app.installation_id);
        const text=runtime.management_uncertain?'存在遗留进程，身份未能确认':runtime.operation_error ? `${runtime.operation_error.code} · ${runtime.operation_error.message || '操作未完成'}` : runtime.status.state === 'RUNNING' ? '已就绪' : runtime.status.state === 'FAILED' ? '运行失败' : '已停止';
        const item=activities.value.get(app.installation_id);
        if(item){item.done=true;item.text=`${item.canceled?'已取消启动 · ':''}${text}`;}
        bubble(app.installation_id,text); message.value=`${app.label} · ${text}`;
      }
    }
  }
  watch(snapshot, observe);
  function dispose() { disposed = true; pending.clear(); for(const timer of bubbleTimers.values())clearTimeout(timer);bubbleTimers.clear(); if (isTauri()) void invoke('desktop_unwatch'); }
  return { activities, feedback, dismiss, intentEpoch: () => epoch, intentVersion: (id:string) => versions.get(id)||0, connect, resumeWatch, startWatch, dispose, action, open, connecting, busy, message };
}
