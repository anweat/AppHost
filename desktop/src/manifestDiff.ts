// Linear, bounded display diff: common prefix/suffix and one changed hunk. Full
// text stays available for review; avoid quadratic LCS on a 1 MiB edit buffer.
export function declarationDiff(before:string, after:string) {
  const a=before.split('\n'), b=after.split('\n'); let first=0, tail=0;
  while(first<a.length && first<b.length && a[first]===b[first]) first++;
  if(first===a.length && first===b.length) return '声明文本没有变化';
  while(tail<a.length-first && tail<b.length-first && a[a.length-tail-1]===b[b.length-tail-1]) tail++;
  const removed=a.slice(first,a.length-tail), added=b.slice(first,b.length-tail);
  const lines=[`@@ 原第 ${first+1} 行起 -${removed.length} / +${added.length} @@`,...removed.slice(0,200).map(l=>`− ${l}`),...added.slice(0,200).map(l=>`+ ${l}`)];
  if(removed.length>200 || added.length>200) lines.push('…差异预览已截断，请展开完整声明核对。');
  return lines.join('\n');
}
export interface EditFocus {service?:string|null;field:'port'|'command'}
export function declarationSelection(text:string, focus:EditFocus) {
  const lines=text.split('\n'); let start=0, end=lines.length;
  if(focus.service) {
    const name=lines.findIndex(l => /^\s*name\s*=/.test(l) && (l.includes(JSON.stringify(focus.service)) || l.includes(`'${focus.service}'`)));
    if(name>=0) {start=name; for(let i=name+1;i<lines.length;i++) if(/^\s*\[\[services\]\]/.test(lines[i]!)){end=i;break;}}
  }
  const relative=lines.slice(start,end).findIndex(l => new RegExp(`^\\s*${focus.field}\\s*=`).test(l));
  const row=relative<0?start:start+relative;
  const offset=lines.slice(0,row).reduce((n,l)=>n+l.length+1,0);
  return {start:offset,end:offset+(lines[row]?.length||0)};
}
