'use strict';
// A single floating menu for tiles, group headers and the canvas. Items are data:
// {label, icon, run, disabled, danger, hint, items}. An item with `items` opens a second
// panel in place (with a back row), which keeps keyboard handling linear and predictable.
const menu=(()=>{
  const el=document.createElement('div');
  el.className='ctx';el.setAttribute('role','menu');el.hidden=true;
  document.body.append(el);
  let stack=[],origin=null,anchor={x:0,y:0};
  const buttons=()=>[...el.querySelectorAll('[role=menuitem]:not([disabled])')];
  function paint(){
    const {title,items}=stack.at(-1);
    el.innerHTML=`${stack.length>1?`<button class="ctx-item ctx-back" role="menuitem" data-back>${icon('back')}<span>${escape(title)}</span></button><hr>`:''}${items.map((item,index)=>item==='-'?'<hr>':`<button class="ctx-item${item.danger?' danger':''}" role="menuitem" data-index="${index}" ${item.disabled?'disabled':''}>${icon(item.icon||'info')}<span>${escape(item.label)}</span>${item.items?`<i class="ctx-more">${icon('chevron')}</i>`:item.hint?`<kbd>${escape(item.hint)}</kbd>`:''}</button>`).join('')}`;
    place();(buttons()[stack.length>1?1:0]||buttons()[0])?.focus();
  }
  function place(){
    el.style.left='0px';el.style.top='0px';
    const {width,height}=el.getBoundingClientRect();
    el.style.left=`${Math.max(8,Math.min(anchor.x,innerWidth-width-8))}px`;
    el.style.top=`${anchor.y+height>innerHeight-8?Math.max(8,anchor.y-height):anchor.y}px`;
  }
  function open(items,point,from,title=''){
    hideTip();stack=[{title,items}];origin=from||document.activeElement;anchor=point;
    el.hidden=false;el.classList.remove('in');void el.offsetWidth;el.classList.add('in');paint();
  }
  function close(restore=true){
    if(el.hidden)return;el.hidden=true;stack=[];
    if(restore&&origin?.isConnected)origin.focus();
  }
  el.addEventListener('click',e=>{
    const button=e.target.closest('[role=menuitem]');if(!button)return;
    if(button.hasAttribute('data-back')){stack.pop();paint();return;}
    const item=stack.at(-1).items[Number(button.dataset.index)];
    if(item.items){stack.push({title:item.label,items:item.items});paint();return;}
    close(!item.keepFocus);item.run?.();
  });
  el.addEventListener('keydown',e=>{
    const list=buttons(),index=list.indexOf(document.activeElement);
    if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();list[(index+(e.key==='ArrowDown'?1:-1)+list.length)%list.length]?.focus();}
    else if(e.key==='Home'||e.key==='End'){e.preventDefault();list[e.key==='Home'?0:list.length-1]?.focus();}
    else if(e.key==='ArrowRight'&&document.activeElement.querySelector('.ctx-more')){e.preventDefault();document.activeElement.click();}
    else if(e.key==='ArrowLeft'&&stack.length>1){e.preventDefault();stack.pop();paint();}
    else if(e.key==='Escape'){e.preventDefault();e.stopPropagation();if(stack.length>1){stack.pop();paint();}else close();}
    else if(e.key==='Tab'){close(false);}
  });
  document.addEventListener('pointerdown',e=>{if(!el.hidden&&!el.contains(e.target))close(false);},true);
  addEventListener('resize',()=>close(false));
  document.addEventListener('scroll',e=>{if(!el.hidden&&!el.contains(e.target))close(false);},true);
  return {open,close,get isOpen(){return !el.hidden;}};
})();
