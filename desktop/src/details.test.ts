// @vitest-environment happy-dom
import { effectScope } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
const { invoke } = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke}));
import { useDetailRead } from './useDetailRead';
import { logEntries, logWarnings, type LogResult } from './logs';
import { commandText } from './diagnostics';
import LogsPanel from './components/LogsPanel.vue';
import FailurePanel from './components/FailurePanel.vue';
import DiagnosticPreview from './components/DiagnosticPreview.vue';
const logs = (): LogResult => ({instance_id:'instance',read_at_ms:1,from_ms:0,to_ms:1,log_incomplete:false,services:[{name:'web',key:'s0'}],window:{chunks:[],warnings:[],prefix_evicted:false,incomplete_tail:false,truncated:false,sequence_gap:false,clock_reversed:false}});
describe('bounded detail reads', () => {
  it('coalesces duplicate reads and ignores a response after the panel closes', async () => {
    let resolve!: (value: string) => void;
    const read = vi.fn(() => new Promise<string>(done => {resolve = done;})), scope = effectScope();
    const result = scope.run(() => useDetailRead(read))!;
    const pending = result.refresh(); await result.refresh(); scope.stop(); resolve('old installation'); await pending;
    expect(read).toHaveBeenCalledTimes(1); expect(result.data.value).toBeNull();
  });
  it('retains the last successful same-panel data on an unsuccessful refresh', async () => {
    const scope = effectScope();
    const read = vi.fn().mockResolvedValueOnce('old log').mockRejectedValue({code:'DAEMON_UNAVAILABLE',message:'offline'});
    const result = scope.run(() => useDetailRead(read))!;
    await result.refresh(); await result.refresh();
    expect(result.data.value).toBe('old log'); expect(result.error.value?.code).toBe('DAEMON_UNAVAILABLE'); scope.stop();
  });
  it('decodes split UTF-8 separately for interleaved stdout and stderr', () => {
    const value = logs(), bytes = [...new TextEncoder().encode('你好')];
    value.window.chunks = [
      {sequence:1,observed_ms:1,service_key:'s0',stream:'stdout',bytes:bytes.slice(0,2)},
      {sequence:2,observed_ms:2,service_key:'s0',stream:'stderr',bytes:[255]},
      {sequence:3,observed_ms:3,service_key:'s0',stream:'stdout',bytes:bytes.slice(2)},
    ];
    const entries = logEntries(value);
    expect(entries.filter(e => e.stream === 'stdout').map(e => e.text).join('')).toBe('你好');
    expect(entries[1]?.text).toBe('�');
    value.log_incomplete = true; value.window.truncated = true; value.window.sequence_gap = true;
    expect(logWarnings(value)).toHaveLength(3);
  });
  it('renders log markup as text and does not re-read merely because live state changes', async () => {
    const value = logs();
    value.window.chunks = [{sequence:1,observed_ms:1,service_key:'s0',stream:'stdout',bytes:[...new TextEncoder().encode('<img src=x onerror=bad()>')]}];
    invoke.mockReset().mockResolvedValue(value);
    const panel = mount(LogsPanel, {props:{installationId:'app',instanceId:'instance',live:true}}); await flushPromises();
    expect(panel.find('img').exists()).toBe(false); expect(panel.text()).toContain('<img src=x onerror=bad()>');
    await panel.setProps({live:false}); expect(panel.text()).toContain('未同步'); expect(invoke).toHaveBeenCalledTimes(1);
    expect(panel.find('button').attributes('disabled')).toBeDefined(); panel.unmount();
  });
  it('uses the core-selected primary advice without executing copy-only commands', async () => {
    invoke.mockReset().mockResolvedValue({failure:{failure_id:'f',at_ms:1,gaps:[],collecting:false},history:{},advice:{title:'核心判断',code:'DEPENDENCY_MISSING',impact:'未启动',consecutive_same_cause:1,evidence:{inferred:true,rule_id:'rule'},hints:[],actions:[{id:'copy_dependency_command',title:'复制建议命令',primary:true,argv:['npm','install'],copy_only:true}]}});
    const panel = mount(FailurePanel, {props:{installationId:'app',canRetry:true}}); await flushPromises();
    await panel.find('button.primary').trigger('click'); await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_copy',{text:'npm install'});
    expect(panel.emitted('retry')).toBeUndefined(); expect(invoke.mock.calls.some(c => c[0] === 'desktop_action')).toBe(false); panel.unmount();
  });
  it('copies exactly the displayed diagnostic preview, never regenerates on copy', async () => {
    const preview = {sections:[{kind:'application',value:{label:'App'}},{kind:'output_tail',value:'<script>no execution</script>'}],omitted:['environment_variable_values'],redaction:'best_effort',redaction_context_available:true};
    invoke.mockReset().mockResolvedValue(preview);
    const panel = mount(DiagnosticPreview,{props:{installationId:'app',failureId:'failure'}});
    expect(panel.find('button.primary').attributes('disabled')).toBeDefined(); await flushPromises();
    const visible = (panel.find('textarea').element as HTMLTextAreaElement).value;
    await panel.find('button.primary').trigger('click'); await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_copy',{text:visible});
    expect(invoke.mock.calls.filter(c => c[0] === 'desktop_diagnostics')).toHaveLength(1);
    expect(visible).toContain('environment_variable_values'); panel.unmount();
  });
  it('quotes arbitrary suggested argv as data without shell evaluation', () => {
    expect(commandText(['tool','$(touch x)',"a'b"])).toBe("tool '$(touch x)' 'a'\\''b'");
  });
});
