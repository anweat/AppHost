'use strict';
// In-place callout (5.9): feedback for the action the user just took on one app, anchored
// to that app's tile. It never takes focus: arrival is announced through the live region,
// the tile points at it with aria-describedby, and Tab from the tile steps into it.
// Success fades on its own; failure stays until its condition clears or the user closes it.
// Spec: {tone:'success'|'fail'|'progress'|'info', title, body, meta, progress:0..1,
//        actions:[{label, action, focus, primary}], details, timeout}
const callouts=(()=>{
  const layer=document.createElement('div');layer.className='callout-layer';document.body.append(layer);
  const items=new Map();let frame=0;
  const idFor=appId=>`callout-${appId.replace(/[^a-zA-Z0-9_-]/g,'-')}`;
  const tileMains=appId=>document.querySelectorAll(`.tile[data-installation="${CSS.escape(appId)}"] .tile-main`);
  // Focus returns to the medium tile, not its copy in the recent group.
  const homeTile=appId=>document.querySelector(`.tile[data-installation="${CSS.escape(appId)}"]:not(.small) .tile-main`)||tileMains(appId)[0];
  // Prefer the medium tile, then the folder that holds it, then the recent copy.
  function anchorFor(appId){
    const tiles=[...document.querySelectorAll(`.tile[data-installation="${CSS.escape(appId)}"]`)].filter(t=>t.getClientRects().length);
    const medium=tiles.find(t=>!t.classList.contains('small'));if(medium)return medium;
    const cell=document.querySelector(`.folder-cell[data-installation="${CSS.escape(appId)}"]`);
    if(cell?.getClientRects().length)return cell.closest('.folder');
    return tiles[0]||null;
  }
  // Anchors move with scrolling, folding and the detail panel resizing the canvas, so
  // placement follows every frame while a callout exists (anchor positioning later).
  function place(){
    frame=0;if(!items.size)return;
    const board=document.querySelector('.board'),bounds=board?.getBoundingClientRect();
    for(const [appId,item] of items){
      const anchor=board&&anchorFor(appId),rect=anchor?.getBoundingClientRect();
      const visible=rect&&rect.right>bounds.left+12&&rect.left<bounds.right-12;
      item.el.hidden=!visible;if(!visible)continue;
      const w=item.el.offsetWidth,h=item.el.offsetHeight;
      let top=rect.bottom+10,side='below';
      if(top+h>bounds.bottom-6){top=rect.top-h-10;side='above';}
      if(top<bounds.top+6){top=Math.max(bounds.top+6,rect.top+20);side='over';}
      const left=Math.max(bounds.left+8,Math.min(rect.left,bounds.right-w-8));
      item.el.style.transform=`translate(${Math.round(left)}px,${Math.round(top)}px)`;
      item.el.dataset.side=side;
      item.el.style.setProperty('--arrow',`${Math.max(16,Math.min(w-16,rect.left+Math.min(rect.width/2,34)-left))}px`);
    }
    frame=requestAnimationFrame(place);
  }
  function paint(appId,item){
    const {spec}=item,app=getApp(appId);
    const actions=(spec.actions||[]).map(a=>`<button class="${a.primary?'co-primary':'co-secondary'}" data-action="${a.action}" data-id="${escape(appId)}"${a.focus?` data-focus="${a.focus}"`:''}>${escape(a.label)}</button>`).join('');
    item.el.dataset.tone=spec.tone;
    item.el.setAttribute('aria-label',`${app?.name||''} 的提示`);
    item.el.innerHTML=`<div class="co-head"><span class="co-icon" aria-hidden="true"></span><div class="co-text"><p class="co-title">${escape(spec.title)}</p>${spec.body?`<p class="co-body">${escape(spec.body)}</p>`:''}${spec.meta?`<p class="co-meta">${escape(spec.meta)}</p>`:''}</div>${spec.tone==='success'?'':`<button class="co-close" data-action="dismiss-callout" data-id="${escape(appId)}" aria-label="关闭提示">×</button>`}</div>${spec.progress!==undefined?`<div class="co-progress" aria-hidden="true"><i style="width:${Math.round(spec.progress*100)}%"></i></div>`:''}${actions||spec.details?`<div class="co-actions">${actions}${spec.details?`<button class="co-link" data-action="details" data-id="${escape(appId)}">详情</button>`:''}</div>`:''}`;
  }
  function show(appId,spec,{announce:say=true}={}){
    let item=items.get(appId);
    if(!item){
      const el=document.createElement('div');el.className='callout';el.id=idFor(appId);el.setAttribute('role','group');
      layer.append(el);item={el};items.set(appId,item);
    }
    clearTimeout(item.timer);item.spec=spec;paint(appId,item);
    item.el.classList.remove('in');void item.el.offsetWidth;if(say)item.el.classList.add('in');
    for(const main of tileMains(appId))main.setAttribute('aria-describedby',item.el.id);
    if(say)announce(`${getApp(appId)?.name||''}：${spec.title}。${spec.body||''}`);
    if(spec.timeout)item.timer=setTimeout(()=>dismiss(appId),spec.timeout);
    if(!frame)frame=requestAnimationFrame(place);
  }
  // Progress ticks repaint quietly; only arrivals and outcomes are announced.
  function update(appId,patch){const item=items.get(appId);if(!item)return;item.spec={...item.spec,...patch};paint(appId,item);}
  function dismiss(appId,{restoreFocus=false}={}){
    const item=items.get(appId);if(!item)return;
    const hadFocus=item.el.contains(document.activeElement);
    clearTimeout(item.timer);item.el.remove();items.delete(appId);
    for(const main of tileMains(appId))main.removeAttribute('aria-describedby');
    if(hadFocus||restoreFocus)homeTile(appId)?.focus();
  }
  // Tab from the tile lands on the main action, not on the close button.
  function focus(appId){const item=items.get(appId);if(!item||item.el.hidden)return false;(item.el.querySelector('.co-primary')||item.el.querySelector('.co-actions button')||item.el.querySelector('button'))?.focus();return true;}
  // Keyboard: Escape closes and returns to the tile; Tab past either end returns too.
  layer.addEventListener('keydown',e=>{
    const el=e.target.closest('.callout');if(!el)return;
    const appId=[...items].find(([,item])=>item.el===el)?.[0];
    const buttons=[...el.querySelectorAll('button')],index=buttons.indexOf(e.target);
    if(e.key==='Escape'){e.preventDefault();e.stopPropagation();dismiss(appId,{restoreFocus:true});}
    else if(e.key==='Tab'&&((e.shiftKey&&index===0)||(!e.shiftKey&&index===buttons.length-1))){e.preventDefault();homeTile(appId)?.focus();}
  });
  // Buttons act through the shared click handler. Acting retires the callout unless the
  // action replaced it with a newer one (a retry in progress, a copy confirmation).
  layer.addEventListener('click',e=>{
    const button=e.target.closest('button[data-action]');if(!button)return;
    const appId=button.dataset.id,before=items.get(appId)?.spec;
    setTimeout(()=>{if(before&&items.get(appId)?.spec===before)dismiss(appId);});
  });
  const has=appId=>items.has(appId),specOf=appId=>items.get(appId)?.spec;
  function clear(){for(const appId of [...items.keys()])dismiss(appId);}
  return {show,update,dismiss,focus,has,specOf,clear,
    // Re-link tiles after the board is rebuilt.
    relink(){for(const [appId,item] of items)for(const main of tileMains(appId))main.setAttribute('aria-describedby',item.el.id);}};
})();
// One polite live region carries callouts and activity results to assistive tech.
function announce(text){const el=document.querySelector('#announce');if(!el)return;el.textContent='';setTimeout(()=>{el.textContent=text;},60);}
