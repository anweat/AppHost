// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
const {invoke}=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke,Channel:class {onmessage?:unknown}}));
import { useFeedback } from './useFeedback';
import AppTile from './components/AppTile.vue';
import type { FailureReport } from './diagnostics';
const report=(at=100):FailureReport=>({failure:{failure_id:'f',instance_id:'i',at_ms:at,service:'web',phase:'start',collecting:false,gaps:[],stderr_tail:''},details_expired:false,history:{},advice:{code:'PORT_IN_USE',title:'端口已被占用',impact:'应用未能完成启动',consecutive_same_cause:2,clears_when:'停止或重试',evidence:{inferred:false,rule_id:'port'},actions:[{id:'edit_manifest',title:'检查声明端口',primary:true,argv:[]}],hints:[]}});
afterEach(()=>{vi.useRealTimers();});
describe('persistent explicit failure feedback',()=>{
  it.each(['REVIEW_REQUIRED','BUSY','STALE_LAUNCH','RESTART_REQUIRED','RUNTIME_LIMIT','BACKEND_MISSING','RPC_TIMEOUT'])('keeps %s visible without an unrelated details link when no matching record exists',async code=>{
    vi.useFakeTimers();invoke.mockReset().mockImplementation(async cmd=>{if(cmd==='desktop_failure')return report(10);return null;});
    const feedback=useFeedback(), error={code,message:'本次请求的具体错误原因'};
    const reading=feedback.failure('a',90,undefined,error);
    expect(feedback.feedback.value.get('a')?.text).toBe(error.message);
    await vi.advanceTimersByTimeAsync(2100);await reading;
    const item=feedback.feedback.value.get('a')!;expect(item.error).toEqual(error);expect(item.report).toBeUndefined();expect(item.impact).toContain('未确认完成');
    const app={installation_id:'a',project_id:'p',label:'应用',path:'/app',registration_state:'REGISTERED',requires_confirmation:code==='REVIEW_REQUIRED',runtime:null};
    const tile=mount(AppTile,{props:{app,snapshot:{apps:[app],live:false,read_at_ms:91,runtime_error:null},feedback:item}});await flushPromises();
    expect(tile.find('.tile-feedback').text()).toContain(code);expect(tile.find('.tile-feedback').text()).toContain(error.message);
    expect(tile.findAll('.tile-feedback button').map(b=>b.text())).not.toContain('详情');expect(tile.findAll('.tile-feedback button')).toHaveLength(2);
    tile.unmount();feedback.dispose();
  });
  it('clears a control error only on fresh resolution, not a historical success',async()=>{
    invoke.mockReset().mockRejectedValue({code:'IO_ERROR'});const feedback=useFeedback();
    const app={installation_id:'a',project_id:'p',label:'应用',path:'/app',registration_state:'REGISTERED',requires_confirmation:true,runtime:null};
    await feedback.failure('a',100,undefined,{code:'REVIEW_REQUIRED',message:'需要复核'});
    feedback.resolved({apps:[{...app,requires_confirmation:false}],live:false,read_at_ms:99,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(true);
    feedback.resolved({apps:[{...app,requires_confirmation:false}],live:false,read_at_ms:101,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(false);
    await feedback.failure('a',100,undefined,{code:'BUSY',message:'busy'});
    const runtime={installation_id:'a',instance_id:'i',updated_at_ms:10,status:{state:'RUNNING' as const,failure:null},services:[]};
    feedback.resolved({apps:[{...app,runtime}],live:true,read_at_ms:101,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(true);
    feedback.resolved({apps:[{...app,runtime:{...runtime,updated_at_ms:110}}],live:true,read_at_ms:111,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(false);feedback.dispose();
  });
  it('retains the original bootstrap error after all evidence reads return NOT_FOUND',async()=>{
    vi.useFakeTimers();invoke.mockReset().mockRejectedValue({code:'NOT_FOUND',message:'No failure record'});
    const feedback=useFeedback(),error={code:'BACKEND_MISSING',message:'Bundled apphost executable is missing'};
    const reading=feedback.failure('a',100,undefined,error);
    await vi.advanceTimersByTimeAsync(2100);await reading;
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_failure')).toHaveLength(5);
    expect(feedback.feedback.value.get('a')).toMatchObject({error,text:error.message,persistent:true});
    expect(feedback.feedback.value.get('a')?.report).toBeUndefined();
    const app={installation_id:'a',project_id:'p',label:'应用',path:'/app',registration_state:'REGISTERED',requires_confirmation:false,runtime:null};
    feedback.resolved({apps:[app],live:true,read_at_ms:99,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(true);
    feedback.resolved({apps:[app],live:true,read_at_ms:101,runtime_error:null});expect(feedback.feedback.value.has('a')).toBe(false);feedback.dispose();
  });
  it('expires successes but retains core failure advice and offers retry only after port release',async()=>{
    vi.useFakeTimers();let channel:any;
    invoke.mockReset().mockImplementation(async(cmd:string,args:any)=>{if(cmd==='desktop_failure')return report();if(cmd==='desktop_condition_watch')channel=args.channel;});
    const feedback=useFeedback();feedback.bubble('ok','已就绪');await feedback.failure('bad',90,'i');
    await vi.advanceTimersByTimeAsync(9000);
    expect(feedback.feedback.value.has('ok')).toBe(false);
    expect(feedback.feedback.value.get('bad')?.report?.advice?.title).toBe('端口已被占用');
    channel.onmessage({data:{state:'cleared',failure_id:'different'}});expect(feedback.feedback.value.get('bad')?.cleared).toBeUndefined();
    channel.onmessage({data:{state:'cleared',failure_id:'f'}});expect(feedback.feedback.value.get('bad')?.cleared).toBe(true);
    expect(invoke.mock.calls.some(c=>c[0]==='desktop_action')).toBe(false);feedback.dispose();
  });
  it('waits for delayed publication without retrying a write or retaining old advice',async()=>{
    vi.useFakeTimers();invoke.mockReset().mockRejectedValueOnce({code:'NOT_FOUND'}).mockResolvedValueOnce(report(80)).mockResolvedValue(report());
    const feedback=useFeedback(),reading=feedback.failure('a',90,'i');await vi.advanceTimersByTimeAsync(500);await reading;
    expect(feedback.feedback.value.get('a')?.report?.failure?.failure_id).toBe('f');
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_failure')).toHaveLength(3);
    expect(invoke.mock.calls.some(c=>c[0]==='desktop_action')).toBe(false);feedback.dispose();
  });
  it.each(['dismiss','retry','stale-record'] as const)('does not revive stale advice after %s',async reason=>{
    vi.useFakeTimers();
    let resolve!:(r:FailureReport)=>void;
    invoke.mockReset().mockImplementation(()=>new Promise(r=>{resolve=r;}));
    const feedback=useFeedback(), reading=feedback.failure('a',90,'i');
    if(reason==='dismiss')feedback.dismiss('a');if(reason==='retry')feedback.bubble('a','已受理');
    resolve(report(reason==='stale-record'?80:100));if(reason==='stale-record'){invoke.mockResolvedValue(report(80));await vi.advanceTimersByTimeAsync(2100);}await reading;
    expect(feedback.feedback.value.get('a')?.report).toBeUndefined();
    if(reason==='dismiss')expect(feedback.feedback.value.has('a')).toBe(false);
    if(reason==='retry')expect(feedback.feedback.value.get('a')?.text).toBe('已受理');feedback.dispose();
  });
  it('renders one core primary action, repeat explanation and an accessible dismissal',async()=>{
    invoke.mockReset().mockResolvedValue(null);
    const app={installation_id:'a',project_id:'p',label:'阅读器',path:'/app',registration_state:'REGISTERED',requires_confirmation:false,runtime:null};
    const item={text:'端口已被占用',persistent:true,report:report()};
    const tile=mount(AppTile,{attachTo:document.body,props:{app,snapshot:{apps:[app],live:true,read_at_ms:1,runtime_error:null},feedback:item}});await flushPromises();
    expect(tile.find('.tile-feedback').text()).toContain('应用未能完成启动');expect(tile.text()).toContain('与上次原因相同');
    const buttons=tile.findAll('.tile-feedback button');expect(buttons.map(b=>b.text())).toEqual(['×','检查声明端口','详情']);
    await buttons[1]!.trigger('click');expect(tile.emitted('feedbackAction')?.[0]).toEqual(['a',item]);
    expect(tile.find('.tile-main').attributes('aria-describedby')).toBe(tile.find('.tile-feedback').attributes('id'));
    await buttons[1]!.trigger('keydown',{key:'Escape'});expect(tile.emitted('dismiss')).toEqual([['a']]);expect(document.activeElement).toBe(tile.find('.tile-main').element);tile.unmount();
  });
});
