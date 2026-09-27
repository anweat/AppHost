import { tileState, type AppEntry } from './model';
import type { Preferences } from './preferences';

export type SearchAction = 'primary'|'details'|'start'|'stop'|'restart'|'logs'|'failure'|'manifest';

const verbs: Record<string, SearchAction> = {
  打开:'primary', open:'primary', 启动:'start', start:'start', run:'start',
  停止:'stop', 关闭:'stop', stop:'stop', kill:'stop',
  重启:'restart', restart:'restart', reload:'restart',
  日志:'logs', logs:'logs', log:'logs', 详情:'details', details:'details', info:'details',
  失败:'failure', failure:'failure', 声明:'manifest', manifest:'manifest',
};
const stateAliases: Record<string, string[]> = {
  running:['running','run','运行','运行中'],
  stopped:['stopped','stop','停止','已停止'],
  failed:['failed','fail','error','失败','异常'],
  review:['review','待确认','确认'],
  missing:['missing','缺失','源码缺失'],
  starting:['starting','启动中','正在启动'],
  stopping:['stopping','停止中','正在停止'],
};

function stateMatches(app: AppEntry, value: string) {
  if (!value) return true;
  const entries = Object.entries(stateAliases);
  const state = (entries.find(([, aliases]) => aliases.includes(value))
    ?? entries.find(([, aliases]) => aliases.some(alias => alias.startsWith(value))))?.[0];
  if (state === 'running') return app.runtime?.status.state === 'RUNNING'
    || app.runtime?.status.state === 'STARTING' || !!app.runtime?.services.some(service => service.alive);
  if (state === 'failed') return ['failed', 'missing'].includes(tileState(app));
  return state === tileState(app);
}

// Lower scores are better. A substring always beats a subsequence in the same field.
function fuzzy(haystack: string, needle: string) {
  if (!needle) return 0;
  const hay = haystack.toLocaleLowerCase();
  if (hay.startsWith(needle)) return 0;
  const at = hay.indexOf(needle);
  if (at >= 0) return 1 + at / (100 + at);
  let index = 0;
  for (const character of hay) {
    if (character === needle[index]) index++;
    if (index === needle.length) return 3;
  }
  return -1;
}

export function searchApps(apps: AppEntry[], query: string, preferences: Preferences) {
  let onlyActions = false, state = '', group = '';
  const words: string[] = [];
  for (let token of query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)) {
    if (token.startsWith('>')) {
      onlyActions = true;
      token = token.slice(1);
      if (!token) continue;
    }
    const filter = token.match(/^(is|in|状态|组):(.*)$/);
    if (filter) {
      if (filter[1] === 'is' || filter[1] === '状态') state = filter[2]!;
      else group = filter[2]!;
    } else words.push(token);
  }
  const first = words[0] ?? '';
  // A bare word remains searchable as an app name; explicit > also accepts a bare verb.
  const action = words.length > 1 || onlyActions
    ? verbs[first] ?? Object.entries(verbs).find(([verb]) => first.length >= Math.min(2, verb.length) && verb.startsWith(first))?.[1]
    : undefined;
  if (action) words.shift();
  const text = words.join(' ');
  const matches = apps.flatMap((app, index) => {
    if (!stateMatches(app, state)) return [];
    const custom = preferences.groups.find(candidate => candidate.members.includes(app.installation_id));
    const groupName = custom?.name || app.group_label || '应用';
    const groupPath = custom ? '' : app.group || app.path.slice(0, app.path.lastIndexOf('/'));
    if (group && ![groupName, groupPath].some(value => value.toLocaleLowerCase().includes(group))) return [];
    const fields = [app.label, groupName, ...(preferences.search.match_path ? [app.path] : [])];
    const scores = fields.map((field, fieldIndex) => {
      const score = fuzzy(field, text);
      return score < 0 ? Infinity : score + fieldIndex * .5;
    });
    const score = Math.min(...scores);
    return Number.isFinite(score) ? [{app, index, score}] : [];
  });
  return {
    apps: matches.map(match => match.app),
    ranked: [...matches].sort((a, b) => a.score - b.score || a.index - b.index).map(match => match.app),
    text, action, onlyActions,
    showActions: onlyActions || (preferences.search.show_actions && !!action),
    actions: action ? [action] : ['start','stop','restart','logs','details'] as SearchAction[],
  };
}
