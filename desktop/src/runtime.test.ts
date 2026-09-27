import { effectScope, nextTick, ref } from 'vue';
import { describe, expect, it, vi } from 'vitest';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => false, Channel: class {} }));
import { useRuntime } from './useRuntime';
import type { Snapshot } from './model';
const snapshot = (): Snapshot => ({ live: true, read_at_ms: 1, runtime_error: null, apps: [{
  installation_id:'app', project_id:'project', label:'App', path:'/app', registration_state:'REGISTERED', requires_confirmation:false, has_entry:true, runtime:null,
}] });
const receipt = { installation_id:'app', instance_id:'instance', operation_id:'operation' };
function ready(value: Snapshot): Snapshot {
  return { ...value, apps: value.apps.map(app => ({ ...app, runtime: { ...receipt, updated_at_ms:2, status:{state:'RUNNING' as const,failure:null}, services:[{name:'web',port:3000,alive:true,ready:true}], operation_done:true } })) };
}
describe('explicit desktop runtime intent', () => {
  it.each(['start', 'restart'] as const)('dispatches offline %s once and reconnects observations', async kind => {
    invoke.mockReset().mockResolvedValue(receipt);
    const value = ref({...snapshot(),live:false}), scope = effectScope(), connected = vi.fn();
    const runtime = scope.run(() => useRuntime(value, async () => {}, vi.fn(), connected, vi.fn()))!;
    await runtime.action(kind, 'app');
    expect(invoke.mock.calls.filter(c => c[0] === 'desktop_action')).toHaveLength(1);
    expect(connected).toHaveBeenCalledOnce(); scope.stop();
  });

  it.each(['absent','empty','transport-error'] as const)('does not dispatch Stop All for %s',async mode=>{
    invoke.mockReset();
    const state=mode==='transport-error'?{...ready(snapshot()),live:false,runtime_error:{code:'PERMISSION_DENIED',message:'denied'}}:mode==='absent'?{...ready(snapshot()),live:false}:snapshot();
    const scope=effectScope(), value=ref(state);
    const runtime=scope.run(()=>useRuntime(value,async()=>{},vi.fn(),vi.fn(),vi.fn()))!;
    expect(runtime.canStopAll.value).toBe(false);await runtime.action('stop-all');expect(invoke).not.toHaveBeenCalled();runtime.dispose();scope.stop();
  });
  it('blocks explicit actions on transport errors, without treating stale running state as current',async()=>{
    invoke.mockReset();const value=ref({...ready(snapshot()),live:false,runtime_error:{code:'SOCKET_UNSAFE',message:'unsafe'}}),scope=effectScope();
    const runtime=scope.run(()=>useRuntime(value,async()=>{},vi.fn(),vi.fn(),vi.fn()))!;
    await runtime.action('start','app');await runtime.action('restart','app');expect(invoke).not.toHaveBeenCalled();runtime.dispose();scope.stop();
  });
  it.each(['receipt','exception'])('passes the actual %s error into persistent feedback',async source=>{
    const error={code:'BACKEND_MISSING',message:'Bundled apphost executable is missing'};
    invoke.mockReset().mockImplementation(async cmd=>{if(cmd==='desktop_failure')throw {code:'IO_ERROR'};if(cmd==='desktop_action'){if(source==='exception')throw error;return {installation_id:'app',error};}});
    const scope=effectScope(),value=ref(snapshot());const runtime=scope.run(()=>useRuntime(value,async()=>{},vi.fn(),vi.fn(),vi.fn()))!;
    await runtime.action('start','app');await nextTick();expect(runtime.feedback.value.get('app')?.error).toEqual(error);expect(runtime.feedback.value.get('app')?.text).toBe(error.message);
    runtime.dispose();scope.stop();
  });
  it('opens once for the matching ready instance, never for repeated snapshots', async () => {
    invoke.mockReset().mockImplementation(async (command: string) => command === 'desktop_action' ? receipt : undefined);
    const value = ref(snapshot()), scope = effectScope();
    const runtime = scope.run(() => useRuntime(value, async () => { value.value = ready(value.value); }, vi.fn(), vi.fn(), vi.fn()))!;
    await runtime.action('start', 'app'); await nextTick();
    value.value = { ...value.value }; await nextTick();
    expect(invoke.mock.calls.filter(call => call[0] === 'desktop_open')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('desktop_open', { installationId:'app',instanceId:'instance',operationId:'operation' });
    scope.stop();
  });
  it('keeps group launch results in activity without spawning single-app bubbles',async()=>{
    invoke.mockReset().mockResolvedValue(receipt);const value=ref(snapshot()),scope=effectScope();
    const runtime=scope.run(()=>useRuntime(value,async()=>{value.value=ready(value.value);},vi.fn(),vi.fn(),vi.fn()))!;
    await runtime.action('start','app',undefined,true);await nextTick();
    expect(runtime.activities.value.get('app')?.text).toBe('已就绪');expect(runtime.feedback.value.size).toBe(0);runtime.dispose();scope.stop();
  });
  it('Stop All cancels a start intent whose acknowledgement arrives late', async () => {
    let resolve!: (value: typeof receipt) => void;
    invoke.mockReset().mockImplementation((command: string, params: {action?:string}) => {
      if (command === 'desktop_action' && params.action === 'start') return new Promise(done => { resolve = done; });
      return Promise.resolve({ results:[] });
    });
    const value = ref(snapshot()), scope = effectScope();
    const runtime = scope.run(() => useRuntime(value, async () => {}, vi.fn(), vi.fn(), vi.fn()))!;
    const start = runtime.action('start', 'app'); await runtime.action('stop-all'); resolve(receipt); await start;
    value.value = ready(value.value); await nextTick();
    expect(invoke.mock.calls.filter(call => call[0] === 'desktop_open')).toHaveLength(0);
    scope.stop();
  });
  it('an uncertain mutation receipt is reported, never automatically retried', async () => {
    invoke.mockReset().mockRejectedValue({code:'RPC_TIMEOUT',message:'deadline'});
    const value = ref(snapshot()), scope = effectScope();
    const runtime = scope.run(() => useRuntime(value, async () => {}, vi.fn(), vi.fn(), vi.fn()))!;
    await runtime.action('start', 'app');
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_action')).toHaveLength(1); expect(runtime.message.value).toContain('RPC_TIMEOUT');
    scope.stop();
  });
  it.each(['replaced', 'offline', 'closed'])('does not open a stale browser intent after %s', async reason => {
    invoke.mockReset().mockResolvedValue(receipt);
    const value = ref(snapshot()), scope = effectScope();
    const runtime = scope.run(() => useRuntime(value, async () => {}, vi.fn(), vi.fn(), vi.fn()))!;
    await runtime.action('start', 'app');
    if (reason === 'closed') runtime.dispose();
    if (reason === 'offline') { value.value = { ...value.value, live:false }; await nextTick(); }
    value.value = ready({ ...value.value, live:true });
    if (reason === 'replaced') value.value.apps[0]!.runtime!.instance_id = 'replacement';
    await nextTick();
    expect(invoke.mock.calls.filter(call => call[0] === 'desktop_open')).toHaveLength(0);
    scope.stop();
  });
});

describe('batch activity outcomes', () => {
  it('keeps every Stop All item and distinguishes cancellation, errors and orphaned processes', async () => {
    const value=ref(snapshot()),scope=effectScope();
    value.value.apps[0]!.runtime={...receipt,updated_at_ms:1,operation_done:false,status:{state:'STARTING',failure:null},services:[]};
    value.value.apps.push({...value.value.apps[0]!,installation_id:'other',label:'Other',runtime:null});
    invoke.mockReset().mockResolvedValue({results:[{...receipt,operation_id:'stop'},{installation_id:'other',error:{code:'BUSY',message:'busy'}}]});
    const runtime=scope.run(()=>useRuntime(value,async()=>{},vi.fn(),vi.fn(),vi.fn()))!;
    await runtime.action('stop-all');
    expect(runtime.activities.value.get('app')?.text).toContain('取消启动');expect(runtime.activities.value.get('other')?.text).toContain('BUSY');
    value.value={...value.value,apps:value.value.apps.map(a=>a.installation_id==='app'?{...a,runtime:{...a.runtime!,operation_id:'stop',operation_done:true,management_uncertain:true,status:{state:'FAILED',failure:'MANAGEMENT_UNCERTAIN'}}}:a)};await nextTick();
    expect(runtime.activities.value.get('app')?.text).toContain('遗留进程');runtime.dispose();scope.stop();
  });
});
