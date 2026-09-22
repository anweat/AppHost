'use strict';
// Design data only: no daemon transport, process execution or filesystem writes.
const seed = [
  {id:'paper',name:'Paper Reader',branch:'main',color:'#a65335',icon:'book',state:'FAILED',problem:'port',port:5173,recent:4},
  {id:'vision',name:'Vision Lab',branch:'feat / camera',color:'#314e71',icon:'camera',state:'STOPPED',review:true,port:8080,recent:0},
  {id:'canvas',name:'Local Canvas',branch:'main',color:'#176b63',icon:'canvas',state:'RUNNING',port:5174,recent:8},
  {id:'whisper',name:'Whisper Studio',branch:'main',color:'#685369',icon:'wave',state:'STOPPED',port:7860,recent:7},
  {id:'notes',name:'Notes',branch:'main',color:'#365f91',icon:'note',state:'RUNNING',port:3001,recent:6},
  {id:'assets',name:'Asset Board',branch:'main',color:'#407064',icon:'grid',state:'RUNNING',port:3000,recent:0},
  {id:'focus',name:'Focus Timer',branch:'main',color:'#84622e',icon:'clock',state:'STOPPED',port:null,recent:0},
  {id:'atlas',name:'Atlas Control',branch:'dev',color:'#4a606a',icon:'sliders',state:'STOPPED',port:8081,recent:0}
].map((app,index)=>({...app,projectId:`00000000-0000-4000-8000-${String(index+1).padStart(12,'0')}`}));
const paths = {
 book:'M4 4h7c2 0 2 2 2 2s0-2 2-2h5v16h-5c-2 0-2 1-2 1s0-1-2-1H4V4Zm9 2v15',
 camera:'M3 7h5l2-3h4l2 3h5v13H3V7Zm13 6a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z',
 canvas:'M3 3h18v18H3V3Zm0 13 6-6 4 4 3-3 5 5M16 7h.01',
 wave:'M3 10v4m4-8v12m5-16v20m5-16v12m4-8v4',
 note:'M5 3h14v18H5V3Zm4 5h6m-6 4h6m-6 4h4',
 grid:'M3 3h7v7H3V3Zm11 0h7v7h-7V3ZM3 14h7v7H3v-7Zm11 0h7v7h-7v-7Z',
 clock:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-6v6l4 2',
 sliders:'M6 3v7m0 4v7m6-18v3m0 4v11m6-18v11m0 4v3M3 10h6v4H3v-4Zm6-4h6v4H9V6Zm6 8h6v4h-6v-4',
 folder:'M3 6h7l2 3h9v12H3V6Z',activity:'M3 12h4l3-8 4 16 3-8h4'
};
const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.grid}"/></svg>`;
const escape = value => String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const main=document.querySelector('#main'),detail=document.querySelector('#detail'),modal=document.querySelector('#modal');
let apps=structuredClone(seed), view='home', section='apps', filter='all', query='', connected=true, paused=false, portBusy=true;
let selected=null, detailTab='overview', events=[], nextId=0, recent=10, toastTimer;
let offlineApps=[], preferences={login:false,tray:false,pathSaved:false};
const displayedApps = () => connected ? apps : offlineApps;
const active = app => ['RUNNING','STARTING','STOPPING'].includes(app.state);
const attention = app => app.state==='FAILED' || app.review || app.problem==='source';
const stateLabel = app => app.review?'待确认声明':({RUNNING:'运行中',STARTING:'正在启动',STOPPING:'正在停止',FAILED:'启动失败',STOPPED:'已停止'}[app.state]);
function tile(app) {
  const hint=app.review?'启动声明已修改':app.state==='FAILED'&&app.problem==='port'?'端口 5173 已被占用':app.state==='RUNNING'?'打开应用':app.state==='STARTING'?'正在等待服务就绪':app.state==='STOPPING'?'正在停止所有服务':'点击启动';
  return `<article class="tile" style="--tile:${app.color}"><button class="tile-main" data-action="launch" data-id="${app.id}" aria-label="${escape(app.name)}，${stateLabel(app)}，${hint}"><span class="tile-top">${icon(app.icon)}<span class="tile-status"><span class="dot"></span>${stateLabel(app)}</span></span><h3>${escape(app.name)}</h3><span class="tile-meta">${escape(app.branch)}</span><span class="tile-bottom"><span>${hint}</span><b aria-hidden="true">↗</b></span></button><button class="tile-menu" data-action="details" data-id="${app.id}" aria-label="${escape(app.name)} 详情">···</button></article>`;
}
function group(title,subtitle,items,isAttention=false) {
  return items.length?`<section class="group ${isAttention?'attention':''}"><div class="group-heading"><h2>${isAttention?'<span class="dot"></span>':''}${title}</h2><span>${subtitle}</span></div><div class="tiles">${items.map(tile).join('')}</div></section>`:'';
}
function render() {
  const focused=document.activeElement;
  const restore=main.contains(focused)&&focused.matches('button[data-action]');
  renderContent();
  // Replacing a view must not drop keyboard focus to the document body.
  if(restore){const replacement=[...main.querySelectorAll('button[data-action]')].find(button=>['action','id','filter','section'].every(key=>button.dataset[key]===focused.dataset[key]));if(replacement)replacement.focus();else{main.tabIndex=-1;main.focus();}}
}
function renderContent() {
  const display=displayedApps();
  const running=display.filter(a=>a.state==='RUNNING').length, needs=display.filter(attention).length;
  document.querySelector('#connection-status').textContent=connected?'● 后台已连接':'○ 后台连接已断开';
  document.querySelector('#connection-banner').innerHTML=!connected?`<div class="banner"><span>后台连接已断开 · 当前显示上次状态，无法确认应用是否仍在运行。</span><button data-action="disconnect">重新连接</button></div>`:paused?`<div class="banner"><span>自动唤醒已暂停。你仍然可以点击磁贴，单独启动应用。</span><button data-action="resume">恢复按需唤醒</button></div>`:'';
  if(view==='manage'){renderManagement();return;}
  const matches=display.filter(a=>(a.name+' '+a.branch).toLowerCase().includes(query.toLowerCase())).filter(a=>filter==='all'||(filter==='running'?a.state==='RUNNING':attention(a)));
  const needsApps=matches.filter(attention), recentApps=matches.filter(a=>!attention(a)&&a.recent).sort((a,b)=>b.recent-a.recent), remaining=matches.filter(a=>!attention(a)&&!a.recent);
  main.innerHTML=`<div class="board"><div class="page-heading"><div><div class="eyebrow">YOUR LOCAL WORKSPACE</div><h1>应用</h1><p class="subtitle">${apps.length?`${running} 个正在运行${needs?` · ${needs} 个需要处理`:''}`:'从一个目录开始，让本地应用随时可用。'}</p></div><button class="primary" data-action="import">＋ 添加目录</button></div>${apps.length?`<nav class="filters" aria-label="应用筛选">${[['all','全部应用',apps.length],['running','运行中',running],['attention','需要处理',needs]].map(([id,label,count])=>`<button data-action="filter" data-filter="${id}" class="${filter===id?'active':''}" aria-pressed="${filter===id}">${label}<span>${count}</span></button>`).join('')}</nav>`:''}${!apps.length?`<div class="empty">${icon('folder')}<h2>把你的第一个应用放在这里</h2><p>选择项目所在目录，检查启动声明。<br>导入后，点击磁贴即可启动。</p><button class="primary" data-action="import">选择目录</button><p class="muted">只扫描你选择的目录，不会自动扫描用户文件。</p></div>`:!matches.length?`<div class="empty">${icon('grid')}<h2>没有匹配的应用</h2><p>试试其他名称或分支，或清除当前筛选。</p><button data-action="clear-search">清除筛选</button></div>`:group('需要处理','查看原因，再决定下一步',needsApps,true)+group('最近使用','继续上次的工作',recentApps)+group('其他应用',`${remaining.length} 个应用`,remaining)}</div>`;
}
function renderManagement() {
  const nav=[['apps','grid','应用管理'],['journal','activity','操作记录'],['settings','sliders','环境与设置']];
  let content='';
  if(section==='apps')content=`<div class="page-heading"><div><div class="eyebrow">MANAGE</div><h1>应用管理</h1><p class="subtitle">管理启动声明、项目路径和登记。</p></div><button class="primary" data-action="import">添加目录</button></div>${apps.length?displayedApps().filter(a=>(a.name+' '+a.branch).toLowerCase().includes(query.toLowerCase())).map(a=>`<div class="app-row"><div class="app-icon" style="--tile:${a.color}">${icon(a.icon)}</div><div><strong>${escape(a.name)}</strong><small>${escape(a.path || `~/Projects/${a.id}`)} · ${escape(a.branch)}</small></div><span class="state ${a.state.toLowerCase()}">${stateLabel(a)}</span><button data-action="details" data-id="${a.id}" aria-label="管理 ${escape(a.name)}">管理</button></div>`).join(''):'<p class="muted">暂无已登记应用。选择目录以添加。</p>'}`;
  if(section==='journal')content=`<div class="page-heading"><div><div class="eyebrow">ACTIVITY</div><h1>操作记录</h1><p class="subtitle">本次原型会话的操作，不包含真实运行记录。</p></div></div>${events.length?events.map(e=>`<div class="event"><time>${e.time}</time><span>${escape(e.text)}</span></div>`).join(''):'<div class="empty"><h2>还没有操作</h2><p>启动、停止、导入和声明确认会显示在这里。</p></div>'}`;
  if(section==='settings')content=`<div class="page-heading"><div><div class="eyebrow">PREFERENCES</div><h1>环境与设置</h1><p class="subtitle">默认保持安静，需要时再运行。</p></div></div><div class="setting"><div><h3>运行环境 PATH</h3><p>所有入口共用同一环境。仅在你主动选择时采集 shell 配置。</p></div><button data-action="environment">查看与配置</button></div><label class="setting"><div><h3>登录时启动 AppHost 后台</h3><p>只启动管理后台，不自动启动应用。</p></div><input type="checkbox" data-setting="login" ${preferences.login?'checked':''} aria-label="登录时启动后台"></label><label class="setting"><div><h3>托盘常驻</h3><p>默认关闭。关闭窗口不影响后台服务。</p></div><input type="checkbox" data-setting="tray" ${preferences.tray?'checked':''} aria-label="托盘常驻"></label><div class="setting"><div><h3>自动唤醒</h3><p>${paused?'已暂停；手动打开某个应用仍然有效。':'仅允许满足规则的浏览器导航触发，不接受任意网络请求。'}</p></div><button data-action="${paused?'resume':'pause'}">${paused?'恢复按需唤醒':'暂停按需唤醒'}</button></div><p class="note">设置在此原型中只演示交互，刷新后恢复默认。</p>`;
  main.innerHTML=`<div class="management"><nav class="sidebar" aria-label="管理导航"><h2>管理工作区</h2>${nav.map(([id,i,label])=>`<button class="${section===id?'active':''}" data-action="section" data-section="${id}" ${section===id?'aria-current="page"':''}>${icon(i)}${label}</button>`).join('')}</nav><div class="manage-content">${content}</div></div>`;
}
function toast(message){
  const dialog=modal.open?modal:detail.open?detail:null;
  if(dialog){let feedback=dialog.querySelector('[data-feedback]');if(!feedback){feedback=document.createElement('p');feedback.dataset.feedback='';feedback.className='note';feedback.setAttribute('role','status');dialog.querySelector('.dialog-body').append(feedback);}feedback.textContent=message;return;}
  const el=document.querySelector('#toast');el.textContent=message;el.classList.add('show');clearTimeout(toastTimer);toastTimer=setTimeout(()=>el.classList.remove('show'),3200);}
function event(text){events.unshift({text,time:new Date().toLocaleTimeString('zh-CN',{hour12:false})});}
function showModal(title,body,actions=''){modal.innerHTML=`<div class="dialog-heading"><h2 id="modal-title">${title}</h2><button class="close" data-action="close-modal" aria-label="关闭对话框">×</button></div><div class="dialog-body">${body}</div>${actions?`<div class="dialog-actions">${actions}</div>`:''}`;if(!modal.open)modal.showModal();else modal.querySelector('.close').focus();}
function closeModal(){modal.close();if(!detail.open)document.querySelector('#search').focus();}
function update(){render();if(detail.open&&selected)renderDetail();}
function getApp(id){return apps.find(a=>a.id===id);}
document.querySelector('#search').addEventListener('input',e=>{query=e.target.value;render();});
document.addEventListener('keydown',e=>{if(!modal.open&&!detail.open&&(e.metaKey||e.ctrlKey)&&e.key==='k'){e.preventDefault();document.querySelector('#search').focus();}});
render();

function declaration(app){return app.declaration || `schema_version = 1\nproject_id = "${app.projectId}"\nname = "${app.name}"\n\n[[services]]\nname = "web"\ncommand = ${JSON.stringify(app.command ? app.command.split(" ") : ["npm","run","dev"])}\ncwd = "."${app.port?`\nport = ${app.port}`:''}\nstartup_timeout_ms = 30000`;}
function renderDetail(){
  const focusedAction=detail.contains(document.activeElement)?document.activeElement.dataset.action:null;
  const focusedTab=document.activeElement.dataset.tab;
  const app=displayedApps().find(a=>a.id===selected);if(!app){detail.close();return;}
  let body='';
  if(detailTab==='logs')body=`<p class="muted">${escape(app.name)} · 最近输出 · 示例日志</p><div style="height:12px"></div><pre>${escape(app.state==='FAILED'?'[14:32:08] 启动前检查\n[14:32:08] 端口 5173 已被占用\n[14:32:08] 已识别：node · PID 18273\n[14:32:08] 未启动服务；未终止占用进程。':app.state==='STOPPED'?'当前没有活动实例。\n上次运行的日志保留在历史实例中。':'[14:30:02] 正在启动 web\n[14:30:03] 服务监听已就绪\n[14:30:03] GET / 200\n[14:30:06] GET /assets/main.css 200')}</pre><p class="note">原始输出按采集观察时间呈现。发生日志缺口时单独提示，不将缺失的内容显示为完整日志。</p>`;
  else if(detailTab==='manifest')body=`<p class="muted">项目内 · ${escape(app.path || `~/Projects/${app.id}`)}/apphost.toml</p><div style="height:12px"></div><pre>${escape(declaration(app))}</pre><button class="full" data-action="edit" data-id="${app.id}">编辑启动声明</button>`;
  else if(app.review)body=`<div class="reason"><code>REVIEW_REQUIRED</code><h3>启动声明有变更，需要你确认</h3><p>当前应用未启动。确认的是启动声明，不代表脚本及依赖内容可信。</p></div>${declarationDiff(app,declaration(app))}<button class="primary full" data-action="review" data-id="${app.id}">复核并确认</button><p class="note">“待确认”是提示；当前运行状态仍为已停止。</p>`;
  else if(app.state==='FAILED')body=`<div class="reason"><code>PORT_IN_USE</code><h3>端口 5173 已被占用</h3><p>${escape(app.name)} 还没有启动。先释放端口，或修改此应用的端口声明。</p></div><dl class="evidence"><dt>占用进程</dt><dd><strong>node · PID 18273</strong></dd><dt>管理归属</dt><dd>外部进程</dd><dt>检查结果</dt><dd>已找到一个监听者</dd></dl><button class="primary full" data-action="retry" data-id="${app.id}">重试启动</button><div class="row-actions" style="margin-top:12px"><button data-action="edit" data-id="${app.id}">修改启动声明</button><button data-action="diagnostic" data-id="${app.id}">预览诊断信息</button></div><p class="note">AppHost 不会结束这个外部进程。你可以将脱敏后的诊断信息交给 agent 排查。</p>`;
  else body=`<dl class="evidence"><dt>当前状态</dt><dd>${stateLabel(app)}</dd><dt>项目分支</dt><dd>${escape(app.branch)}</dd><dt>服务</dt><dd>web${app.port?' · 端口 '+app.port:' · 无浏览器入口'}</dd><dt>项目目录</dt><dd>${escape(app.path || `~/Projects/${app.id}`)}</dd></dl><button class="primary full" data-action="launch" data-id="${app.id}" ${['STARTING','STOPPING'].includes(app.state)?'disabled':''}>${app.state==='RUNNING'?(app.port?'打开应用':'查看运行日志'):'启动应用'}</button>${active(app)?`<div class="row-actions" style="margin-top:12px"><button data-action="restart" data-id="${app.id}">完整重启</button><button data-action="stop" data-id="${app.id}">停止应用</button></div>`:''}`;
  detail.innerHTML=`<div class="dialog-heading"><div class="detail-identity"><div class="app-icon" style="--tile:${app.color}">${icon(app.icon)}</div><div><h2 id="detail-title">${escape(app.name)}</h2><p class="subtitle">${escape(app.branch)} · ${stateLabel(app)}</p></div></div><button class="close" data-action="close-detail" aria-label="关闭应用详情">×</button></div><nav class="detail-tabs" aria-label="应用详情">${[['overview','概览'],['logs','日志'],['manifest','启动声明']].map(([id,label])=>`<button class="${detailTab===id?'active':''}" data-action="detail-tab" data-tab="${id}" aria-current="${detailTab===id?'page':'false'}">${label}</button>`).join('')}</nav><div class="dialog-body">${body}<div class="divider"></div><div class="row-actions"><button data-action="relink" data-id="${app.id}">重新关联路径</button><button class="text-button" data-action="detach" data-id="${app.id}">解除登记</button></div></div>`;
  if(detail.open&&!modal.open){const replacement=[...detail.querySelectorAll('button[data-action]')].find(button=>button.dataset.action===focusedAction&&button.dataset.tab===focusedTab);(replacement || detail.querySelector('.close')).focus();}
}
function openDetail(app,tabName='overview'){selected=app.id;detailTab=tabName;renderDetail();if(!detail.open)detail.showModal();}
function previewOpen(app){if(!app.port){openDetail(app,'logs');return;}showModal('浏览器入口已就绪',`<div class="empty" style="padding:20px 0">${icon(app.icon)}<h3>${escape(app.name)}</h3><p>正式版本会在系统浏览器中打开此应用。<br>这里仅演示“就绪后打开”的交互。</p><span class="muted">示例服务端口 ${app.port}</span></div>`,`<button class="primary" data-action="close-modal">返回应用面板</button>`);}
function requireConnection(){if(connected)return true;toast('请先重新连接后台，再执行此操作。');return false;}
function startApp(app){
  if(!requireConnection()||['STARTING','STOPPING'].includes(app.state))return;
  if(app.review){openDetail(app);return;}
  if(app.state==='RUNNING'){previewOpen(app);return;}
  const version=app.version=(app.version||0)+1;app.state='STARTING';event(`启动 ${app.name}`);update();
  setTimeout(()=>{
    // Accepted intent is superseded by Stop All, explicit stop or a reset.
    if(!apps.includes(app)||app.version!==version)return;
    if(app.id==='paper'&&app.port===5173&&portBusy){app.state='FAILED';app.problem='port';event(`${app.name} 启动失败：端口被占用`);if(connected)openDetail(app);}
    else{app.state='RUNNING';app.problem=null;app.recent=++recent;event(`${app.name} 已就绪`);if(connected)previewOpen(app);}
    if(connected)update();
  },1400);
}
function stopApp(app,after){
  app.version=(app.version||0)+1;const version=app.version;
  if(!active(app)){app.state='STOPPED';if(after)after();return;}
  app.state='STOPPING';event(`停止 ${app.name}`);update();
  setTimeout(()=>{if(!apps.includes(app)||app.version!==version)return;app.state='STOPPED';event(`${app.name} 已停止`);if(connected)update();if(after)after();},700);
}
function stopAll(){if(!requireConnection())return;paused=true;modal.close();for(const app of apps)stopApp(app);event('全部停止；自动唤醒已暂停');update();toast('正在停止应用，并取消尚未完成的启动。');}
function reset(empty=false){document.querySelector('#toast').classList.remove('show');preferences={login:false,tray:false,pathSaved:false};apps=empty?[]:structuredClone(seed);events=[];paused=false;connected=true;portBusy=true;query='';filter='all';view='home';document.querySelector('#search').value='';detail.close();modal.close();render();}
document.addEventListener('click',async e=>{
  const button=e.target.closest('button[data-action]');if(!button||button.disabled)return;
  const action=button.dataset.action, app=getApp(button.dataset.id||selected);
  switch(action){
    case 'home':view='home';render();break;
    case 'manage':view='manage';section='apps';render();break;
    case 'section':section=button.dataset.section;render();break;
    case 'filter':filter=button.dataset.filter;render();break;
    case 'clear-search':query='';filter='all';document.querySelector('#search').value='';render();break;
    case 'details':openDetail(app);break;
    case 'detail-tab':detailTab=button.dataset.tab;renderDetail();break;
    case 'close-detail':detail.close();break;
    case 'close-modal':closeModal();break;
    case 'launch':if(!requireConnection())break;if(app.state==='FAILED'||app.review)openDetail(app);else startApp(app);break;
    case 'retry':startApp(app);break;
    case 'stop':if(requireConnection())stopApp(app);break;
    case 'restart':if(requireConnection())stopApp(app,()=>startApp(app));break;
    case 'stop-all':stopAll();break;
    case 'pause':if(requireConnection()){paused=true;event('暂停按需唤醒');update();}break;
    case 'resume':if(requireConnection()){paused=false;event('恢复按需唤醒');update();toast('已恢复按需唤醒；不会立即启动应用。');}break;
    case 'reset':reset();break;
    case 'empty':reset(true);break;
    case 'free-port':portBusy=false;toast('示例占用已解除。Paper Reader 仍保留失败状态，请手动重试。');break;
    case 'disconnect':if(connected)offlineApps=structuredClone(apps);connected=!connected;update();break;
    case 'close-window':showModal('关闭窗口之后',`<p>桌面窗口会退出，后台及正在运行的应用继续保留。</p><p class="note">需要结束应用时，请使用“全部停止”。默认不在托盘常驻，也不发送系统通知。</p>`,`<button class="primary" data-action="close-modal">知道了</button>`);break;
    default:await handleFlow(action,app,button);
  }
});
