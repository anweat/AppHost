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
