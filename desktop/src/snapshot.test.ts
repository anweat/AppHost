// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import AppTile from './components/AppTile.vue';
import { hasLiveServices, stateLabel, type AppEntry, type Snapshot } from './model';
import { useSnapshot } from './useSnapshot';
const app: AppEntry = {
  installation_id: 'installation', project_id: 'project', label: '<img src=x onerror=alert(1)>',
  path: '/project', registration_state: 'REGISTERED', requires_confirmation: false,
  runtime: { installation_id: 'installation', instance_id: 'instance', updated_at_ms: 12,
    status: { state: 'FAILED', failure: 'SERVICE_EXITED' },
    services: [{ name: 'web', port: 3000, alive: true, ready: false }] },
};
const snapshot: Snapshot = { apps: [app], live: true, read_at_ms: 13, runtime_error: null };
describe('desktop observations', () => {
  it('preserves last observation but removes live authority after refresh failure', async () => {
    const read = vi.fn().mockResolvedValueOnce(snapshot).mockRejectedValueOnce({code:'DAEMON_UNAVAILABLE',message:'offline'});
    const view = useSnapshot(read);
    await view.refresh();
    expect(view.displayed.value?.live).toBe(true);
    await view.refresh();
    expect(view.displayed.value?.apps[0]?.runtime?.status.state).toBe('FAILED');
    expect(view.displayed.value?.read_at_ms).toBe(13);
    expect(view.displayed.value?.live).toBe(false);
    expect(view.displayed.value?.runtime_error?.code).toBe('DAEMON_UNAVAILABLE');
  });
  it('coalesces refresh clicks so a stale response cannot replace a newer one', async () => {
    let resolve!: (value: Snapshot) => void;
    const read = vi.fn(() => new Promise<Snapshot>(done => { resolve = done; }));
    const view = useSnapshot(read);
    const first = view.refresh();
    await view.refresh();
    expect(read).toHaveBeenCalledTimes(1);
    resolve(snapshot); await first;
    expect(view.loading.value).toBe(false);
  });
  it('never paints historical live services as currently alive or errors as stopped', () => {
    const historical = { ...snapshot, live: false };
    expect(hasLiveServices(app, historical)).toBe(false);
    expect(stateLabel(app, historical)).toContain('上次记录');
    expect(stateLabel({ ...app, runtime: null }, { ...historical, runtime_error: { code:'PROTOCOL_ERROR', message:'bad' } })).toBe('状态未知');
  });
  it('renders project text without HTML and exposes details to keyboard users', async () => {
    const wrapper = mount(AppTile, { props: { app, snapshot } });
    expect(wrapper.find('img').exists()).toBe(false);
    expect(wrapper.text()).toContain(app.label);
    expect(wrapper.attributes('data-live')).toBe('true');
    await wrapper.find('button').trigger('keydown', { key:'F10', shiftKey:true });
    expect(wrapper.emitted('details')).toEqual([['installation']]);
    await wrapper.setProps({ snapshot: { ...snapshot, live:false } });
    expect(wrapper.attributes('data-live')).toBe('false');
  });
});
