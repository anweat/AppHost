'use strict';
// Design data only: no daemon transport, process execution or filesystem writes.
// Groups start from the scanned parent directory. Users may add their own groups; those
// are appearance data (ui.json), never registry facts, and never change an app's source.
const groupsSeed=[{key:'work',name:'工作',path:'~/Projects/work'},{key:'lab',name:'实验',path:'~/Lab'},{key:'tools',name:'工具',path:'~/Tools'}];
// Identity hue is derived from project_id (see identityHue), never picked per app.
// Most real projects have no findable icon, so most samples show a monogram.
// lastUsed is minutes ago and only feeds the recent group; it never reorders other groups.
const svc=(name,port,state='stopped')=>({name,port,state});
const projectIdAt=n=>`00000000-0000-4000-8000-${String(n).padStart(12,'0')}`;
const seed = [
  {id:'paper',name:'Paper Reader',branch:'main',icon:'book',state:'FAILED',sim:'port-external',port:5173,group:'work',lastUsed:3,services:[svc('frontend',5173,'failed'),svc('backend',8000)],
    history:{lastSuccess:'昨天 18:40',changes:[]},
    failure:{code:'PORT_IN_USE',phase:'preflight',service:'frontend',port:5173,process:'node · PID 18273',processSince:'今天 09:12',count:1,at:Date.now()-3*60000}},
  {id:'vision',name:'Vision Lab',branch:'feat / camera',icon:'camera',state:'STOPPED',review:true,port:8080,group:'lab',lastUsed:180,services:[svc('web',8080)]},
  {id:'canvas',name:'Local Canvas',branch:'main',icon:'canvas',state:'RUNNING',port:5174,group:'work',lastUsed:0,services:[svc('web',5174,'running')]},
  {id:'whisper',name:'Whisper Studio',branch:'main',command:'uv run python app.py',sim:'dependency',state:'STOPPED',port:7860,group:'lab',lastUsed:2880,services:[svc('web',7860)],
    history:{lastSuccess:'2 天前',changes:[]}},
  {id:'notes',name:'Notes',branch:'main',icon:'note',state:'RUNNING',port:3001,group:'work',lastUsed:12,services:[svc('web',3001,'running'),svc('api',3002,'running')]},
  {id:'assets',name:'Asset Board',branch:'main',state:'RUNNING',port:3000,group:'work',lastUsed:40,services:[svc('web',3000,'running')]},
  {id:'focus',name:'Focus Timer',branch:'main',icon:'clock',command:'npm run serve',sim:'command',state:'STOPPED',port:null,group:'tools',lastUsed:43200,services:[svc('timer',null)],
    history:{lastSuccess:'30 天前',changes:[{label:'启动声明',text:'command 从 npm run dev 改为 npm run serve',by:'agent 修改，2 天前由你确认'}]}},
  {id:'atlas',name:'Atlas Control',branch:'dev',sim:'timeout',startupTimeout:8000,state:'STOPPED',port:8081,group:'tools',lastUsed:10080,services:[svc('web',8081),svc('worker',null)],
    history:{lastSuccess:'7 天前',changes:[{label:'分支',text:'main → dev'}]}},
  // A second worktree of Paper Reader: same project, so same color; the branch tells them apart.
  {id:'paper-ocr',name:'Paper Reader',branch:'feat / ocr',icon:'book',sim:'early-exit',state:'STOPPED',port:5175,group:'work',lastUsed:4320,projectId:projectIdAt(1),services:[svc('frontend',5175),svc('backend',8010)],
    history:{lastSuccess:null,changes:[]}},
  {id:'ledger',name:'家庭账本',branch:'main',sim:'runtime',state:'STOPPED',port:5180,group:'tools',lastUsed:20160,services:[svc('web',5180)],
    history:{lastSuccess:'5 个月前',changes:[{label:'运行环境',text:'上次成功时 node 来自 ~/.nvm/versions/node/v20.11.1/bin，这个目录现在不在共享 PATH 中'}]}},
  {id:'scrape',name:'Scrape Kit',branch:'main',state:'STOPPED',problem:'source',port:9000,group:'lab',lastUsed:86400,services:[svc('api',9000)]},
  // Declares the same fixed port as Notes, so it cannot run while Notes does.
  {id:'blog',name:'Blog Draft',branch:'main',state:'STOPPED',port:3001,group:'work',lastUsed:1440,services:[svc('web',3001)],
    history:{lastSuccess:'昨天',changes:[]}},
  {id:'playground',name:'LLM Playground',branch:'main',sim:'slow',state:'STOPPED',port:7000,group:'lab',lastUsed:90,services:[svc('web',7000),svc('api',7001)]}
].map((app,index)=>({...app,home:app.group,path:`${groupsSeed.find(g=>g.key===app.group).path}/${app.id}`,projectId:app.projectId||projectIdAt(index+1)}));
const defaultPreferences=()=>({login:false,tray:false,pathSaved:false,theme:'system',enterAction:'primary',searchPaths:false,searchActions:true});
const SETTING_LABEL={login:'登录自启',tray:'托盘常驻',theme:'外观',enterAction:'回车默认操作',searchPaths:'搜索项目路径',searchActions:'搜索中显示操作'};
const main=document.querySelector('#main'),detail=document.querySelector('#detail'),modal=document.querySelector('#modal'),searchInput=document.querySelector('#search');
let apps=structuredClone(seed), groups=structuredClone(groupsSeed), view='home', section='apps', query='', connected=true, paused=false, portBusy=true;
let selected=null, detailTab='overview', events=[], nextId=0, groupSeq=0;
let collapsed=new Set(), lens=null, builtKey='';
const logReads=new Map();
let offlineApps=[], preferences=defaultPreferences(), orphanDemo=false;
const displayedApps = () => connected ? apps : offlineApps;
const active = app => ['RUNNING','STARTING','STOPPING'].includes(app.state);
// Two kinds of "needs you": failures (red) and things waiting on a human decision
// (neutral). Needing review is not an error, so it never borrows the failure color.
const failedNow = app => app.state==='FAILED' && !app.review && app.problem!=='source';
const needsReview = app => app.review || app.problem==='source';
const attention = app => failedNow(app) || needsReview(app);
const hasLive = app => app.state==='STARTING' || liveServices(app).length>0;
const stateLabel = app => app.review?'待确认声明':app.problem==='source'?'源码缺失':({RUNNING:'运行中',STARTING:'正在启动',STOPPING:'正在停止',FAILED:'启动失败',STOPPED:'已停止'}[app.state]);
const reducedMotion=matchMedia('(prefers-reduced-motion: reduce)');

/* ── Model helpers ─────────────────────────────────────────────── */
// Imported apps arrive without services or a group: derive them from the declaration and directory.
function normalize(){
  for(const app of apps){
    if(!app.services)app.services=[svc('web',app.port??null,app.state==='RUNNING'?'running':'stopped')];
    if(app.hue===undefined)app.hue=identityHue(app.projectId);
    if(!app.home)app.home=(app.path||'').split('/').slice(0,-1).join('/')||'other';
    if(!app.group)app.group=app.home;
    if(!groups.some(g=>g.key===app.group))groups.push({key:app.group,name:app.group.split('/').pop()||'其他',path:app.group});
  }
}
function groupOf(app){return app.group||app.home||'other';}
function groupInfo(key){return groups.find(g=>g.key===key)||{key,name:key.split('/').pop()||'其他',path:key};}
const vtName=key=>`g-${key.replace(/[^a-zA-Z0-9_-]/g,'-')}`;
const setServices=(app,state)=>{for(const s of app.services||[])s.state=state;};

/* ── Search index ──────────────────────────────────────────────── */
// Query language: free text, `is:<state>`, `in:<group>`, and a leading `>` for actions only.
const STATE_ALIASES={running:['running','run','运行','运行中'],stopped:['stopped','stop','停止','已停止'],failed:['failed','fail','error','失败','异常'],review:['review','待确认','确认']};
function parseQuery(raw){
  const q={text:'',is:null,in:null,command:false},words=[];
  for(const token of raw.trim().split(/\s+/).filter(Boolean)){
    if(token.startsWith('>')){q.command=true;if(token.length>1)words.push(token.slice(1).toLowerCase());continue;}
    const filter=token.match(/^(is|in|状态|组):(.*)$/i);
    if(filter){q[/^(is|状态)$/i.test(filter[1])?'is':'in']=filter[2].toLowerCase();continue;}
    words.push(token.toLowerCase());
  }
  q.text=words.join(' ');return q;
}
function stateMatches(app,value){
  if(!value)return true;
  const key=Object.keys(STATE_ALIASES).find(k=>STATE_ALIASES[k].some(alias=>alias.startsWith(value)));
  if(!key)return false;
  return key==='running'?hasLive(app):key==='failed'?['failed','missing'].includes(tileState(app)):tileState(app)===key;
}
function groupMatches(app,value){if(!value)return true;const group=groupInfo(groupOf(app));return group.name.toLowerCase().includes(value)||(group.path||'').toLowerCase().includes(value);}
// Lower is better: prefix < substring < subsequence. -1 means no match.
function fuzzy(hay,needle){
  if(!needle)return 0;hay=String(hay).toLowerCase();
  if(hay.startsWith(needle))return 0;
  const at=hay.indexOf(needle);if(at>=0)return 1+at/100;
  let j=0;for(const c of hay){if(c===needle[j])j++;if(j===needle.length)return 3;}
  return -1;
}
function matchApp(app,q){
  if(!stateMatches(app,q.is)||!groupMatches(app,q.in))return -1;
  if(!q.text)return 0;
  const fields=[app.name,app.branch,groupInfo(groupOf(app)).name,...(preferences.searchPaths?[app.path||'']:[])];
  const scores=fields.map((field,i)=>{const s=fuzzy(field,q.text);return s<0?-1:s+i*.5;}).filter(s=>s>=0);
  return scores.length?Math.min(...scores):-1;
}
function highlight(text,needle){
  const at=needle?text.toLowerCase().indexOf(needle):-1;
  return at<0?escape(text):`${escape(text.slice(0,at))}<mark>${escape(text.slice(at,at+needle.length))}</mark>${escape(text.slice(at+needle.length))}`;
}
const VERBS=[
  {keys:['打开','启动','open','start','run'],act:'primary'},
  {keys:['停止','关闭','stop','kill'],act:'stop'},
  {keys:['重启','restart','reload'],act:'restart'},
  {keys:['日志','logs','log'],act:'logs'},
  {keys:['详情','details','info'],act:'details'}
];
function appActions(app){
  const alive=hasLive(app);
  return [
    {label:primaryLabel(app),icon:primaryIcon(app),act:'primary',disabled:['starting','stopping'].includes(tileState(app))},
    {label:'停止',icon:'stop',act:'stop',disabled:!alive},
    {label:'完整重启',icon:'restart',act:'restart',disabled:!alive||app.review},
    {label:'查看日志',icon:'logs',act:'logs'},
    {label:'详情',icon:'info',act:'details'}
  ];
}
function runAppAction(app,act){
  ({primary:()=>primary(app),stop:()=>{if(requireConnection())stopApp(app);},restart:()=>restartApp(app),logs:()=>openDetail(app,'logs'),details:()=>openDetail(app)})[act]();
}
function globalCommands(){
  const running=displayedApps().filter(hasLive).length;
  return [
    {label:'添加应用',icon:'plus',keys:['添加','导入','新增','add','import'],run:()=>handleFlow('import')},
    {label:'新建分组',icon:'newgroup',keys:['新建分组','分组','group','new group'],run:()=>promptGroup({})},
    {label:`停止全部运行中的应用（${running}）`,icon:'stop',keys:['全部停止','停止全部','stop all'],run:stopAll,disabled:!running},
    paused?{label:'恢复自动唤醒',icon:'play',keys:['恢复','唤醒','resume','wake'],run:resumeWake}:{label:'暂停自动唤醒',icon:'pause',keys:['暂停','唤醒','pause','wake'],run:pauseWake},
    {label:'折叠全部分组',icon:'chevron',keys:['折叠','collapse'],run:()=>setAllCollapsed(true)},
    {label:'展开全部分组',icon:'expand',keys:['展开','expand'],run:()=>setAllCollapsed(false)},
    {label:'切换深色 / 浅色',icon:'moon',keys:['深色','浅色','主题','外观','dark','light','theme'],run:toggleTheme},
    {label:'应用管理',icon:'grid',keys:['管理','manage','应用管理'],run:()=>openSection('apps')},
    {label:'操作记录',icon:'activity',keys:['记录','journal','activity','历史'],run:()=>openSection('journal')},
    {label:'环境与设置',icon:'sliders',keys:['设置','环境','settings','path','偏好'],run:()=>openSection('settings')}
  ];
}
const visual=name=>`<span class="pal-icon">${icon(name)}</span>`;
const appVisual=app=>`<span class="pal-badge" style="${idStyle(app)}" data-state="${tileState(app)}">${glyph(app)}</span>`;
function searchProvider(raw){
  const q=parseQuery(raw),display=displayedApps();
  const appItem=app=>{
    const primaryItem={run:()=>primary(app),label:primaryLabel(app)},detailsItem={run:()=>openDetail(app),label:'详情'};
    const [first,second]=preferences.enterAction==='details'?[detailsItem,primaryItem]:[primaryItem,detailsItem];
    return {visual:appVisual(app),label:highlight(app.name,q.text),sub:`${groupInfo(groupOf(app)).name} · ${STATE_LABEL[tileState(app)]}`,hint:first.label,run:first.run,alt:second.run};
  };
  // An empty query shows the recent apps and the syntax, never an empty panel.
  if(!raw.trim()){
    const recent=[...display].filter(a=>a.lastUsed!==undefined).sort((a,b)=>a.lastUsed-b.lastUsed).slice(0,5);
    return {sections:[{title:'最近使用',items:recent.map(appItem)},{title:'操作',items:preferences.searchActions?globalCommands().slice(0,3).filter(c=>!c.disabled).map(c=>({visual:visual(c.icon),label:escape(c.label),run:c.run})):[]}]};
  }
  // "停止 notes" style: a verb followed by an app narrows actions to that app.
  const [head,...rest]=q.text.split(' ');
  const verb=rest.length?VERBS.find(v=>v.keys.some(k=>k.startsWith(head)&&head.length>=Math.min(2,k.length))):null;
  const targetText=verb?rest.join(' '):q.text;
  const ranked=display.map(app=>[app,matchApp(app,{...q,text:targetText})]).filter(([,s])=>s>=0).sort((a,b)=>a[1]-b[1]).map(([app])=>app);
  const actions=[];
  if(preferences.searchActions||q.command){
    if(verb)for(const app of ranked.slice(0,4)){const action=appActions(app).find(a=>a.act===verb.act);if(!action.disabled)actions.push({visual:visual(action.icon),label:`${escape(action.label)} <b>${highlight(app.name,targetText)}</b>`,run:()=>runAppAction(app,verb.act)});}
    // A single clear match indexes that app's own operations.
    if(!verb&&!q.command&&ranked.length===1)for(const action of appActions(ranked[0]).filter(a=>!a.disabled&&a.act!=='details'))actions.push({visual:visual(action.icon),label:`${escape(action.label)} <b>${escape(ranked[0].name)}</b>`,run:()=>runAppAction(ranked[0],action.act)});
    // Filters alone (is:/in:) are about apps; short fragments only match commands in > mode.
    const commandText=q.command||q.text.length>=2;
    if(!verb&&(q.command||(q.text&&commandText)))for(const command of globalCommands())if(!command.disabled&&(!q.text||[command.label,...command.keys].some(k=>{const s=fuzzy(k,q.text);return s>=0&&s<3;})))actions.push({visual:visual(command.icon),label:highlight(command.label,q.text),run:command.run});
  }
  return {sections:[{title:'应用',items:q.command||verb?[]:ranked.slice(0,6).map(appItem)},{title:'操作',items:actions.slice(0,7)}]};
}
palette.setProvider(searchProvider);

/* ── Rendering ─────────────────────────────────────────────────── */
function render() {
  const focused=document.activeElement;
  const restore=main.contains(focused)&&focused.matches('button[data-action]');
  renderContent();
  // Replacing a view must not drop keyboard focus to the document body.
  if(restore&&!main.contains(focused)){const replacement=[...main.querySelectorAll('button[data-action]')].find(button=>['action','id','filter','section','group'].every(key=>button.dataset[key]===focused.dataset[key]));if(replacement)replacement.focus();else{main.tabIndex=-1;main.focus();}}
  palette.refresh();
}
// Layout changes (fold, search, regroup) morph tiles to their new places when supported.
function morph(change){
  if(!document.startViewTransition||reducedMotion.matches||!main.querySelector('.board')){change();return;}
  // A rapid second change skips the running transition; that rejection is expected.
  hideTip();const transition=document.startViewTransition(change);transition.ready.catch(()=>{});transition.finished.catch(()=>{});
}
function renderContent() {
  normalize();
  const display=displayedApps();
  document.querySelector('#connection-dot').classList.toggle('off',!connected);
  document.querySelector('.brand').dataset.tip=connected?'AppHost · 后台已连接':'AppHost · 后台未连接';
  document.querySelector('[data-action=manage]').toggleAttribute('aria-current',view==='manage');
  renderStatus(display);
  if(view==='manage'){builtKey='';renderManagement();return;}
  const q=parseQuery(query),filtering=!q.command&&!!(q.text||q.is||q.in);
  const matches=filtering?display.filter(a=>matchApp(a,q)>=0):display;
  // Rebuild only when the structure changes. Lifecycle updates patch tiles in place,
  // so state transitions animate and positions (including the recent group) never jump.
  const key=JSON.stringify([connected,filtering,display.map(a=>[a.id,a.name,groupOf(a)]),groups.map(g=>[g.key,g.name]),matches.map(a=>a.id)]);
  if(key===builtKey&&main.querySelector('.board')){display.forEach(syncTiles);syncGroups(display);return;}
  builtKey=key;
  if(!display.length){
    main.innerHTML=`<div class="board first-run"><h1 class="sr-only">添加第一个应用</h1><div class="canvas"><section class="group"><div class="group-head"></div><div class="group-grid"><button class="tile-add hero" data-action="import" aria-label="添加第一个应用">${icon('plus')}<span>添加应用</span></button></div></section></div></div>`;
    layoutRows();return;
  }
  if(!matches.length){main.innerHTML=`<div class="empty">${icon('search')}<h2>没有匹配的应用</h2><button data-action="clear-search">清除搜索</button></div>`;return;}
  main.innerHTML=`<div class="board" data-lens="${lens||''}" data-offline="${!connected}"><h1 class="sr-only">应用</h1><div class="canvas">${filtering?'':recentGroup(display)}${groupSections(matches,display,filtering)}${filtering?'':`<section class="group add-group" style="view-transition-name:g-add"><div class="group-head" aria-hidden="true"></div><div class="group-grid"><button class="tile-add" data-action="add-menu" aria-label="添加应用或新建分组" data-tip="添加应用 · 新建分组" data-tip-delay="300">${icon('plus')}</button><div class="drop-new" data-drop-new aria-hidden="true">${icon('newgroup')}</div></div></section>`}</div></div>`;
  if(!filtering)for(const key of [...collapsed])toggleGroup(key,true);
  layoutRows();syncGroups(display);syncEdges();syncSelection();callouts.relink();
}
const LENSES={running:{label:'运行中',dot:'run',test:hasLive},failed:{label:'启动失败',dot:'fail',test:failedNow},review:{label:'待确认或需重新关联',dot:'review',test:needsReview}};
// Status lives in the top bar as dots and numbers. It is a lens, never a sort. Stop All is
// a labeled, always-present bar button (trust contract): it never moves, and is disabled
// rather than hidden when nothing is running.
function renderStatus(display){
  const bar=document.querySelector('#bar-status');
  if(!bar.childElementCount)bar.innerHTML=`${Object.entries(LENSES).map(([name,info])=>`<button class="lens" data-action="lens" data-lens="${name}" aria-pressed="false" data-tip="${info.label} · 点击突出显示" data-tip-delay="300"><i class="chip-dot ${info.dot}" aria-hidden="true"></i><b data-count></b></button>`).join('')}`;
  const counts=Object.fromEntries(Object.entries(LENSES).map(([name,info])=>[name,display.filter(info.test).length]));
  for(const name of Object.keys(LENSES)){
    const button=bar.querySelector(`[data-lens="${name}"]`);
    button.querySelector('[data-count]').textContent=counts[name];
    button.setAttribute('aria-label',`${LENSES[name].label} ${counts[name]} 个，突出显示`);
    // Running stays visible at zero (it anchors Stop All); the "needs you" counts appear only when non-zero.
    button.hidden=name!=='running'&&!counts[name];
    if(!counts[name]&&lens===name)lens=null;
    button.setAttribute('aria-pressed',String(lens===name));
  }
  const stop=document.querySelector('#stop-all'),stopping=display.some(a=>a.state==='STOPPING');
  stop.disabled=!connected||!counts.running;
  stop.querySelector('span').textContent=stopping&&!counts.running?'正在停止':'全部停止';
  stop.setAttribute('aria-label',counts.running?`全部停止：停止 ${counts.running} 个运行中的应用并暂停自动唤醒`:'全部停止：当前没有运行中的应用');
  stop.dataset.tip=connected?(counts.running?`停止全部 ${counts.running} 个运行中的应用，并暂停自动唤醒`:'没有运行中的应用'):'后台未连接';stop.dataset.tipDelay='200';
  const board=main.querySelector('.board');if(board&&board.dataset.lens!==(lens||''))board.dataset.lens=lens||'';
}
const matchesLens=(app,name)=>LENSES[name]?LENSES[name].test(app):true;
function setLens(name){
  lens=name;renderStatus(displayedApps());
  // A lens reveals matches inside folded groups instead of hiding them.
  if(name)for(const group of main.querySelectorAll('.group[data-collapsed="true"]')){if(displayedApps().some(a=>groupOf(a)===group.dataset.group&&matchesLens(a,name)))toggleGroup(group.dataset.group,false);}
}
// Recency order is frozen when the group is built; a start in progress never slides tiles.
// With few apps the whole library is already on screen and "recent" would only repeat it,
// so the group appears once the library outgrows a glance, and stays short.
const RECENT_MIN_APPS=12,RECENT_MAX=6;
function recentGroup(display){
  if(display.length<RECENT_MIN_APPS)return '';
  const recent=[...display].filter(a=>a.lastUsed!==undefined).sort((a,b)=>a.lastUsed-b.lastUsed).slice(0,RECENT_MAX);
  if(!recent.length)return '';
  return `<section class="group recent" aria-label="最近使用" style="view-transition-name:g-recent"><div class="group-head"><span class="group-mark" data-tip="最近使用" data-tip-delay="200">${icon('clock')}</span></div><div class="group-grid small">${recent.map(app=>appTile(app,{size:'small'})).join('')}</div></section>`;
}
function groupSections(list,display,filtering){
  return groups.map(info=>{
    const members=list.filter(a=>groupOf(a)===info.key);
    // Directory groups exist only while they hold apps; user groups persist while empty.
    if(!members.length&&(!info.custom||filtering))return '';
    const closed=collapsed.has(info.key)&&!filtering&&members.length>0;
    const tip=info.custom?`${info.name} · 自定义分组`:`${info.name} · ${info.path}`;
    const k=escape(info.key),n=escape(info.name);
    return `<section class="group" data-group="${k}" data-collapsed="${closed}" aria-label="${n}" style="view-transition-name:${vtName(info.key)}"><div class="group-head"><button class="group-title" data-action="toggle-group" data-group="${k}" aria-expanded="${!closed}" data-tip="${escape(tip)}" data-tip-delay="500"><span class="group-name">${n}</span><span class="group-dots" data-group-dots></span><span class="chevron" aria-hidden="true">${icon('chevron')}</span></button><span class="group-tools"><button class="mini-btn" data-action="group-start" data-group="${k}" aria-label="启动 ${n} 中的应用" data-tip="全部启动" data-tip-delay="200">${icon('play')}</button><button class="mini-btn" data-action="group-stop" data-group="${k}" aria-label="停止 ${n} 中的应用" data-tip="全部停止" data-tip-delay="200">${icon('stop')}</button></span></div><div class="group-grid">${members.length?members.map(app=>appTile(app,{branch:display.filter(other=>other.name===app.name).length>1})).join(''):`<div class="drop-slot" aria-hidden="true">${icon('newgroup')}</div>`}</div><button class="folder" data-action="toggle-group" data-group="${k}" aria-label="展开 ${n}" data-tip="${n} · ${members.length} 个应用" data-tip-delay="200" tabindex="-1">${members.slice(0,4).map(folderCell).join('')}</button></section>`;
  }).join('');
}
function syncGroups(display){
  for(const group of main.querySelectorAll('.group[data-group]')){
    const members=display.filter(a=>groupOf(a)===group.dataset.group);
    group.querySelector('[data-group-dots]').innerHTML=`${members.some(failedNow)?'<i class="chip-dot fail"></i>':''}${members.some(needsReview)?'<i class="chip-dot review"></i>':''}${members.some(hasLive)?'<i class="chip-dot run"></i>':''}`;
    group.querySelector('[data-action=group-start]').disabled=!connected||!members.some(canStart);
    group.querySelector('[data-action=group-stop]').disabled=!connected||!members.some(hasLive);
  }
}
function toggleGroup(key,close=!collapsed.has(key)){
  const group=[...main.querySelectorAll('.group[data-group]')].find(g=>g.dataset.group===key);
  if(!group||!group.querySelector('.tile')){collapsed.delete(key);return;}
  if(close)collapsed.add(key);else collapsed.delete(key);
  const moveFocus=group.querySelector('.folder').contains(document.activeElement);
  group.dataset.collapsed=String(close);group.querySelector('.group-title').setAttribute('aria-expanded',String(!close));
  // Folded tiles leave the tab order; the folder tile takes their place.
  for(const button of group.querySelectorAll('.group-grid button'))button.tabIndex=close?-1:0;
  group.querySelector('.folder').tabIndex=close?0:-1;
  if(moveFocus)group.querySelector('.group-title').focus();
  syncEdges();
}
function setAllCollapsed(close){morph(()=>{for(const g of groups)if(displayedApps().some(a=>groupOf(a)===g.key))toggleGroup(g.key,close);});}
// Rows and tile size follow the available height, like a Start screen: at most three
// rows keep the composition horizontal, and tiles grow to use the height they are given.
function layoutRows(){
  const board=main.querySelector('.board');if(!board)return;
  const style=getComputedStyle(board),gap=parseFloat(style.getPropertyValue('--gap'));
  const free=board.clientHeight-parseFloat(style.paddingTop)-parseFloat(style.paddingBottom)-38;
  const narrow=innerWidth<700,min=narrow?96:104,max=narrow?120:148;
  const rows=Math.max(1,Math.min(3,Math.floor((free+gap)/(min+gap))));
  const tile=Math.max(min,Math.min(max,Math.floor((free-(rows-1)*gap)/rows)));
  board.style.setProperty('--rows',rows);board.style.setProperty('--tile',`${tile}px`);
}
function syncEdges(){
  const board=main.querySelector('.board');if(!board)return;
  board.toggleAttribute('data-at-start',board.scrollLeft<=2);
  board.toggleAttribute('data-at-end',board.scrollLeft+board.clientWidth>=board.scrollWidth-2);
}

function renderManagement() {
  const nav=[['apps','grid','应用管理'],['journal','activity','操作记录'],['settings','sliders','环境与设置']];
  let content='';
  if(section==='apps')content=`<div class="page-heading"><div><div class="eyebrow">MANAGE</div><h1>应用管理</h1><p class="subtitle">管理启动声明、项目路径和登记。</p></div><button class="primary" data-action="import">添加目录</button></div>${apps.length?displayedApps().map(a=>`<div class="app-row"><div class="app-icon" style="${idStyle(a)}">${glyph(a)}</div><div><strong>${escape(a.name)}</strong><small>${escape(a.path || `~/Projects/${a.id}`)} · ${escape(a.branch)} · ${escape(groupInfo(groupOf(a)).name)}</small></div><span class="state ${a.state.toLowerCase()}">${stateLabel(a)}</span><button data-action="details" data-id="${a.id}" aria-label="管理 ${escape(a.name)}">管理</button></div>`).join(''):'<p class="muted">暂无已登记应用。选择目录以添加。</p>'}`;
  if(section==='journal')content=`<div class="page-heading"><div><div class="eyebrow">ACTIVITY</div><h1>操作记录</h1><p class="subtitle">本次原型会话的操作，不包含真实运行记录。</p></div></div>${events.length?events.map(e=>`<div class="event"><time>${e.time}</time><span>${escape(e.text)}</span></div>`).join(''):'<div class="empty"><h2>还没有操作</h2><p>启动、停止、导入和声明确认会显示在这里。</p></div>'}`;
  if(section==='settings')content=`<div class="page-heading"><div><div class="eyebrow">PREFERENCES</div><h1>环境与设置</h1><p class="subtitle">默认保持安静，需要时再运行。</p></div></div>
    <h2 class="setting-group">外观</h2><label class="setting"><div><h3>主题</h3><p>默认跟随系统外观。</p></div><select data-setting="theme" aria-label="主题">${[['system','跟随系统'],['light','浅色'],['dark','深色']].map(([v,l])=>`<option value="${v}" ${preferences.theme===v?'selected':''}>${l}</option>`).join('')}</select></label>
    <h2 class="setting-group">搜索与操作索引</h2><label class="setting"><div><h3>回车默认操作</h3><p>在搜索结果上按回车时执行；Shift+回车执行另一项。</p></div><select data-setting="enterAction" aria-label="回车默认操作"><option value="primary" ${preferences.enterAction==='primary'?'selected':''}>打开或启动</option><option value="details" ${preferences.enterAction==='details'?'selected':''}>查看详情</option></select></label><label class="setting"><div><h3>搜索项目路径</h3><p>除名称、分支和分组外，也匹配项目目录。</p></div><input type="checkbox" data-setting="searchPaths" ${preferences.searchPaths?'checked':''} aria-label="搜索项目路径"></label><label class="setting"><div><h3>在搜索中显示操作</h3><p>关闭后只列应用；输入 &gt; 仍可只搜索操作。</p></div><input type="checkbox" data-setting="searchActions" ${preferences.searchActions?'checked':''} aria-label="在搜索中显示操作"></label><div class="setting"><div><h3>搜索语法</h3><p><code>is:运行</code> <code>is:失败</code> 按状态，<code>in:工作</code> 按分组，<code>&gt;</code> 只搜操作，<code>停止 notes</code> 直接执行。在画布上直接打字即可开始搜索。</p></div></div>
    <h2 class="setting-group">运行</h2><div class="setting"><div><h3>运行环境 PATH</h3><p>所有入口共用同一环境。仅在你主动选择时采集 shell 配置。</p></div><button data-action="environment">查看与配置</button></div><label class="setting"><div><h3>登录时启动 AppHost 后台</h3><p>只启动管理后台，不自动启动应用。</p></div><input type="checkbox" data-setting="login" ${preferences.login?'checked':''} aria-label="登录时启动后台"></label><label class="setting"><div><h3>托盘常驻</h3><p>默认关闭。关闭窗口不影响后台服务。</p></div><input type="checkbox" data-setting="tray" ${preferences.tray?'checked':''} aria-label="托盘常驻"></label><div class="setting"><div><h3>自动唤醒</h3><p>${paused?'已暂停；手动打开某个应用仍然有效。':'仅允许满足规则的浏览器导航触发，不接受任意网络请求。'}</p></div><button data-action="${paused?'resume':'pause'}">${paused?'恢复按需唤醒':'暂停按需唤醒'}</button></div><div class="setting"><div><h3>关闭窗口</h3><p>窗口退出后，后台及正在运行的应用保持不变。</p></div><button data-action="close-window">了解</button></div><p class="note">设置在此原型中只演示交互，刷新后恢复默认。</p>`;
  main.innerHTML=`<div class="management"><nav class="sidebar" aria-label="管理导航"><h2>管理工作区</h2>${nav.map(([id,i,label])=>`<button class="${section===id?'active':''}" data-action="section" data-section="${id}" ${section===id?'aria-current="page"':''}>${icon(i)}${label}</button>`).join('')}</nav><div class="manage-content">${content}</div></div>`;
}
function openSection(name){view='manage';section=name;render();}
function applyTheme(){document.documentElement.dataset.theme=preferences.theme==='system'?'':preferences.theme;}
function toggleTheme(){
  const dark=preferences.theme==='dark'||(preferences.theme==='system'&&matchMedia('(prefers-color-scheme: dark)').matches);
  preferences.theme=dark?'light':'dark';applyTheme();event(`切换为${dark?'浅色':'深色'}外观`);if(view==='manage')render();
}

/* ── Shared tooltip ────────────────────────────────────────────── */
// One floating tooltip for every [data-tip]; it escapes the canvas' overflow clipping.
const tip=document.createElement('div');tip.id='tip';tip.setAttribute('role','tooltip');document.body.append(tip);
let tipTimer,tipTarget=null,tipWarmUntil=0;
function hideTip(){clearTimeout(tipTimer);if(tip.classList.contains('show'))tipWarmUntil=Date.now()+500;tip.classList.remove('show');tipTarget=null;}
function showTip(target){
  if(!target.isConnected||!target.dataset.tip||menu.isOpen||document.body.classList.contains('dragging'))return;
  tip.textContent=target.dataset.tip;tip.classList.add('show');
  const rect=target.getBoundingClientRect(),box=tip.getBoundingClientRect();
  const below=rect.bottom+8+box.height<innerHeight;
  tip.style.left=`${Math.max(8,Math.min(rect.left+rect.width/2-box.width/2,innerWidth-box.width-8))}px`;
  tip.style.top=`${below?rect.bottom+8:rect.top-box.height-8}px`;
}
function scheduleTip(target){
  clearTimeout(tipTimer);tipTarget=target;
  const delay=Date.now()<tipWarmUntil?0:Number(target.dataset.tipDelay??250);
  tipTimer=setTimeout(()=>showTip(target),delay);
}
document.addEventListener('pointerover',e=>{const target=e.target.closest?.('[data-tip]');if(target===tipTarget)return;hideTip();if(target)scheduleTip(target);});
document.addEventListener('focusin',e=>{const target=e.target.closest?.('[data-tip]');if(target&&target.matches(':focus-visible'))scheduleTip(target);});
document.addEventListener('focusout',()=>hideTip());
for(const type of ['pointerdown','keydown','wheel'])document.addEventListener(type,()=>hideTip(),true);

/* ── Feedback placement ────────────────────────────────────────── */
// Feedback goes where its scope is (5.9): tile shape for ongoing state, a callout for the
// action just taken on one app, the activity area for batch and global results, and the
// detail panel for evidence. A message raised inside a dialog stays in that dialog.
function notify(message,action){
  if(modal.open){let feedback=modal.querySelector('[data-feedback]');if(!feedback){feedback=document.createElement('p');feedback.dataset.feedback='';feedback.className='note';feedback.setAttribute('role','status');modal.querySelector('.dialog-body').append(feedback);}feedback.textContent=message;return;}
  activity.notice(message,action);
}
function event(text){events.unshift({text,time:new Date().toLocaleTimeString('zh-CN',{hour12:false})});}
function showModal(title,body,actions=''){menu.close(false);modal.innerHTML=`<div class="dialog-heading"><h2 id="modal-title">${title}</h2><button class="close" data-action="close-modal" aria-label="关闭对话框">×</button></div><div class="dialog-body">${body}</div>${actions?`<div class="dialog-actions">${actions}</div>`:''}`;if(!modal.open)modal.showModal();else modal.querySelector('.close').focus();}
function closeModal(){modal.close();if(!detail.open)searchInput.focus();}
function update(){render();if(detail.open&&selected)renderDetail();refreshCallouts();}
function getApp(id){return apps.find(a=>a.id===id);}
// Two installations of one project share a name; the branch tells them apart in lists.
const labelOf=app=>displayedApps().filter(a=>a.name===app.name).length>1?`${app.name} · ${app.branch}`:app.name;
function declaration(app){return app.declaration || `schema_version = 1\nproject_id = "${app.projectId}"\nname = "${app.name}"\n\n${(app.services||[svc('web',app.port)]).map(s=>`[[services]]\nname = "${s.name}"\ncommand = ${JSON.stringify(app.command ? app.command.split(" ") : ["npm","run","dev"])}\ncwd = "."${s.port?`\nport = ${s.port}`:''}\nstartup_timeout_ms = ${app.startupTimeout||30000}`).join('\n\n')}`;}
// Left click is always the one obvious next step for the tile's state.
function primaryLabel(app){return ({running:app.port?'打开':'查看日志',stopped:'启动',failed:isPartial(app)?'完整重启':'重新启动',review:'复核声明',missing:'重新关联',starting:'正在启动',stopping:'正在停止'})[tileState(app)];}
function primaryIcon(app){return ({running:'arrow',stopped:'play',failed:'restart',review:'edit',missing:'folder',starting:'play',stopping:'stop'})[tileState(app)];}
function primary(app){
  // Offline, a tile still opens its last known details; nothing is sent to the daemon.
  if(!connected){openDetail(app);return;}
  const state=tileState(app);
  if(state==='review'||state==='missing')openDetail(app);
  else if(state==='failed'&&isPartial(app))restartApp(app);
  else if(state==='running')openEntry(app);
  else if(state==='stopped'||state==='failed')startApp(app);
}
const canStart=app=>!app.review&&app.problem!=='source'&&['STOPPED','FAILED'].includes(app.state)&&!isPartial(app);

/* ── Detail panel ──────────────────────────────────────────────── */
// Non-modal (5.11): the canvas stays usable and the tile stays selected while it is open.
const DAEMON_ACTIONS=new Set(['launch','retry','restart','stop','refresh-logs','stop-holder-start','open-entry']);
function renderServices(app){
  return `<div class="section-label">服务</div><ul class="svc-list">${app.services.map(s=>`<li data-svc="${s.state}"><i class="svc-dot" aria-hidden="true"></i><span class="svc-name">${escape(s.name)}</span><span class="muted">${s.port?`:${s.port}`:'无端口'}</span><span class="svc-state">${s.note?escape(s.note):SERVICE_LABEL[s.state]}${s.exitCode!==undefined?` · exit ${s.exitCode}`:''}</span></li>`).join('')}</ul>`;
}
const actionButton=(app,a,cls='')=>`<button class="${cls}" data-action="${a.action}" data-id="${app.id}"${a.focus?` data-focus="${a.focus}"`:''}>${escape(a.label)}</button>`;
// "What changed since it last worked" is the first question for an app you have forgotten.
function sinceSuccess(app){
  const h=app.history;if(!h)return '';
  if(!h.lastSuccess)return `<div class="section-label">自上次成功以来</div><p class="since-empty">这个安装还没有成功运行过。</p>`;
  return `<div class="section-label">自上次成功以来 · ${escape(h.lastSuccess)}</div>${h.changes.length?`<ul class="since">${h.changes.map(c=>`<li><b>${escape(c.label)}</b><span>${escape(c.text)}${c.by?`<small>${escape(c.by)}</small>`:''}</span></li>`).join('')}</ul>`:'<p class="since-empty">AppHost 记录的分支、启动声明和运行环境都没有变化，原因更可能在项目依赖或外部环境。</p>'}`;
}
function failureHistory(app){
  const list=(app.failures||[]).slice(1,4);if(!list.length)return '';
  return `<div class="section-label">更早的失败</div><ul class="since">${list.map(f=>`<li><b>${clock(f.at)}</b><span>${escape(explainFailure(app,f).title)}</span></li>`).join('')}</ul>`;
}
function failureView(app){
  const f=app.failure,x=explainFailure(app,f),cleared=clearedCondition(app);
  // Inside the panel the cause is already on screen, so "view cause" gives way to the fix.
  const escalated=mainAction(x,f),main=cleared?ACT.retry:escalated.action==='details'?x.primary:escalated;
  const others=[x.primary,...x.secondary].filter((a,i,list)=>a.action!==main.action&&list.findIndex(b=>b.action===a.action)===i);
  return `<section class="fail-card"><p class="fail-kicker"><span>启动失败 · ${clock(f.at)} · ${PHASE_LABEL[f.phase]}</span><code>${f.code}</code></p><h3>${escape(x.title)}</h3><p>${escape(x.impact)}</p>${f.count>1?`<p class="fail-flag">第 ${f.count} 次，与上次原因相同。先处理原因，再重试。</p>`:''}${cleared?`<p class="fail-flag cleared">${escape(cleared)}，可以重试。</p>`:''}</section>
    <dl class="evidence">${x.evidence.filter(([,v])=>v).map(([k,v])=>`<dt>${escape(k)}</dt><dd>${escape(v)}</dd>`).join('')}</dl>
    ${f.tail?`<div class="section-label">输出末尾 · ${escape(f.service)} · stderr</div><pre class="tail" tabindex="0">${f.tail.map((line,i)=>i===f.mark?`<mark>${escape(line)}</mark>`:escape(line)).join('\n')}</pre>`:''}
    ${x.suggestion?`<div class="suggest">${x.suggestion.command?`<div class="suggest-cmd"><code>${escape(x.suggestion.command)}</code><button data-action="copy-command" data-id="${app.id}">复制</button></div>`:''}<p>${escape(x.suggestion.note||x.suggestion.text)}</p></div>`:''}
    ${actionButton(app,main,'primary full')}
    <div class="row-actions" style="margin-top:12px">${others.map(a=>actionButton(app,a)).join('')}</div>
    ${x.note?`<p class="note">${escape(x.note)}</p>`:''}
    ${sinceSuccess(app)}${failureHistory(app)}${renderServices(app)}`;
}
function lastRun(app){
  if(app.failure&&app.state!=='FAILED')return `上次启动失败 · ${clock(app.failure.at)} · ${explainFailure(app,app.failure).title}`;
  if(app.history?.lastSuccess)return `上次成功 · ${app.history.lastSuccess}`;
  return app.history?'还没有成功运行过':null;
}
function renderDetail(){
  const focusedAction=detail.contains(document.activeElement)?document.activeElement.dataset.action:null;
  const focusedTab=document.activeElement.dataset.tab;
  const app=displayedApps().find(a=>a.id===selected);if(!app){closeDetail();return;}
  let body='';
  if(detailTab==='logs')body=logsPanel(app,connected,logReads.get(app.id));
  else if(detailTab==='terminal')body=terminalPanel(app);
  else if(detailTab==='manifest')body=`<p class="muted">项目内 · ${escape(app.path || `~/Projects/${app.id}`)}/apphost.toml</p><div style="height:12px"></div><pre>${escape(declaration(app))}</pre><button class="full" data-action="edit" data-id="${app.id}">编辑启动声明</button>`;
  else if(app.problem==='source')body=`<div class="reason neutral"><code>SOURCE_MISSING</code><h3>找不到项目目录</h3><p>${escape(app.path)} 已不存在，可能被移动或重命名。登记和历史记录都还在。</p></div><button class="primary full" data-action="relink" data-id="${app.id}">重新关联路径</button><p class="note">选择新的项目目录后，AppHost 会核对项目身份；下一次启动需要重新确认声明。</p>`;
  else if(app.review)body=`<div class="reason neutral"><code>REVIEW_REQUIRED</code><h3>启动声明有变更，需要你确认</h3><p>当前应用未启动。确认的是启动声明，不代表脚本及依赖内容可信。</p></div>${declarationDiff(app,declaration(app))}<button class="primary full" data-action="review" data-id="${app.id}">复核并确认</button><p class="note">“待确认”是提示；当前运行状态仍为已停止。</p>`;
  else if(app.state==='FAILED'&&app.failure)body=failureView(app);
  else{const last=lastRun(app);body=`<dl class="evidence"><dt>当前状态</dt><dd>${stateLabel(app)}</dd>${last?`<dt>上次运行</dt><dd>${escape(last)}</dd>`:''}<dt>项目分支</dt><dd>${escape(app.branch)}</dd><dt>分组</dt><dd>${escape(groupInfo(groupOf(app)).name)}</dd><dt>项目目录</dt><dd>${escape(app.path || `~/Projects/${app.id}`)}</dd></dl>${renderServices(app)}<button class="primary full" data-action="launch" data-id="${app.id}" ${['STARTING','STOPPING'].includes(app.state)?'disabled':''}>${app.state==='RUNNING'?(app.port?'打开应用':'查看运行日志'):'启动应用'}</button>${active(app)?`<div class="row-actions" style="margin-top:12px"><button data-action="restart" data-id="${app.id}">完整重启</button><button data-action="stop" data-id="${app.id}">停止应用</button></div>`:''}${app.failure&&app.state!=='FAILED'?`<div class="divider"></div>${sinceSuccess(app)}`:''}`;}
  const offline=connected?'':`<p class="note offline-note">后台未连接：这里是断开前的状态，启动、停止和刷新暂不可用。</p>`;
  detail.innerHTML=`<div class="dialog-heading"><div class="detail-identity"><div class="app-icon" style="${idStyle(app)}">${glyph(app)}</div><div><h2 id="detail-title">${escape(app.name)}</h2><p class="subtitle">${escape(app.branch)} · ${stateLabel(app)}</p></div></div><button class="close" data-action="close-detail" aria-label="关闭应用详情">×</button></div><nav class="detail-tabs" aria-label="应用详情">${[['overview','概览'],['logs','日志'],['terminal','命令行'],['manifest','声明']].map(([id,label])=>`<button class="${detailTab===id?'active':''}" data-action="detail-tab" data-tab="${id}" aria-current="${detailTab===id?'page':'false'}">${label}</button>`).join('')}</nav><div class="dialog-body">${offline}${body}<div class="divider"></div><div class="row-actions">${app.problem==="source"?"":`<button data-action="relink" data-id="${app.id}">重新关联路径</button>`}<button class="text-button" data-action="detach" data-id="${app.id}">解除登记</button></div></div>`;
  // Controls that need the daemon are disabled ahead of time, with the reason on hover.
  if(!connected)for(const button of detail.querySelectorAll('button[data-action]'))if(DAEMON_ACTIONS.has(button.dataset.action)){button.disabled=true;button.dataset.tip='后台未连接';}
  if(detail.contains(document.activeElement)||focusedAction){const replacement=[...detail.querySelectorAll('button[data-action]')].find(button=>button.dataset.action===focusedAction&&button.dataset.tab===focusedTab);if(!modal.open)(replacement || detail.querySelector('.close')).focus();}
}
function syncSelection(){for(const tile of main.querySelectorAll('.tile'))tile.toggleAttribute('data-selected',detail.open&&tile.dataset.installation===selected);}
function openDetail(app,tabName='overview'){
  menu.close(false);selected=app.id;detailTab=tabName;renderDetail();
  if(!detail.open){detail.show();detail.querySelector('.close').focus();}
  syncSelection();
  // Keep the selected tile in view beside the panel.
  main.querySelector(`.tile[data-installation="${CSS.escape(app.id)}"]:not(.small)`)?.scrollIntoView({inline:'nearest',block:'nearest',behavior:reducedMotion.matches?'auto':'smooth'});
}
function closeDetail(){
  if(!detail.open)return;
  const hadFocus=detail.contains(document.activeElement);
  detail.close();syncSelection();
  if(hadFocus)main.querySelector(`.tile[data-installation="${CSS.escape(selected)}"]:not(.small) .tile-main`)?.focus();
}
function openEntry(app){
  if(!app.port){openDetail(app,'logs');return;}
  event(`打开 ${app.name}`);
  callouts.show(app.id,{tone:'success',title:'已在浏览器打开',body:`localhost:${app.port}（原型演示，未真正打开）`,timeout:2600});
}
function requireConnection(){if(connected)return true;notify('后台未连接，这个操作需要先重连。');return false;}

/* ── Lifecycle simulation ──────────────────────────────────────── */
// interactive=false for batch starts: their results go to the activity area, not callouts.
function startApp(app,{interactive=true,onSettle}={}){
  if(!requireConnection()||['STARTING','STOPPING'].includes(app.state))return;
  if(app.review||app.problem==='source'){if(interactive)openDetail(app);return;}
  if(app.state==='RUNNING'){if(interactive)openEntry(app);return;}
  const plan=simulateStart(app),timeout=startupTimeoutOf(app),began=Date.now();
  const version=app.version=(app.version||0)+1;
  app.state='STARTING';app.lastUsed=0;app.ready=true;setServices(app,'starting');
  for(const s of app.services){delete s.exitCode;delete s.note;}
  event(`启动 ${app.name}`);
  if(interactive)callouts.dismiss(app.id);
  update();
  // A start that outlives a glance reports progress against its declared deadline, so a
  // slow app never looks like a hung one. Dismissing it once keeps it away for this start.
  let shown=false;
  const tick=setInterval(()=>{
    if(app.version!==version||app.state!=='STARTING'){clearInterval(tick);return;}
    const waited=Math.floor((Date.now()-began)/1000);
    if(!interactive||waited<5)return;
    const spec={tone:'progress',title:'仍在启动',body:`已等待 ${waited} 秒，最多等待 ${Math.round(timeout/1000)} 秒。`,progress:Math.min(1,(Date.now()-began)/timeout),actions:[{label:'查看日志',action:'logs'},{label:'停止',action:'stop'}]};
    if(!shown){shown=true;callouts.show(app.id,spec);}else if(callouts.specOf(app.id)?.tone==='progress')callouts.update(app.id,spec);
  },1000);
  setTimeout(()=>{
    clearInterval(tick);
    // Accepted intent is superseded by Stop All, explicit stop or a reset.
    if(!apps.includes(app)||app.version!==version)return;
    if(plan.failure){
      recordFailure(app,{...plan.failure});
      const reason=explainFailure(app,app.failure).title;
      event(`${app.name} 启动失败：${reason}`);
      if(interactive&&connected)showFailureCallout(app);
      onSettle?.({ok:false,reason});
    }else{
      app.state='RUNNING';setServices(app,'running');app.failure=null;
      if(app.history)app.history={lastSuccess:'刚刚',changes:[]};
      event(`${app.name} 已就绪`);
      if(interactive&&connected)callouts.show(app.id,app.port?{tone:'success',title:'已就绪，已在浏览器打开',body:`localhost:${app.port}（原型演示，未真正打开）`,timeout:2800}:{tone:'success',title:'已启动',body:'没有声明端口，只确认了进程存活。',timeout:2800});
      onSettle?.({ok:true});
    }
    if(connected)update();
  },plan.delay);
}
// One sentence, the impact, one main action. The drawer holds the evidence.
function showFailureCallout(app){
  const f=app.failure,x=explainFailure(app,f),act=mainAction(x,f);
  callouts.show(app.id,{tone:'fail',title:x.title,body:f.count>1?`与上次原因相同（第 ${f.count} 次）。${x.impact}`:x.impact,failureAt:f.at,actions:[{...act,primary:true}],details:act.action!=='details'});
}
// When a failure's condition clears, its callout turns into an offer to retry.
function refreshCallouts(){
  for(const app of apps){
    const spec=callouts.specOf(app.id);if(!spec||spec.tone!=='fail'||spec.failureAt!==app.failure?.at)continue;
    const cleared=clearedCondition(app);
    if(cleared)callouts.show(app.id,{tone:'info',title:cleared,body:`可以重新启动 ${app.name}。`,actions:[{...ACT.retry,primary:true}],details:true});
  }
}
function stopApp(app,after){
  app.version=(app.version||0)+1;const version=app.version;
  callouts.dismiss(app.id);
  if(!active(app)&&!liveServices(app).length){app.state='STOPPED';setServices(app,'stopped');update();after?.();return;}
  app.state='STOPPING';for(const s of app.services)if(s.state!=='stopped')s.state='stopping';event(`停止 ${app.name}`);update();
  setTimeout(()=>{if(!apps.includes(app)||app.version!==version)return;app.state='STOPPED';setServices(app,'stopped');for(const s of app.services)delete s.note;event(`${app.name} 已停止`);if(connected)update();after?.();},700);
}
function restartApp(app){if(!requireConnection()||app.review)return;if(hasLive(app))stopApp(app,()=>startApp(app));else startApp(app);}
// Stop All (5.6): pause wake, cancel starts, stop what AppHost can prove it owns, and
// report honestly, including processes it could not claim and therefore left alone.
function stopAll(){
  if(!requireConnection())return;
  paused=true;activity.setPaused(true);modal.close();menu.close(false);
  const targets=apps.filter(a=>hasLive(a)||a.state==='STOPPING');
  const op=activity.begin({title:'全部停止',verbs:{working:'正在停止',done:'已停止'},note:'自动唤醒已暂停，直到你恢复。',
    items:[...targets.map(a=>({key:a.id,appId:a.id,name:labelOf(a),status:'working'})),...(orphanDemo?[{key:'orphan',name:'node · PID 40211',status:'working'}]:[])]});
  for(const app of targets){const wasStarting=app.state==='STARTING';stopApp(app,()=>op.set(app.id,wasStarting?'canceled':'done'));}
  if(orphanDemo)setTimeout(()=>op.set('orphan','orphan','身份无法确认，AppHost 没有结束它'),900);
  event(`全部停止（${targets.length} 个）；自动唤醒已暂停`);if(lens==='running')lens=null;update();
}
function pauseWake(){if(requireConnection()){paused=true;activity.setPaused(true);event('暂停按需唤醒');update();}}
function resumeWake(){if(requireConnection()){paused=false;activity.setPaused(false);event('恢复按需唤醒');update();notify('已恢复按需唤醒，不会立即启动应用。');}}

/* ── Groups: batch operations and user-defined groups ──────────── */
// Batches never raise per-app callouts; the activity area lists each outcome.
function groupStart(key){
  if(!requireConnection())return;
  const all=displayedApps().filter(a=>groupOf(a)===key);
  const starting=all.filter(canStart),skipped=all.filter(needsReview);
  const op=activity.begin({title:`启动 ${groupInfo(key).name}`,verbs:{working:'正在启动',done:'已就绪'},
    items:[...starting.map(a=>({key:a.id,appId:a.id,name:labelOf(a),status:'working'})),...skipped.map(a=>({key:a.id,appId:a.id,name:labelOf(a),status:'skipped',text:a.review?'启动声明待确认':'找不到项目目录'}))]});
  for(const app of starting)startApp(app,{interactive:false,onSettle:result=>op.set(app.id,result.ok?'done':'failed',result.reason)});
  event(`启动分组 ${groupInfo(key).name}（${starting.length} 个）`);
}
function groupStop(key){
  if(!requireConnection())return;
  const members=displayedApps().filter(a=>groupOf(a)===key&&hasLive(a));
  const op=activity.begin({title:`停止 ${groupInfo(key).name}`,verbs:{working:'正在停止',done:'已停止'},items:members.map(a=>({key:a.id,appId:a.id,name:labelOf(a),status:'working'}))});
  for(const app of members)stopApp(app,()=>op.set(app.id,'done'));
  event(`停止分组 ${groupInfo(key).name}（${members.length} 个）`);
}
// Grouping is appearance data, so it is undone rather than confirmed.
function moveApp(app,key,{undoable=true}={}){
  if(groupOf(app)===key)return;
  const from=groupOf(app);
  app.group=key;event(`将 ${app.name} 移到 ${groupInfo(key).name}`);render();
  if(undoable)notify(`已将 ${app.name} 移到 ${groupInfo(key).name}`,{label:'撤销',run:()=>morph(()=>moveApp(app,from,{undoable:false}))});
}
function createGroup(name,appId){
  const key=`custom-${++groupSeq}`;
  groups.push({key,name,custom:true});
  event(`新建分组 ${name}`);
  const app=appId&&getApp(appId);
  if(app){app.group=key;event(`将 ${app.name} 移到 ${name}`);}
  render();
  requestAnimationFrame(()=>{const found=[...main.querySelectorAll('.group[data-group]')].find(g=>g.dataset.group===key);found?.scrollIntoView({inline:'nearest',block:'nearest',behavior:reducedMotion.matches?'auto':'smooth'});found?.querySelector('.group-title')?.focus();});
}
// Dissolving returns members to their directory groups; nothing in the project changes.
function dissolveGroup(key){
  const info=groupInfo(key);
  const members=apps.filter(app=>app.group===key),index=groups.findIndex(g=>g.key===key);
  for(const app of members)app.group=app.home;
  groups=groups.filter(g=>g.key!==key);collapsed.delete(key);
  event(`解散分组 ${info.name}，应用回到原目录分组`);render();
  notify(`已解散 ${info.name}，${members.length} 个应用回到原目录分组`,{label:'撤销',run:()=>morph(()=>{groups.splice(index,0,info);for(const app of members)app.group=key;event(`恢复分组 ${info.name}`);render();})});
}
function promptGroup({appId,rename}){
  const current=rename?groupInfo(rename).name:'';
  showModal(rename?'重命名分组':'新建分组',`<label class="section-label" for="group-name">名称</label><input id="group-name" class="full" maxlength="24" value="${escape(current||'新分组')}" autocomplete="off"><p id="group-error" class="form-error" role="alert"></p>${appId?`<p class="note">创建后将 ${escape(getApp(appId).name)} 移入。</p>`:''}<p class="note">分组只影响 AppHost 的界面排布，不修改项目文件或登记。</p>`,`<button data-action="close-modal">取消</button><button class="primary" data-action="commit-group" ${appId?`data-id="${appId}"`:''} ${rename?`data-group="${escape(rename)}"`:''}>${rename?'保存':'创建'}</button>`);
  const input=modal.querySelector('#group-name');input.focus();input.select();
}
function commitGroup(button){
  const name=modal.querySelector('#group-name').value.trim();
  if(!name){modal.querySelector('#group-error').textContent='请输入分组名称。';return;}
  modal.close();
  if(button.dataset.group){const info=groups.find(g=>g.key===button.dataset.group);event(`分组 ${info.name} 重命名为 ${name}`);info.name=name;morph(render);}
  else morph(()=>createGroup(name,button.dataset.id));
}

/* ── Context menus ─────────────────────────────────────────────── */
function appMenu(app){
  const state=tileState(app),alive=hasLive(app),entry=app.port&&app.services.some(s=>s.port===app.port&&s.state==='running');
  const targets=groups.filter(g=>g.key!==groupOf(app)&&(g.custom||apps.some(a=>groupOf(a)===g.key)));
  return [
    {label:primaryLabel(app),icon:primaryIcon(app),run:()=>primary(app),disabled:!connected||['starting','stopping'].includes(state),hint:connected?'':'未连接'},
    ...(entry&&state!=='running'?[{label:'打开仍在运行的入口',icon:'arrow',run:()=>openEntry(app),disabled:!connected}]:[]),
    {label:'停止',icon:'stop',run:()=>{if(requireConnection())stopApp(app);},disabled:!connected||!alive},
    {label:'完整重启',icon:'restart',run:()=>restartApp(app),disabled:!connected||!alive||app.review},
    '-',
    {label:'查看日志',icon:'logs',run:()=>openDetail(app,'logs')},
    {label:'详情与声明',icon:'info',run:()=>openDetail(app)},
    '-',
    {label:'移到分组',icon:'folder',items:[
      ...targets.map(g=>({label:g.name,icon:'folder',run:()=>morph(()=>moveApp(app,g.key))})),
      ...(targets.length?['-']:[]),
      {label:'新建分组…',icon:'newgroup',run:()=>promptGroup({appId:app.id}),keepFocus:true},
      ...(groupOf(app)!==app.home?[{label:`移回 ${groupInfo(app.home).name}`,icon:'ungroup',run:()=>morph(()=>moveApp(app,app.home))}]:[])
    ]}
  ];
}
function groupMenu(key){
  const info=groupInfo(key),members=displayedApps().filter(a=>groupOf(a)===key);
  return [
    {label:'全部启动',icon:'play',run:()=>groupStart(key),disabled:!connected||!members.some(canStart)},
    {label:'全部停止',icon:'stop',run:()=>groupStop(key),disabled:!connected||!members.some(hasLive)},
    '-',
    {label:collapsed.has(key)?'展开':'折叠',icon:'chevron',run:()=>morph(()=>toggleGroup(key)),disabled:!members.length},
    ...(info.custom?[{label:'重命名…',icon:'edit',run:()=>promptGroup({rename:key}),keepFocus:true},{label:'解散分组',icon:'ungroup',danger:true,run:()=>morph(()=>dissolveGroup(key))}]:[{label:'新建分组…',icon:'newgroup',run:()=>promptGroup({}),keepFocus:true}])
  ];
}
function addMenu(){return [{label:'添加应用',icon:'plus',run:()=>handleFlow('import'),keepFocus:true},{label:'新建分组',icon:'newgroup',run:()=>promptGroup({}),keepFocus:true}];}
function canvasMenu(){
  const running=displayedApps().filter(hasLive).length;
  return [
    ...addMenu(),'-',
    {label:'折叠全部分组',icon:'chevron',run:()=>setAllCollapsed(true)},
    {label:'展开全部分组',icon:'expand',run:()=>setAllCollapsed(false)},'-',
    {label:`停止全部运行中的应用（${running}）`,icon:'stop',danger:true,run:stopAll,disabled:!connected||!running},
    paused?{label:'恢复自动唤醒',icon:'play',run:resumeWake}:{label:'暂停自动唤醒',icon:'pause',run:pauseWake}
  ];
}
function menuFor(el){
  const tile=el.closest('.tile'),group=el.closest('.group[data-group]');
  if(tile){const app=getApp(tile.dataset.installation);return app&&[appMenu(app),app.name];}
  if(el.closest('.tile-add'))return [addMenu(),''];
  if(group&&el.closest('.group-head,.folder'))return [groupMenu(group.dataset.group),groupInfo(group.dataset.group).name];
  return [canvasMenu(),''];
}
document.addEventListener('contextmenu',e=>{
  if(view!=='home'||!e.target.closest?.('.board')||modal.open)return;
  const found=menuFor(e.target);if(!found)return;
  e.preventDefault();menu.open(found[0],{x:e.clientX,y:e.clientY},e.target.closest('button')||null,found[1]);
});

/* ── Keyboard: type to search, arrows move across the canvas ───── */
function focusables(){
  return [...main.querySelectorAll('.tile-main,.folder,.tile-add')].filter(el=>el.tabIndex>=0&&el.getClientRects().length);
}
function moveFocus(key,from){
  const list=focusables();if(!list.length)return;
  if(!from||!list.includes(from)){list[0].focus();return;}
  const a=from.getBoundingClientRect(),ax=a.left+a.width/2,ay=a.top+a.height/2;
  let best=null,bestScore=Infinity;
  for(const el of list){
    if(el===from)continue;
    const b=el.getBoundingClientRect(),dx=b.left+b.width/2-ax,dy=b.top+b.height/2-ay;
    const [along,across]=key==='ArrowRight'?[dx,dy]:key==='ArrowLeft'?[-dx,dy]:key==='ArrowDown'?[dy,dx]:[-dy,dx];
    if(along<=4)continue;
    const score=along+Math.abs(across)*2;
    if(score<bestScore){bestScore=score;best=el;}
  }
  if(best){best.focus({preventScroll:true});best.scrollIntoView({block:'nearest',inline:'nearest',behavior:reducedMotion.matches?'auto':'smooth'});}
}
document.addEventListener('keydown',e=>{
  if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='k'&&!modal.open){e.preventDefault();menu.close(false);searchInput.focus();searchInput.select();return;}
  if(modal.open||menu.isOpen||e.defaultPrevented)return;
  // The detail panel is non-modal: Escape inside it closes it and returns to the tile.
  if(detail.contains(e.target)){if(e.key==='Escape'){e.preventDefault();closeDetail();}return;}
  if(e.target.closest?.('input,textarea,select,[contenteditable],.callout-layer,.act-panel,.topbar'))return;
  // Start-screen behaviour: typing anywhere on the canvas starts a search. Only focus
  // moves here; the keystroke itself lands in the box, so an IME can compose it.
  if(view==='home'&&e.key.length===1&&e.key!==' '&&!e.metaKey&&!e.ctrlKey&&!e.altKey){searchInput.focus();return;}
  const target=e.target.closest?.('.tile-main,.folder,.tile-add,.group-title,.mini-btn');
  // Tab from a tile steps into its callout; the callout never takes focus by itself.
  if(e.key==='Tab'&&!e.shiftKey&&target?.matches('.tile-main')&&callouts.focus(target.dataset.id)){e.preventDefault();return;}
  if(e.key.startsWith('Arrow')&&view==='home'&&main.querySelector('.board')&&(target||e.target===document.body||e.target===main)){
    e.preventDefault();
    const from=target&&target.matches('.tile-main,.folder,.tile-add')?target:target?.closest('.group')?.querySelector('.tile-main,.folder');
    moveFocus(e.key,from);return;
  }
  if((e.key==='ContextMenu'||(e.shiftKey&&e.key==='F10'))&&target){
    e.preventDefault();const found=menuFor(target),rect=target.getBoundingClientRect();
    if(found)menu.open(found[0],{x:rect.left+12,y:rect.bottom+4},target,found[1]);return;
  }
  if(e.key==='Escape'){if(target?.matches('.tile-main')&&callouts.has(target.dataset.id))callouts.dismiss(target.dataset.id);else if(query){query='';searchInput.value='';morph(render);}else if(lens)setLens(null);else closeDetail();}
});

/* ── Drag and drop between groups ──────────────────────────────── */
let dragId=null;
const clearDrop=()=>{for(const el of document.querySelectorAll('.drop-target'))el.classList.remove('drop-target');};
document.addEventListener('dragstart',e=>{
  const tile=e.target.closest?.('.tile[draggable=true]');if(!tile||view!=='home')return;
  dragId=tile.dataset.installation;e.dataTransfer.effectAllowed='move';e.dataTransfer.setData('text/plain',dragId);
  hideTip();menu.close(false);requestAnimationFrame(()=>{document.body.classList.add('dragging');tile.classList.add('drag-source');});
});
// A drop may rebuild the board and detach the source tile; its dragend then never
// reaches document, so drop ends the drag itself as well.
function endDrag(){dragId=null;document.body.classList.remove('dragging');for(const el of document.querySelectorAll('.drag-source'))el.classList.remove('drag-source');clearDrop();}
document.addEventListener('dragend',endDrag);
document.addEventListener('dragover',e=>{
  if(!dragId)return;
  const zone=e.target.closest?.('[data-drop-new],.group[data-group]');
  if(!zone){clearDrop();return;}
  e.preventDefault();e.dataTransfer.dropEffect='move';
  if(!zone.classList.contains('drop-target')){clearDrop();zone.classList.add('drop-target');}
});
document.addEventListener('drop',e=>{
  if(!dragId)return;
  const zone=e.target.closest?.('[data-drop-new],.group[data-group]'),app=getApp(dragId);
  if(!zone||!app){endDrag();return;}
  e.preventDefault();endDrag();
  if(zone.hasAttribute('data-drop-new'))promptGroup({appId:app.id});
  else if(zone.dataset.group!==groupOf(app))morph(()=>moveApp(app,zone.dataset.group));
});

/* ── Commands ──────────────────────────────────────────────────── */
function reset(empty=false){logReads.clear();callouts.clear();activity.reset();preferences=defaultPreferences();applyTheme();apps=empty?[]:structuredClone(seed);groups=structuredClone(groupsSeed);events=[];paused=false;connected=true;portBusy=true;orphanDemo=false;query='';view='home';collapsed.clear();lens=null;builtKey='';searchInput.value='';detail.close();modal.close();menu.close(false);render();}
document.addEventListener('click',async e=>{
  const button=e.target.closest('button[data-action]');if(!button||button.disabled)return;
  const action=button.dataset.action, app=getApp(button.dataset.id||selected);
  switch(action){
    case 'home':view='home';render();break;
    case 'manage':view='manage';section='apps';render();break;
    case 'section':section=button.dataset.section;render();break;
    case 'clear-search':query='';searchInput.value='';render();break;
    case 'details':openDetail(app);break;
    case 'refresh-logs':if(requireConnection()){logReads.set(app.id,new Date().toLocaleTimeString('zh-CN',{hour12:false}));renderDetail();}break;
    case 'open-terminal':showModal('系统终端入口',`<p>在项目目录打开系统终端：</p><pre>${escape(app.path || `~/Projects/${app.id}`)}</pre><p class="note">交互演示：未打开真实终端，也未执行命令。</p>`,`<button class="primary" data-action="close-modal">返回</button>`);break;
    case 'detail-tab':detailTab=button.dataset.tab;renderDetail();break;
    case 'close-detail':closeDetail();break;
    case 'logs':openDetail(app,'logs');break;
    case 'dismiss-callout':callouts.dismiss(button.dataset.id,{restoreFocus:true});break;
    case 'close-modal':closeModal();break;
    case 'launch':primary(app);break;
    case 'retry':startApp(app);break;
    case 'open-entry':openEntry(app);break;
    case 'stop':if(requireConnection())stopApp(app);break;
    case 'restart':restartApp(app);break;
    case 'stop-all':stopAll();break;
    case 'pause':pauseWake();break;
    case 'resume':resumeWake();break;
    case 'reset':reset();break;
    case 'empty':reset(true);break;
    case 'free-port':portBusy=false;notify('示例：外部进程已释放端口 5173');update();break;
    case 'orphan-demo':orphanDemo=!orphanDemo;notify(orphanDemo?'下次全部停止时，会报告一个身份无法确认的遗留进程':'已关闭遗留进程示例');break;
    case 'disconnect':
      if(connected)offlineApps=structuredClone(apps);connected=!connected;
      activity.setOffline(connected?null:new Date().toLocaleTimeString('zh-CN',{hour12:false,hour:'2-digit',minute:'2-digit'}));
      update();break;
    case 'copy-command':{
      const command=app.failure&&explainFailure(app,app.failure).suggestion?.command;if(!command)break;
      try{await navigator.clipboard.writeText(command);}catch{}
      event(`复制建议命令 ${command}`);
      // Copying reports in place; the next step is the user's, in their own terminal.
      if(callouts.has(app.id))callouts.show(app.id,{tone:'info',title:`已复制 ${command}`,body:'在项目目录中运行它，完成后再点磁贴重试。',actions:[{...ACT.terminal,primary:true}]});
      else notify(`已复制 ${command}`);
      break;
    }
    case 'stop-holder-start':{
      const holder=getApp(app.failure?.holderId);if(!holder||!requireConnection())break;
      callouts.show(app.id,{tone:'progress',title:`正在停止 ${holder.name}`,body:`停止后启动 ${app.name}。`});
      event(`停止 ${holder.name} 以启动 ${app.name}`);
      stopApp(holder,()=>startApp(app));break;
    }
    case 'activity-toggle':activity.toggle();break;
    case 'activity-close':activity.close();break;
    case 'activity-notice':activity.runNotice();break;
    case 'activity-journal':activity.close();openSection('journal');break;
    case 'close-window':showModal('关闭窗口之后',`<p>桌面窗口会退出，后台及正在运行的应用继续保留。</p><p class="note">需要结束应用时，使用顶栏的“全部停止”，或在分组上批量停止。默认不在托盘常驻，也不发送系统通知。</p>`,`<button class="primary" data-action="close-modal">知道了</button>`);break;
    case 'lens':{const next=lens===button.dataset.lens?null:button.dataset.lens;if(view!=='home'){view='home';lens=next;render();}else setLens(next);break;}
    case 'toggle-group':morph(()=>toggleGroup(button.dataset.group));break;
    case 'group-start':groupStart(button.dataset.group);break;
    case 'group-stop':groupStop(button.dataset.group);break;
    case 'add-menu':{const rect=button.getBoundingClientRect();menu.open(addMenu(),{x:rect.left,y:rect.bottom+6},button);break;}
    case 'commit-group':commitGroup(button);break;
    case 'reload-blip':{
      // A hot reload keeps RUNNING; only the readiness hint changes for the grace period.
      const target=apps.find(a=>a.state==='RUNNING'&&a.port);if(!target){notify('没有正在运行、带端口的示例应用');break;}
      target.ready=false;event(`${target.name} 端口暂不可用（热重载）`);update();
      setTimeout(()=>{target.ready=true;event(`${target.name} 端口已恢复`);update();},2600);break;
    }
    case 'service-fail':{
      // One service exits on its own; the instance fails but keeps the survivors for
      // diagnosis. The user did not just act, so this shows on the tile, not in a callout.
      const target=apps.find(a=>a.state==='RUNNING'&&a.services.length>1);if(!target){notify('请先启动一个多服务示例应用（Notes）');break;}
      const dead=target.services.at(-1);
      recordFailure(target,{code:'UNKNOWN',phase:'running',service:dead.name,exitCode:1,live:true,mark:1,tail:['GET /api/notes 200 12ms','TypeError: Cannot read properties of undefined (reading \'id\')','    at listNotes (src/routes.ts:42:18)']});
      event(`${target.name} 的 ${dead.name} 服务退出，其余服务保留`);update();break;
    }
    default:await handleFlow(action,app,button);
  }
});
document.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.isComposing&&e.keyCode!==229&&e.target.id==='group-name'){e.preventDefault();modal.querySelector('[data-action=commit-group]')?.click();}});
// Filtering while typing is instant: a transition per keystroke would lag input and
// paint tile snapshots above the search results for its duration.
searchInput.addEventListener('input',e=>{query=e.target.value;if(view!=='home')view='home';render();});
// Like a Start screen: a vertical wheel over the canvas travels horizontally while it can.
document.addEventListener('wheel',e=>{
  const board=e.target.closest?.('.board');if(!board||Math.abs(e.deltaX)>Math.abs(e.deltaY))return;
  const max=board.scrollWidth-board.clientWidth;
  if(max<=0||(e.deltaY<0&&board.scrollLeft<=0)||(e.deltaY>0&&board.scrollLeft>=max))return;
  e.preventDefault();board.scrollLeft+=e.deltaY;
},{passive:false});
document.addEventListener('scroll',e=>{if(e.target.classList?.contains('board'))syncEdges();},true);
addEventListener('resize',()=>{layoutRows();syncEdges();});
// The detail panel narrows the canvas without a window resize.
new ResizeObserver(()=>syncEdges()).observe(main);
applyTheme();
render();
