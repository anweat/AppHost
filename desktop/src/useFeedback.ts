import { ref } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import type { FailureReport } from './diagnostics';
import type { Snapshot } from './model';

export interface Feedback {
  text: string;
  persistent: boolean;
  report?: FailureReport;
  cleared?: boolean;
}
// Only explicit single-app operations create bubbles. Tokens bind late reads and
// condition events to the displayed result, so dismissal/retry cannot revive it.
export function useFeedback() {
  const feedback = ref(new Map<string, Feedback>());
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const tokens = new Map<string, string>();
  const conditions = new Map<string, string>();
  function dismiss(id:string) {
    feedback.value.delete(id); tokens.delete(id);
    clearTimeout(timers.get(id)); timers.delete(id);
    const token=conditions.get(id);
    if(token) { conditions.delete(id); void invoke('desktop_condition_unwatch',{token}).catch(()=>{}); }
  }
  function bubble(id:string,text:string) {
    dismiss(id); feedback.value.set(id,{text,persistent:false});
    timers.set(id,setTimeout(()=>dismiss(id),8000));
  }
  async function failure(id:string, since:number, instance?:string) {
    dismiss(id);
    const token=crypto.randomUUID(); tokens.set(id,token);
    feedback.value.set(id,{text:'操作未能完成，请查看失败详情',persistent:true});
    try {
      let report:FailureReport|undefined;
      // State notifications can precede publication of the failure file. Retry
      // reads only (five attempts, 2 s total backoff); never show older advice.
      for(const delay of [0,100,300,600,1000]) {
        if(delay) await new Promise(resolve=>setTimeout(resolve,delay));
        if(tokens.get(id)!==token) return;
        try { report=await invoke<FailureReport>('desktop_failure',{installationId:id,failureId:null}); }
        catch(cause) { if((cause as {code?:string})?.code!=='NOT_FOUND') return; }
        if(report?.failure && report.advice && report.failure.at_ms>=since &&
          (!instance || !report.failure.instance_id || report.failure.instance_id===instance)) break;
        report=undefined;
      }
      if(tokens.get(id)!==token || !report?.failure || !report.advice) return;
      const currentReport=report;
      feedback.value.set(id,{text:report.advice.title,persistent:true,report});
      if(report.advice.code!=='PORT_IN_USE') return;
      const channel=new Channel<{data:{state:string;failure_id:string}}>();
      channel.onmessage=event=>{
        const item=feedback.value.get(id);
        if(tokens.get(id)===token && event.data.failure_id===currentReport.failure!.failure_id && event.data.state==='cleared' && item) item.cleared=true;
      };
      conditions.set(id,token);
      try { await invoke('desktop_condition_watch',{installationId:id,failureId:report.failure.failure_id,token,channel}); }
      finally { if(conditions.get(id)===token) conditions.delete(id); }
    } catch { /* Evidence/watch unavailable: retain the failure and its details entry. */ }
  }
  function resolved(snapshot:Snapshot|null) {
    if(!snapshot?.live) return;
    for(const [id,item] of feedback.value) {
      if(!item.persistent || !item.report) continue;
      const app=snapshot.apps.find(a=>a.installation_id===id);
      if(!app || (app.runtime && !app.runtime.operation_error && ['RUNNING','STOPPED'].includes(app.runtime.status.state))) dismiss(id);
    }
  }
  function dispose(){ for(const id of [...feedback.value.keys()]) dismiss(id); }
  return {feedback,bubble,failure,dismiss,resolved,dispose};
}
