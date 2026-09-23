'use strict';
// One tile component in two sizes. Identity (hue, icon or monogram) never encodes state.
// State is drawn by three channels only: the fill (identity color while an instance is
// live, neutral at rest), one corner flag for exceptions (failed / needs review), and
// motion while something is changing. The only visible text is the application name;
// details live in the shared tooltip, aria-label, the context menu and the drawer.
const STATE_LABEL={running:'运行中',starting:'正在启动',stopping:'正在停止',failed:'启动失败',stopped:'已停止',review:'待确认声明',missing:'源码缺失'};
const SERVICE_LABEL={running:'运行中',starting:'正在启动',stopping:'正在停止',failed:'已退出',stopped:'未运行'};
function tileState(app){return app.review?'review':app.problem==='source'?'missing':app.state.toLowerCase();}
// A failed instance may still own live services (M1 keeps them for diagnosis).
const liveServices=app=>(app.services||[]).filter(s=>['running','starting'].includes(s.state));
const isPartial=app=>app.state==='FAILED'&&liveServices(app).length>0;
// "Live" means processes AppHost owns are running now: that alone earns the identity fill.
const isLive=app=>['RUNNING','STOPPING'].includes(app.state)||isPartial(app);
// Identity hues come from project_id, so installations of one project share a color.
// Reds, oranges and ambers (roughly 0–100°) are reserved for failure and warning.
const IDENTITY_HUES=[115,145,170,195,220,245,270,295,320];
function identityHue(projectId){let x=0x811c9dc5;for(const c of String(projectId))x=Math.imul(x^c.charCodeAt(0),0x01000193)>>>0;return IDENTITY_HUES[x%IDENTITY_HUES.length];}
const idStyle=app=>`--hue:${app.hue}`;
// Found icons are rare in real projects, so the monogram is the common case, not a fallback.
function glyph(app){
  if(app.icon)return icon(app.icon);
  const first=[...String(app.name).trim()][0]||'?';
  return `<b class="mono${/[㐀-鿿]/.test(first)?' cjk':''}">${escape(first.toUpperCase())}</b>`;
}
function tileHint(app){
  const state=tileState(app);
  if(state==='failed')return isPartial(app)?'完整重启':'重新启动';
  if(state==='review'||state==='missing')return '查看详情';
  if(state==='running')return app.port?'打开应用':'查看日志';
  return state==='stopped'?'启动应用':STATE_LABEL[state];
}
// The one-sentence reason travels with the state into the tooltip and accessible name.
const reasonOf=app=>tileState(app)==='failed'&&app.failure?explainFailure(app,app.failure).title:'';
function tileLabel(app){return `${app.name}，${STATE_LABEL[tileState(app)]}${reasonOf(app)?`：${reasonOf(app)}`:''}${isPartial(app)?'，仍有服务运行':''}${app.ready===false?'，端口暂不可用':''}，${tileHint(app)}。右键或菜单键查看更多操作`;}
function tileTip(app){
  const services=app.services||[];
  const detail=services.length>1?services.map(s=>`${s.name} ${SERVICE_LABEL[s.state]}`).join(' · '):app.port?`:${app.port}`:'';
  return `${app.name} · ${reasonOf(app)||STATE_LABEL[tileState(app)]}${app.ready===false?' · 端口暂不可用':''}${detail?` · ${detail}`:''}`;
}
// Multi-service apps split the badge ring into one arc per service, so a partial
// failure reads at a glance. The ring appears only while some service is not at rest;
// segment colors are tokens, so they read on both the neutral and the identity fill.
function serviceRing(app){
  const services=app.services||[];if(services.length<2||services.every(s=>s.state==='stopped'))return '';
  const color={running:'var(--seg-on)',starting:'var(--seg-wait)',stopping:'var(--seg-wait)',failed:'var(--fail)',stopped:'var(--seg-off)'};
  const step=360/services.length,gap=12;
  return `conic-gradient(${services.map((s,i)=>`transparent ${i*step}deg ${i*step+gap/2}deg,${color[s.state]} ${i*step+gap/2}deg ${(i+1)*step-gap/2}deg,transparent ${(i+1)*step-gap/2}deg ${(i+1)*step}deg`).join(',')})`;
}
function badge(app){
  const ring=serviceRing(app);
  return `<span class="badge${ring?' multi':''}" aria-hidden="true">${glyph(app)}<i class="svc-ring" style="--segments:${ring||'none'}"></i></span>`;
}
function appTile(app,{size='medium',branch=false}={}) {
  const small=size==='small';
  // Transition names are per rendered copy; the recent group repeats installations.
  return `<article class="tile${small?' small':''}" draggable="true" data-installation="${app.id}" data-state="${tileState(app)}" data-live="${isLive(app)}" data-partial="${isPartial(app)}" data-ready="${app.ready!==false}" style="${idStyle(app)};view-transition-name:${small?'r':'t'}-${app.id}">
    <button class="tile-main" data-action="launch" data-id="${app.id}" aria-label="${escape(tileLabel(app))}" data-tip="${escape(tileTip(app))}" data-tip-delay="${small?80:700}">
      ${badge(app)}<span class="tile-flag" aria-hidden="true"></span><span class="tile-dot" aria-hidden="true"></span>${small?'':`<span class="tile-name">${escape(app.name)}${branch?`<small>${escape(app.branch)}</small>`:''}</span><span class="tile-hint" aria-hidden="true">${icon(primaryIcon(app))}</span>`}
    </button>
  </article>`;
}
// A collapsed group becomes one folder tile; its cells mirror member states.
const folderCell=app=>`<span class="folder-cell" data-installation="${app.id}" data-state="${tileState(app)}" data-live="${isLive(app)}" data-partial="${isPartial(app)}" style="${idStyle(app)}">${glyph(app)}</span>`;
// Patch every rendered copy of an installation without rebuilding the board,
// so state changes animate in place and keyboard focus never jumps.
function syncTiles(app){
  for(const el of document.querySelectorAll(`[data-installation="${CSS.escape(app.id)}"]`)){
    el.dataset.state=tileState(app);
    el.dataset.live=String(isLive(app));
    el.dataset.partial=String(isPartial(app));
    el.dataset.ready=String(app.ready!==false);
    const main=el.querySelector('.tile-main');
    if(main){main.setAttribute('aria-label',tileLabel(app));main.dataset.tip=tileTip(app);}
    const hint=el.querySelector('.tile-hint');if(hint)hint.innerHTML=icon(primaryIcon(app));
    const ring=el.querySelector('.svc-ring');
    if(ring){const segments=serviceRing(app);ring.parentElement.classList.toggle('multi',!!segments);ring.style.setProperty('--segments',segments||'none');}
  }
}
