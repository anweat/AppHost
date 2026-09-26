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
    expect(invoke).toHaveBeenCalledTimes(1); expect(runtime.message.value).toContain('RPC_TIMEOUT');
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
