// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
const { invoke } = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke}));
import EnvironmentSettings from './components/EnvironmentSettings.vue';
import RetentionSettings from './components/RetentionSettings.vue';
import ManagementPanel from './components/ManagementPanel.vue';
import { validRetention } from './settings';
import type { AppEntry } from './model';
const baseline = {schema_version:1,path:'/usr/bin:/bin',capture:{shell:'/bin/zsh',captured_at_ms:1}};
const retention = {schema_version:1,days:7,max_mib:100};
const apps: AppEntry[] = ['one','two'].map(id => ({installation_id:id,project_id:id,label:id,path:`/${id}`,registration_state:'REGISTERED',requires_confirmation:false,runtime:null}));
const button = (panel: ReturnType<typeof mount>, text: string) => panel.findAll('button').find(b => b.text().includes(text))!;
beforeEach(() => invoke.mockReset());
describe('environment settings boundaries', () => {
  it('reads only on entry and requires a separate explicit capture action', async () => {
    invoke.mockResolvedValueOnce(baseline).mockRejectedValueOnce({code:'CAPTURE_TIMEOUT',message:'timeout'});
    const panel = mount(EnvironmentSettings,{props:{apps}}); await flushPromises();
    await panel.find('textarea').setValue('/custom/bin');
    await button(panel,'从 shell 采集').trigger('click');
    expect(invoke.mock.calls.map(c => c[0])).toEqual(['desktop_environment_read']);
    await panel.find('[aria-label="采集 Shell"]').setValue('bash');
    await button(panel,'执行采集并保存').trigger('click'); await flushPromises();
    expect(invoke).toHaveBeenLastCalledWith('desktop_environment_capture',{shell:'bash'});
    expect(panel.find('textarea').element.value).toBe('/custom/bin');
    expect(panel.text()).toContain('CAPTURE_TIMEOUT'); expect(invoke).toHaveBeenCalledTimes(2); panel.unmount();
  });
  it('saves exact input, clears capture provenance, and shows audit warnings independently', async () => {
    invoke.mockResolvedValueOnce(baseline).mockResolvedValueOnce({schema_version:1,path:'/custom/bin:/bin',warnings:[{code:'JOURNAL_WRITE_FAILED',message:'audit unavailable'}]});
    const panel = mount(EnvironmentSettings,{props:{apps}}); await flushPromises();
    await panel.find('textarea').setValue('/custom/bin:/bin'); await button(panel,'保存 PATH').trigger('click'); await flushPromises();
    expect(invoke).toHaveBeenLastCalledWith('desktop_environment_save',{path:'/custom/bin:/bin'});
    expect(panel.text()).toContain('PATH 已保存'); expect(panel.text()).toContain('JOURNAL_WRITE_FAILED'); expect(panel.text()).not.toContain('上次采集'); panel.unmount();
  });
  it('preserves invalid edits on failed save and prevents checking unsaved PATH', async () => {
    invoke.mockResolvedValueOnce(baseline).mockRejectedValueOnce({code:'INVALID_PATH',message:'absolute paths required'});
    const panel = mount(EnvironmentSettings,{props:{apps,installationId:'one'}}); await flushPromises();
    await panel.find('textarea').setValue('relative'); expect(button(panel,'检查运行环境').attributes('disabled')).toBeDefined();
    await button(panel,'保存 PATH').trigger('click'); await flushPromises();
    expect(panel.find('textarea').element.value).toBe('relative'); expect(panel.text()).toContain('INVALID_PATH'); panel.unmount();
  });
  it('does not display a late check for a different installation', async () => {
    let resolve!: (value: unknown) => void;
    invoke.mockResolvedValueOnce(baseline).mockImplementationOnce(() => new Promise(r => {resolve = r;}));
    const panel = mount(EnvironmentSettings,{props:{apps,installationId:'one'}}); await flushPromises();
    await button(panel,'检查运行环境').trigger('click'); expect(invoke).toHaveBeenLastCalledWith('desktop_environment_check',{installationId:'one'});
    await panel.setProps({installationId:'two'}); resolve({installation_id:'one',services:[{name:'worker',cwd:'/one',executable:'/usr/bin/true'}]}); await flushPromises();
    expect(panel.text()).not.toContain('检查通过'); panel.unmount();
  });
  it('disables writes after a refresh error while retaining the previous buffer', async () => {
    invoke.mockResolvedValueOnce(baseline).mockRejectedValueOnce({code:'INVALID_ENVIRONMENT',message:'invalid file'});
    const panel = mount(EnvironmentSettings,{props:{apps}}); await flushPromises(); await panel.find('textarea').setValue('/custom');
    await button(panel,'重新读取').trigger('click'); await flushPromises(); expect(panel.find('textarea').element.value).toBe('/custom');
    expect(button(panel,'保存 PATH').attributes('disabled')).toBeDefined(); expect(button(panel,'从 shell 采集').attributes('disabled')).toBeDefined(); panel.unmount();
  });
});
describe('retention and management boundaries', () => {
  it('accepts inclusive limits and rejects missing, fractional and out-of-range numbers', () => {
    expect(validRetention(1,10)).toBe(true); expect(validRetention(365,10240)).toBe(true);
    for (const [days,mib] of [['',100],[1,''],[0,100],[366,100],[1.5,100],[7,9],[7,10241],[7,10.5],[NaN,100]]) expect(validRetention(days,mib)).toBe(false);
  });
  it('blocks invalid input, preserves a failed save, then publishes a complete valid setting', async () => {
    invoke.mockResolvedValueOnce(retention).mockRejectedValueOnce({code:'IO_ERROR',message:'write failed'}).mockResolvedValueOnce({schema_version:1,days:365,max_mib:10240});
    const panel = mount(RetentionSettings); await flushPromises();
    await panel.find('[aria-label="日志保留天数"]').setValue(''); expect(button(panel,'保存日志').attributes('disabled')).toBeDefined();
    await panel.find('[aria-label="日志保留天数"]').setValue('365'); await panel.find('[aria-label="每个应用日志容量 MiB"]').setValue('10240');
    await button(panel,'保存日志').trigger('click'); await flushPromises(); expect(panel.text()).toContain('IO_ERROR'); expect(panel.find('input').element.value).toBe('365');
    await button(panel,'保存日志').trigger('click'); await flushPromises(); expect(invoke).toHaveBeenLastCalledWith('desktop_retention_save',{days:365,maxMib:10240});
    expect(panel.text()).toContain('已保存，下次启动'); expect(button(panel,'保存日志').attributes('disabled')).toBeDefined(); panel.unmount();
  });
  it('lazily reads visited pages and keeps environment edits while navigating', async () => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'desktop_environment_read' ? baseline : retention));
    const panel = mount(ManagementPanel,{props:{theme:'light',saving:false,apps}}); await flushPromises(); expect(invoke).not.toHaveBeenCalled();
    await button(panel,'运行环境').trigger('click'); await flushPromises(); await panel.find('textarea').setValue('/draft');
    await button(panel,'日志保留').trigger('click'); await flushPromises(); await button(panel,'运行环境').trigger('click');
    expect(panel.find('textarea').element.value).toBe('/draft'); expect(invoke.mock.calls.map(c => c[0])).toEqual(['desktop_environment_read','desktop_retention_read']); panel.unmount();
  });
  it('keeps retention available when the platform cannot supply environment settings', async () => {
    invoke.mockRejectedValueOnce({code:'UNSUPPORTED_PLATFORM',message:'unsupported'}).mockResolvedValueOnce(retention);
    const panel = mount(ManagementPanel,{props:{theme:'light',saving:false,apps,section:'environment'}}); await flushPromises(); expect(panel.text()).toContain('UNSUPPORTED_PLATFORM');
    await button(panel,'日志保留').trigger('click'); await flushPromises(); expect(panel.find('input').element.value).toBe('7'); panel.unmount();
  });
});
