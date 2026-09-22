'use strict';
// One tile component in two sizes. Identity (fill color, icon) never encodes state:
// state lives only in data-state / data-partial / data-ready and is drawn by ring, dot,
// veil and motion. The only visible text is the application name; details live in the
// shared tooltip, aria-label, the context menu and the drawer.
const STATE_LABEL={running:'运行中',starting:'正在启动',stopping:'正在停止',failed:'启动失败',stopped:'已停止',review:'待确认声明',missing:'源码缺失'};
const SERVICE_LABEL={running:'运行中',starting:'正在启动',stopping:'正在停止',failed:'已退出',stopped:'未运行'};
function tileState(app){return app.review?'review':app.problem==='source'?'missing':app.state.toLowerCase();}
// A failed instance may still own live services (M1 keeps them for diagnosis).
const liveServices=app=>(app.services||[]).filter(s=>['running','starting'].includes(s.state));
const isPartial=app=>app.state==='FAILED'&&liveServices(app).length>0;
function tileHint(app){
  const state=tileState(app);
  if(state==='failed')return isPartial(app)?'完整重启':'重新启动';
  if(state==='review'||state==='missing')return '查看详情';
  if(state==='running')return app.port?'打开应用':'查看日志';
  return state==='stopped'?'启动应用':STATE_LABEL[state];
}
function tileLabel(app){return `${app.name}，${STATE_LABEL[tileState(app)]}${isPartial(app)?'，仍有服务运行':''}${app.ready===false?'，端口暂不可用':''}，${tileHint(app)}。右键或菜单键查看更多操作`;}
function tileTip(app){
  const services=app.services||[];
  const detail=services.length>1?services.map(s=>`${s.name} ${SERVICE_LABEL[s.state]}`).join(' · '):app.port?`:${app.port}`:'';
  return `${app.name} · ${STATE_LABEL[tileState(app)]}${app.ready===false?' · 端口暂不可用':''}${detail?` · ${detail}`:''}`;
}
// Multi-service apps split the badge ring into one arc per service, so a partial
// failure reads at a glance without any text. Single-service apps keep the CSS ring.
function serviceRing(app){
  const services=app.services||[];if(services.length<2)return '';
  const color={running:'var(--seg-on)',starting:'var(--seg-wait)',stopping:'var(--seg-wait)',failed:'var(--fail)',stopped:'var(--seg-off)'};
  const step=360/services.length,gap=10;
  return `conic-gradient(${services.map((s,i)=>`transparent ${i*step}deg ${i*step+gap/2}deg,${color[s.state]} ${i*step+gap/2}deg ${(i+1)*step-gap/2}deg,transparent ${(i+1)*step-gap/2}deg ${(i+1)*step}deg`).join(',')})`;
}
function badge(app){
  const ring=serviceRing(app);
  return `<span class="badge${ring?' multi':''}" aria-hidden="true">${icon(app.icon)}<i class="svc-ring" style="--segments:${ring||'none'}"></i></span>`;
}
function appTile(app,{size='medium',branch=false}={}) {
  const small=size==='small';
  // Transition names are per rendered copy; the recent group repeats installations.
  return `<article class="tile${small?' small':''}" draggable="true" data-installation="${app.id}" data-state="${tileState(app)}" data-partial="${isPartial(app)}" data-ready="${app.ready!==false}" style="--tile:${app.color};view-transition-name:${small?'r':'t'}-${app.id}">
    <button class="tile-main" data-action="launch" data-id="${app.id}" aria-label="${escape(tileLabel(app))}" data-tip="${escape(tileTip(app))}" data-tip-delay="${small?80:700}">
      ${badge(app)}<span class="tile-dot" aria-hidden="true"></span>${small?'':`<span class="tile-name">${escape(app.name)}${branch?`<small>${escape(app.branch)}</small>`:''}</span><span class="tile-arrow" aria-hidden="true">${icon('arrow')}</span>`}
    </button>
  </article>`;
}
// A collapsed group becomes one folder tile; its cells mirror member states.
const folderCell=app=>`<span class="folder-cell" data-installation="${app.id}" data-state="${tileState(app)}" data-partial="${isPartial(app)}" style="--tile:${app.color}">${icon(app.icon)}</span>`;
// Patch every rendered copy of an installation without rebuilding the board,
// so state changes animate in place and keyboard focus never jumps.
function syncTiles(app){
  for(const el of document.querySelectorAll(`[data-installation="${CSS.escape(app.id)}"]`)){
    el.dataset.state=tileState(app);
    el.dataset.partial=String(isPartial(app));
    el.dataset.ready=String(app.ready!==false);
    const main=el.querySelector('.tile-main');
    if(main){main.setAttribute('aria-label',tileLabel(app));main.dataset.tip=tileTip(app);}
    const ring=el.querySelector('.svc-ring');
    if(ring){const segments=serviceRing(app);ring.parentElement.classList.toggle('multi',!!segments);ring.style.setProperty('--segments',segments||'none');}
  }
}
