'use strict';
// Design data only: no daemon transport, process execution or filesystem writes.
const seed = [
  {id:'paper',name:'Paper Reader',branch:'main',color:'#a65335',icon:'book',state:'FAILED',problem:'port',port:5173},
  {id:'vision',name:'Vision Lab',branch:'feat / camera',color:'#314e71',icon:'camera',state:'STOPPED',review:true,port:8080},
  {id:'canvas',name:'Local Canvas',branch:'main',color:'#176b63',icon:'canvas',state:'RUNNING',port:5174},
  {id:'whisper',name:'Whisper Studio',branch:'main',color:'#685369',icon:'wave',state:'STOPPED',port:7860},
  {id:'notes',name:'Notes',branch:'main',color:'#365f91',icon:'note',state:'RUNNING',port:3001},
  {id:'assets',name:'Asset Board',branch:'main',color:'#407064',icon:'grid',state:'RUNNING',port:3000},
  {id:'focus',name:'Focus Timer',branch:'main',color:'#84622e',icon:'clock',state:'STOPPED',port:null},
  {id:'atlas',name:'Atlas Control',branch:'dev',color:'#4a606a',icon:'sliders',state:'STOPPED',port:8081}
].map((app,index)=>({...app,projectId:`00000000-0000-4000-8000-${String(index+1).padStart(12,'0')}`}));
const main=document.querySelector('#main'),detail=document.querySelector('#detail'),modal=document.querySelector('#modal');
let apps=structuredClone(seed), view='home', section='apps', query='', connected=true, paused=false, portBusy=true;
let selected=null, detailTab='overview', events=[], nextId=0, toastTimer;
const logReads=new Map();
let offlineApps=[], preferences={login:false,tray:false,pathSaved:false};
const displayedApps = () => connected ? apps : offlineApps;
const active = app => ['RUNNING','STARTING','STOPPING'].includes(app.state);
const attention = app => app.state==='FAILED' || app.review || app.problem==='source';
const stateLabel = app => app.review?'待确认声明':({RUNNING:'运行中',STARTING:'正在启动',STOPPING:'正在停止',FAILED:'启动失败',STOPPED:'已停止'}[app.state]);
function render() {
  const focused=document.activeElement;
  const restore=main.contains(focused)&&focused.matches('button[data-action]');
  renderContent();
  // Replacing a view must not drop keyboard focus to the document body.
  if(restore){const replacement=[...main.querySelectorAll('button[data-action]')].find(button=>['action','id','filter','section'].every(key=>button.dataset[key]===focused.dataset[key]));if(replacement)replacement.focus();else{main.tabIndex=-1;main.focus();}}
}
function renderContent() {
  const display=displayedApps();
  document.querySelector('#connection-status').textContent=connected?'● 已连接':'○ 未连接';
  document.querySelector('#connection-banner').innerHTML=!connected?`<div class="banner"><span>连接已断开，显示上次状态。</span><button data-action="disconnect">重新连接</button></div>`:paused?`<div class="banner"><span>自动唤醒已暂停</span><button data-action="resume">恢复</button></div>`:'';
  if(view==='manage'){renderManagement();return;}
  const matches=display.filter(a=>(a.name+' '+a.branch).toLowerCase().includes(query.toLowerCase()));
  // Preserve installation order through every lifecycle transition. Search is a temporary view.
  const content=!display.length?`<div class="empty">${icon('folder')}<h2>添加第一个应用</h2><button class="primary" data-action="import">选择目录</button></div>`:!matches.length?`<div class="empty"><h2>没有匹配的应用</h2><button data-action="clear-search">清除搜索</button></div>`:`<div class="tiles">${matches.map(app=>appTile(app,stateLabel(app),display.filter(other=>other.name===app.name).length>1)).join('')}${!query?'<button class="add-tile" data-action="import" aria-label="添加应用"><span aria-hidden="true">＋</span><span>添加应用</span></button>':''}</div>`;
  main.innerHTML=`<div class="board"><h1 class="sr-only">应用</h1>${content}</div>`;
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
  if(detailTab==='logs')body=logsPanel(app,connected,logReads.get(app.id));
  else if(detailTab==='terminal')body=terminalPanel(app);
  else if(detailTab==='manifest')body=`<p class="muted">项目内 · ${escape(app.path || `~/Projects/${app.id}`)}/apphost.toml</p><div style="height:12px"></div><pre>${escape(declaration(app))}</pre><button class="full" data-action="edit" data-id="${app.id}">编辑启动声明</button>`;
  else if(app.review)body=`<div class="reason"><code>REVIEW_REQUIRED</code><h3>启动声明有变更，需要你确认</h3><p>当前应用未启动。确认的是启动声明，不代表脚本及依赖内容可信。</p></div>${declarationDiff(app,declaration(app))}<button class="primary full" data-action="review" data-id="${app.id}">复核并确认</button><p class="note">“待确认”是提示；当前运行状态仍为已停止。</p>`;
  else if(app.state==='FAILED')body=`<div class="reason"><code>PORT_IN_USE</code><h3>端口 5173 已被占用</h3><p>${escape(app.name)} 还没有启动。先释放端口，或修改此应用的端口声明。</p></div><dl class="evidence"><dt>占用进程</dt><dd><strong>node · PID 18273</strong></dd><dt>管理归属</dt><dd>外部进程</dd><dt>检查结果</dt><dd>已找到一个监听者</dd></dl><button class="primary full" data-action="retry" data-id="${app.id}">重试启动</button><div class="row-actions" style="margin-top:12px"><button data-action="edit" data-id="${app.id}">修改启动声明</button><button data-action="diagnostic" data-id="${app.id}">预览诊断信息</button></div><p class="note">AppHost 不会结束这个外部进程。你可以将脱敏后的诊断信息交给 agent 排查。</p>`;
  else body=`<dl class="evidence"><dt>当前状态</dt><dd>${stateLabel(app)}</dd><dt>项目分支</dt><dd>${escape(app.branch)}</dd><dt>服务</dt><dd>web${app.port?' · 端口 '+app.port:' · 无浏览器入口'}</dd><dt>项目目录</dt><dd>${escape(app.path || `~/Projects/${app.id}`)}</dd></dl><button class="primary full" data-action="launch" data-id="${app.id}" ${['STARTING','STOPPING'].includes(app.state)?'disabled':''}>${app.state==='RUNNING'?(app.port?'打开应用':'查看运行日志'):'启动应用'}</button>${active(app)?`<div class="row-actions" style="margin-top:12px"><button data-action="restart" data-id="${app.id}">完整重启</button><button data-action="stop" data-id="${app.id}">停止应用</button></div>`:''}`;
  detail.innerHTML=`<div class="dialog-heading"><div class="detail-identity"><div class="app-icon" style="--tile:${app.color}">${icon(app.icon)}</div><div><h2 id="detail-title">${escape(app.name)}</h2><p class="subtitle">${escape(app.branch)} · ${stateLabel(app)}</p></div></div><button class="close" data-action="close-detail" aria-label="关闭应用详情">×</button></div><nav class="detail-tabs" aria-label="应用详情">${[['overview','概览'],['logs','日志'],['terminal','命令行'],['manifest','声明']].map(([id,label])=>`<button class="${detailTab===id?'active':''}" data-action="detail-tab" data-tab="${id}" aria-current="${detailTab===id?'page':'false'}">${label}</button>`).join('')}</nav><div class="dialog-body">${body}<div class="divider"></div><div class="row-actions"><button data-action="relink" data-id="${app.id}">重新关联路径</button><button class="text-button" data-action="detach" data-id="${app.id}">解除登记</button></div></div>`;
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
    else{app.state='RUNNING';app.problem=null;event(`${app.name} 已就绪`);if(connected)previewOpen(app);}
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
function reset(empty=false){logReads.clear();document.querySelector('#toast').classList.remove('show');preferences={login:false,tray:false,pathSaved:false};apps=empty?[]:structuredClone(seed);events=[];paused=false;connected=true;portBusy=true;query='';view='home';document.querySelector('#search').value='';detail.close();modal.close();render();}
document.addEventListener('click',async e=>{
  const button=e.target.closest('button[data-action]');if(!button||button.disabled)return;
  const action=button.dataset.action, app=getApp(button.dataset.id||selected);
  switch(action){
    case 'home':view='home';render();break;
    case 'manage':view='manage';section='apps';render();break;
    case 'section':section=button.dataset.section;render();break;
    case 'clear-search':query='';document.querySelector('#search').value='';render();break;
    case 'details':openDetail(app);break;
    case 'refresh-logs':if(requireConnection()){logReads.set(app.id,new Date().toLocaleTimeString('zh-CN',{hour12:false}));renderDetail();}break;
    case 'open-terminal':showModal('系统终端入口',`<p>在项目目录打开系统终端：</p><pre>${escape(app.path || `~/Projects/${app.id}`)}</pre><p class="note">交互演示：未打开真实终端，也未执行命令。</p>`,`<button class="primary" data-action="close-modal">返回</button>`);break;
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
