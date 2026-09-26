// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
const { invoke } = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke}));
import ImportPanel from './components/ImportPanel.vue';
import ManifestEditor from './components/ManifestEditor.vue';
import type { AppEntry } from './model';
const draft = () => ({root:'/project',candidates:[{path:'/project',human_label:'App',selected:true,manifest:{services:[{name:'web',command:['echo','<script>text</script>']}]},issues:[],notes:[]}],groups:[],issues:[]});
const app = (): AppEntry => ({installation_id:'app',project_id:'project',label:'App',path:'/project',registration_state:'REGISTERED',requires_confirmation:false,runtime:null});
const source = {revision:1,base_sha256:'base',text:'original',manifest_path:'/project/apphost.toml'};
const review = { ...source, before_text:'original',after_text:'edited',launch_sha256:'launch',previous_confirmation:{services:[]},warnings:[] };
const button = (panel: ReturnType<typeof mount>, text: string) => panel.findAll('button').find(b => b.text().includes(text))!;
beforeEach(() => invoke.mockReset());
describe('desktop registration confirmation', () => {
  it('cancelled native selection closes without importing', async () => {
    invoke.mockResolvedValue(null);
    const panel = mount(ImportPanel); await flushPromises();
    expect(panel.emitted('close')).toHaveLength(1); expect(invoke.mock.calls.map(c => c[0])).toEqual(['desktop_scan']); panel.unmount();
  });
  it('requires explicit selection, escapes declarations, and disables retry after partial failure', async () => {
    invoke.mockResolvedValueOnce({token:'scan',draft:draft()}).mockRejectedValueOnce({code:'IO_ERROR',message:'failed',details:{completed:[{installation_id:'done'}]}});
    const panel = mount(ImportPanel); await flushPromises();
    expect(button(panel,'确认并登记').attributes('disabled')).toBeDefined();
    await panel.find('input').setValue(true);
    expect(panel.find('script').exists()).toBe(false); expect(panel.text()).toContain('<script>text</script>');
    await button(panel,'确认并登记').trigger('click'); await flushPromises();
    expect(panel.text()).toContain('已登记 1 个应用'); expect(panel.emitted('changed')).toHaveLength(1);
    expect(button(panel,'确认并登记').attributes('disabled')).toBeDefined();
    expect(invoke).toHaveBeenCalledWith('desktop_import',{token:'scan',selected:[0]}); expect(invoke.mock.calls.filter(c => c[0] === 'desktop_import')).toHaveLength(1); panel.unmount();
  });
  it('requires each group decision before import', async () => {
    invoke.mockResolvedValueOnce({token:'scan',draft:{...draft(),groups:[{members:['/project/a','/project/b'],candidate:{path:'/project'}}]}}).mockResolvedValueOnce({token:'grouped',draft:draft()});
    const panel = mount(ImportPanel); await flushPromises(); expect(panel.find('input').attributes('disabled')).toBeDefined();
    expect(button(panel,'确认并登记').attributes('disabled')).toBeDefined();
    await button(panel,'分别登记').trigger('click'); await flushPromises(); await panel.find('input').setValue(true); expect(button(panel,'确认并登记').attributes('disabled')).toBeUndefined(); panel.unmount();
  });
});
describe('manifest editor boundaries', () => {
  it('invalidates a reviewed declaration after further edits', async () => {
    invoke.mockResolvedValueOnce(source).mockResolvedValueOnce(review);
    const panel = mount(ManifestEditor,{props:{app:app(),live:false}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click'); await flushPromises();
    expect(button(panel,'确认并保存')).toBeDefined();
    await panel.find('textarea').setValue('edited again');
    expect(button(panel,'确认并保存')).toBeUndefined(); expect(panel.emitted('dirty')?.at(-1)).toEqual([true]); panel.unmount();
  });
  it('preserves the buffer when disk changes before review', async () => {
    invoke.mockResolvedValueOnce(source).mockResolvedValueOnce({...review,base_sha256:'external'});
    const panel = mount(ManifestEditor,{props:{app:app(),live:false}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click'); await flushPromises();
    expect(panel.text()).toContain('EDIT_CONFLICT'); expect(panel.find('textarea').element.value).toBe('edited');
    expect(button(panel,'确认并保存')).toBeUndefined(); panel.unmount();
  });
  it('never treats an accepted operation as saved, and preserves edits on stop failure', async () => {
    invoke.mockResolvedValueOnce(source).mockResolvedValueOnce(review).mockResolvedValueOnce({operation_id:'save',operation_done:false});
    const item = app(); item.runtime = {installation_id:'app',instance_id:'instance',operation_id:'old',status:{state:'RUNNING',failure:null},services:[],updated_at_ms:1};
    const panel = mount(ManifestEditor,{props:{app:item,live:true}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click'); await flushPromises();
    await button(panel,'确认并保存').trigger('click'); await flushPromises();
    expect(panel.text()).toContain('等待后台'); expect(panel.text()).not.toContain('已保存并确认');
    await panel.setProps({live:false}); expect(panel.text()).toContain('结果尚未确认');
    await panel.setProps({live:true,app:{...item,runtime:{...item.runtime!,operation_id:'save',operation_done:true,operation_error:{code:'STOP_TIMEOUT',message:'stop failed'}}}}); await flushPromises();
    expect(panel.text()).toContain('STOP_TIMEOUT'); expect(panel.find('textarea').element.value).toBe('edited');
    expect(invoke.mock.calls.filter(c => c[0] === 'desktop_manifest_save')).toHaveLength(1); panel.unmount();
  });
  it('keeps text after a save conflict and does not retry automatically', async () => {
    invoke.mockResolvedValueOnce(source).mockResolvedValueOnce(review).mockRejectedValueOnce({code:'EDIT_CONFLICT',message:'external write'});
    const panel = mount(ManifestEditor,{props:{app:app(),live:false}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click'); await flushPromises();
    await button(panel,'确认并保存').trigger('click'); await flushPromises();
    expect(panel.find('textarea').element.value).toBe('edited'); expect(panel.text()).toContain('EDIT_CONFLICT');
    expect(button(panel,'确认并保存')).toBeUndefined(); expect(panel.emitted('changed')).toHaveLength(1); panel.unmount();
  });
});

describe('late responses and completion', () => {
  it('ignores a review response for a buffer that changed meanwhile', async () => {
    let resolve!: (value: unknown) => void;
    invoke.mockResolvedValueOnce(source).mockImplementationOnce(() => new Promise(done => {resolve = done;}));
    const panel = mount(ManifestEditor,{props:{app:app(),live:false}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click');
    await panel.find('textarea').setValue('new buffer'); resolve(review); await flushPromises();
    expect(button(panel,'确认并保存')).toBeUndefined(); expect(panel.find('textarea').element.value).toBe('new buffer'); panel.unmount();
  });
  it('reads the saved source only after matching completion and clears stale success on further edits', async () => {
    invoke.mockResolvedValueOnce(source).mockResolvedValueOnce(review).mockResolvedValueOnce({operation_id:'save',operation_done:false}).mockResolvedValueOnce({...source,revision:2,text:'edited'});
    const item = app(); item.runtime = {installation_id:'app',instance_id:'instance',operation_id:'old',status:{state:'RUNNING',failure:null},services:[],updated_at_ms:1};
    const panel = mount(ManifestEditor,{props:{app:item,live:true}}); await flushPromises();
    await panel.find('textarea').setValue('edited'); await button(panel,'复核').trigger('click'); await flushPromises();
    await button(panel,'确认并保存').trigger('click'); await flushPromises();
    await panel.setProps({app:{...item,runtime:{...item.runtime!,operation_done:true}}}); await flushPromises();
    expect(invoke.mock.calls.filter(c => c[0] === 'desktop_manifest_read')).toHaveLength(1);
    await panel.setProps({app:{...item,runtime:{...item.runtime!,operation_id:'save',operation_done:true,status:{state:'STOPPED',failure:null}}}}); await flushPromises();
    expect(panel.text()).toContain('已保存并确认'); expect(panel.emitted('dirty')?.at(-1)).toEqual([false]);
    await panel.find('textarea').setValue('another edit'); expect(panel.text()).not.toContain('已保存并确认'); panel.unmount();
  });
});
