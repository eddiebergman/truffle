'use strict';
const $ = (s, root = document) => root.querySelector(s);
const labels = { 'todo': 'To do', 'in-progress': 'In progress', 'needs-review': 'Needs review', 'ready-to-merge': 'Ready to merge', 'draft': 'Draft', 'blocked': 'Blocked', 'done': 'Done' };
const order = ['blocked', 'ready-to-merge', 'needs-review', 'in-progress', 'draft', 'todo', 'done'];
const sourceNames = { 'chat-results':'Search results',agents:'Agents',skills:'Skills', newsletters:'Newsletters', artifacts:'Artifacts', logs:'Logs', move:'Your move', changes:'Changes', briefing:'Briefings', github: 'GitHub', linear: 'Linear', sentry: 'Sentry', notes: 'Projects', sessions: 'Agent sessions', archive: 'Archive', all: 'All Items', pinned: 'Pinned' };
const icons = { github: '⌘', linear: '◩', sentry: '◬', notes: '▤' };
let prefs = {};
try { prefs = JSON.parse(localStorage.getItem('truffle-view') || '{}'); } catch (_) { /* Browser storage is optional. */ }
if (!prefs || typeof prefs !== 'object') prefs = {};
let source = ['chat-results','agents','skills','newsletters','artifacts','logs','move','changes','briefing','all', 'pinned', 'linear', 'sentry', 'github', 'notes', 'sessions', 'archive'].includes(prefs.source) ? prefs.source : 'agents';
if(prefs.agentWorkspaceVersion!==1)source='agents';
let state = null, currentItem = null, toastTimer, refreshing = false;
const workPageSize = 100;
let workPageLimit = workPageSize, workPageKey = '';
const statusFilter = $('#status-filter');
for (const [value, text] of Object.entries(labels)) statusFilter.add(new Option(text, value));
statusFilter.value = ['all', 'active', ...order].includes(prefs.status) ? prefs.status : 'active';
const groupingPrefs = Object.fromEntries(['newsletters','artifacts','logs','move','changes','briefing','all','pinned','linear','sentry','github','notes','sessions','archive'].map(name=>{
  const allowed=['status','source','none',...(['github','linear','sentry'].includes(name)?['project']:[]),...(name==='github'?['stack']:[])];
  const saved=prefs.groups?.[name];
  return [name,allowed.includes(saved)?saved:['github','linear','sentry'].includes(name)?'project':['status','source','none'].includes(prefs.group)?prefs.group:'status'];
}));
const scopePrefs = Object.fromEntries(['github','linear','sentry'].map(name=>[name,typeof prefs.scopes?.[name]==='string'?prefs.scopes[name]:'']));
let githubView=['mine','review'].includes(prefs.githubView)?prefs.githubView:'all';
let scopeSource = null;
const folderPrefs = prefs.launchFolders&&typeof prefs.launchFolders==='object'?prefs.launchFolders:{};

for (const select of [$('#note-status'), $('#item-status')]) {
  for (const [value, text] of Object.entries(labels)) select.add(new Option(text, value));
}
function persist() {
  try { localStorage.setItem('truffle-view', JSON.stringify({ leftSidebarCollapsed:typeof leftSidebarCollapsed==='undefined'?false:leftSidebarCollapsed,agentContextCollapseVersion:1,agentSidebarCollapsed:typeof agentSidebarCollapsed==='undefined'?true:agentSidebarCollapsed,contextSectionsCollapsed:typeof contextSectionsCollapsed==='undefined'?{}:contextSectionsCollapsed,linkedWorkCollapsed:typeof linkedWorkCollapsed!=='undefined'&&linkedWorkCollapsed, snoozedAgentTabs:typeof snoozedAgentTabs==='undefined'?{}:snoozedAgentTabs,gitDiffSelections:typeof gitDiffSelections==='undefined'?{}:gitDiffSelections,agentWorkspaceVersion:1,closedAgentTabs:typeof closedAgentTabs==='undefined'?[]:closedAgentTabs,selectedAgentKey:typeof selectedAgentKey==='undefined'?'':selectedAgentKey, source, githubView, status: statusFilter.value, group: $('#group-by').value, agent: $('#quick-agent').value, groups: groupingPrefs, scopes: scopePrefs, launchFolders: folderPrefs, sentryFilters, sentryView })); } catch (_) { /* The server still stores all work. */ }
}
function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}
function toast(message,undo) {
  $('#toast').replaceChildren(document.createTextNode(message));
  if(undo){const button=el('button','toast-undo','Undo');button.onclick=async()=>{button.disabled=true;try{const result=await undo();if(result!==false)$('#toast').hidden=true;}catch(error){toast(error.message);}};$('#toast').append(button);}
  $('#toast').hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $('#toast').hidden = true; }, undo?12000:5000);
}
const serviceClient=createDashboardClient({native:window.__TAURI__?.core.invoke||null,onConnection:connected=>window.dispatchEvent(new CustomEvent('truffle-connection',{detail:connected}))});
async function api(path,payload){return serviceClient.request(path,payload);}

function sameSnapshot(previous,next) {
  if (!previous) return false;
  if (previous.revision !== undefined && next.revision !== undefined) return previous.revision === next.revision;
  // The original Python service has no revision. Keep it usable during migration.
  return JSON.stringify(previous) === JSON.stringify(next);
}

// Commit inside the guarded continuation: callers must not assign a returned
// snapshot after another await. Local immutable changes invalidate older reads.
async function loadStateSnapshot({attempts=1}={}) {
  for(let attempt=0;attempt<attempts;attempt++){
    const baseline=state,next=await api('/api/state');
    if(state!==baseline)continue;
    const changed=!sameSnapshot(state,next);state=next;
    return {accepted:true,changed};
  }
  return {accepted:false,changed:false};
}
async function refresh() {
  if (refreshing) return;
  refreshing = true;
  try {
    const result = await loadStateSnapshot();
    if(result.accepted){
      if (result.changed) render(); else renderSync();
      handleSessionHash();
    }
    if(typeof window!=='undefined'&&typeof window.refreshConfiguration==='function')void window.refreshConfiguration();
    if(typeof window!=='undefined'&&typeof window.refreshPresentations==='function')void window.refreshPresentations();
  }
  catch (_) { $('#sync-label').textContent = 'Server unavailable · showing last loaded view'; $('#sync-dot').classList.add('error-dot'); }
  finally { refreshing = false; }
}
async function presentLocalWrite(write) {
  try { await write(); }
  catch(error){const kind=[400,401,403,404,409].includes(error.status)?'rejected':'unknown';if(kind==='unknown')state={...state};return {kind,error:error.message||'The local write could not be confirmed.'};}
  // Invalidate reads that started before this accepted write. The POST body is
  // never a snapshot commit source; only the guarded state reader can commit it.
  state={...state};
  try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('The dashboard changed while refreshing.');render();return {kind:'accepted',presentation:'ready'};}
  catch(error){return {kind:'accepted',presentation:'pending',error:error.message||'The dashboard could not refresh.'};}
}
async function mutate(payload){return presentLocalWrite(()=>api('/api/edit',payload));}
async function markChangesRead(ids){return presentLocalWrite(()=>api('/api/commands/invoke',{name:'changes.mark-read',args:{ids}}));}
function localWriteMessage(result,acceptedText){
  if(result.kind==='accepted')return acceptedText+(result.presentation==='pending'?' The dashboard could not refresh. Refresh the view; do not repeat this action.':'');
  if(result.kind==='unknown')return result.error+' The save outcome is unknown. Inspect the current view before trying again.';
  return result.error;
}
async function mutateChatMetadata(name,args,{renderAfter=true}={}){
 try{const fields={'chats.rename':['title','updatedAt'],'chats.visibility':['hidden'],'chats.archive':['archivedAt']}[name];if(!fields)throw new Error('Unsupported chat metadata action.');const result=await api('/api/commands/invoke',{name,args}),patch=Object.fromEntries(fields.map(key=>[key,result.chat[key]]));state={...state,sessions:state.sessions.map(record=>record.id===result.chat.id?{...record,...patch}:record)};if(renderAfter)render();return true;}
 catch(error){toast(error.message);return false;}
}
function workStatusLabel(item){return item.sourceUnavailable?'Unavailable':item.archivedAt?'Archived':item.source==='sentry'&&!item.overridden?item.sourceStatus:labels[item.status]||'Unknown';}
let itemCacheState=null,itemCache=[];
let visibleWorkItems=[];
function allItems() {
  if(itemCacheState===state)return itemCache;
  itemCacheState=state;
  itemCache=[...state.items, ...state.notes].map(item => {
    const override = state.overrides[item.id] || {};
    return { ...item, kind: item.source === 'notes' ? 'Project' : item.kind, status: override.status || item.status, sourceStatusCode: item.status,
      pinned: !!override.pinned, hidden: !!override.hidden, overridden: !!override.status };
  });
  return itemCache;
}
function relative(timestamp) {
  if (!timestamp) return 'not synced yet';
  const seconds = Math.max(0, (Date.now() / 1000) - timestamp);
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
function sourceMark(item) { return el('span', `source-icon ${item.source}`, icons[item.source]); }
function workReference(item) {
  if (item.source !== 'github' || !item.reference) return item.reference || '';
  const number = item.reference.match(/#(\d+)$/)?.[1];
  const repository = item.url?.match(/^https:\/\/github\.com\/[^/]+\/([^/]+)\//)?.[1] || item.project?.split('/').pop();
  return number && repository ? `${repository}#${number}` : item.reference;
}
function githubRole(item) {
  if(item.source!=='github')return null;
  if(item.relations?.includes('authored'))return {label:item.kind==='Issue'?'My issue':'My PR',kind:'mine',detail:'You authored this work.'};
  if(item.relations?.includes('reviews'))return {label:'Review requested',kind:'review',detail:'Your review is requested on this pull request.'};
  if(item.relations?.some(x=>['assigned','issues'].includes(x)))return {label:'Assigned',kind:'assigned',detail:'This work is assigned to you.'};
  return {label:'Linked',kind:'linked',detail:'Linked work; your ownership or review role is not in the cache.'};
}
function styleWorkTitle(title,item) {
  if(item.source!=='github')return;
  title.classList.toggle('draft-title',item.kind==='Pull request'&&(item.isDraft===true||item.status==='draft'));
  const prefix=item.title.match(/^(feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert)(?:\([^)]*\))?!?:\s*/i);
  if(prefix)title.classList.add('commit-'+prefix[1].toLowerCase());
  if((prefix&&['chore','build','ci','style'].includes(prefix[1].toLowerCase()))||/^(?:update|bump)\s+\S/i.test(item.title))title.classList.add('maintenance-title');
  title.replaceChildren();let cursor=0, primary=false;
  function appendPR(text,offset){
    if(!text)return;
    const link=el(item.url?'a':'span','pr-title-part '+(/^[\s(),.;]*$/.test(text)?'title-punctuation':'title-prose'));
    if(item.url){link.href=item.url;link.target='_blank';link.rel='noopener noreferrer';}
    if(!primary){link.dataset.focusKey=item.id+':title';primary=true;}
    if(offset===0&&prefix)link.append(el('span','commit-prefix '+prefix[1].toLowerCase(),prefix[0]),document.createTextNode(text.slice(prefix[0].length)));
    else link.textContent=text;
    title.append(link);
  }
  for(const match of item.title.matchAll(/\b[A-Z][A-Z0-9]*-[0-9]+\b/g)){
    appendPR(item.title.slice(cursor,match.index),cursor);
    const known=state.items.find(x=>x.source==='linear'&&x.reference===match[0]);
    const link=el('a','linear-ticket-link',match[0]);link.href=known?.url||state.linearLinks?.[match[0]]||'/linear/'+match[0];
    link.target='_blank';link.rel='noopener noreferrer';link.title='Open '+match[0]+' in Linear';link.dataset.focusKey=item.id+':linear:'+match.index;
    title.append(link);cursor=match.index+match[0].length;
  }
  appendPR(item.title.slice(cursor),cursor);
}
const collapsedIssueTrees=new Set();
function appendLinearTree(container,items){
  const nodes=new Map(), order=[];
  function ensure(ref){if(!nodes.has(ref.id)){nodes.set(ref.id,{ref,children:[]});order.push(ref.id);}return nodes.get(ref.id);}
  for(const item of items){
    const node=ensure(item);node.item=item;node.ref=item;
    let child=node,parent=item.parent,seen=new Set([item.id]);
    while(parent&&!seen.has(parent.id)&&seen.size<9){seen.add(parent.id);const ancestor=ensure(parent);child.parent=parent.id;if(!ancestor.children.includes(child.ref.id))ancestor.children.push(child.ref.id);child=ancestor;parent=parent.parent;}
  }
  function renderNode(node,seen){
    if(seen.has(node.ref.id))return el('div');const next=new Set(seen).add(node.ref.id);
    const wrap=el('div','issue-tree-node'),header=el('div','issue-tree-header');
    if(node.children.length){const toggle=el('button','tree-toggle',collapsedIssueTrees.has(node.ref.id)?'▸':'▾');toggle.setAttribute('aria-label','Toggle sub-issues of '+(node.ref.reference||node.ref.title));toggle.setAttribute('aria-expanded',String(!collapsedIssueTrees.has(node.ref.id)));toggle.onclick=()=>{if(collapsedIssueTrees.has(node.ref.id))collapsedIssueTrees.delete(node.ref.id);else collapsedIssueTrees.add(node.ref.id);render();};header.append(toggle);}
    else header.append(el('span','tree-spacer'));
    if(node.item)header.append(renderRow(node.item));
    else{const link=el('a','parent-issue-context',(node.ref.reference?node.ref.reference+' · ':'')+node.ref.title);link.href=node.ref.url;link.target='_blank';link.rel='noopener noreferrer';link.title='Parent issue';header.append(link);}
    wrap.append(header);
    if(!collapsedIssueTrees.has(node.ref.id)){const children=el('div','issue-tree-children');for(const id of node.children)children.append(renderNode(nodes.get(id),next));if(node.children.length)wrap.append(children);}
    return wrap;
  }
  for(const id of order){const node=nodes.get(id);if(!node.parent)container.append(renderNode(node,new Set()));}
}
const hiddenSectionsOpen = new Set();
function scopeKey(item) {
  return item.source==='github' ? item.url?.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\//)?.[1] || item.project || 'Unknown repository' : item.project || 'No project';
}
function scopeLabel(key) {
  if(source!=='github')return key;
  const short=key.split('/').pop();
  const owners=new Set(allItems().filter(x=>x.source==='github'&&scopeKey(x).split('/').pop()===short).map(scopeKey));
  return owners.size>1?key:short;
}
function isSource(item,name){return item.source===name;}
function matchesSource(item){return source==='all'||(source==='pinned'?item.pinned:isSource(item,source));}
function matchesScope(item) { return !['github','linear','sentry'].includes(source)||!scopePrefs[source]||scopeKey(item)===scopePrefs[source]; }
function updateGroupingControl() {
  const select=$('#group-by');let option=select.querySelector('[value="project"]');
  if(['all','github','linear','sentry','notes'].includes(source)){
    if(!option){option=new Option('', 'project');select.add(option);}
    option.textContent=source==='github'?'Group: Repo':'Group: Project';
  } else option?.remove();
  let stack=select.querySelector('[value="stack"]');
  if(source==='github'){if(!stack)select.add(new Option('Group: Stack','stack'));}else stack?.remove();
  select.value=groupingPrefs[source];
}
function renderScopeFilters(items) {
  const strip=$('#scope-filters');
  strip.hidden=!['github','sentry'].includes(source);
  if(strip.hidden){scopeSource=null;return;}
  const scroll=scopeSource===source?strip.scrollLeft:0, focused=document.activeElement?.dataset.scopeKey;
  scopeSource=source;
  const keys=[...new Set(items.filter(x=>isSource(x,source)&&!x.archivedAt&&(typeof pageSettingsAllows!=='function'||pageSettingsAllows(x))).map(scopeKey))].sort((a,b)=>a.localeCompare(b));
  if(scopePrefs[source]&&!keys.includes(scopePrefs[source]))keys.push(scopePrefs[source]);
  const name=source==='github'?'repository':'project';
  strip.setAttribute('aria-label',`Filter ${name==='repository'?'repositories':'projects'}`);
  strip.replaceChildren(...['',...keys].map(key=>{
    const button=el('button','scope-chip',key?scopeLabel(key):source==='github'?'All repos':'All projects');
    button.type='button';button.dataset.scopeKey=key;button.title=key||button.textContent;
    button.setAttribute('aria-pressed',String(scopePrefs[source]===key));
    button.setAttribute('aria-label',key?`Filter ${name}: ${key}`:button.textContent);
    button.onclick=()=>{scopePrefs[source]=scopePrefs[source]===key?'':key;if(typeof setSavedViewScope==='function')setSavedViewScope(scopePrefs[source]);persist();render();};
    return button;
  }));
  strip.scrollLeft=scroll;
  if(focused!==undefined)for(const button of strip.children)if(button.dataset.scopeKey===focused)button.focus({preventScroll:true});
}
function visibilityButton(record, session=false) {
  const button = el('button','icon-button visibility-button');
  const label = `${record.hidden ? 'Show' : 'Hide'} ${session ? 'session' : 'item'}: ${record.title}`;
  button.setAttribute('aria-label',label); button.title=record.hidden?'Show again':'Hide from dashboard';
  button.dataset.focusKey=record.id+':visibility';
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
  for (const [key,value] of Object.entries({viewBox:'0 0 24 24',width:'16',height:'16',fill:'none',stroke:'currentColor','stroke-width':'1.6','aria-hidden':'true'})) svg.setAttribute(key,value);
  for (const shape of [['path',{d:'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z'}],['circle',{cx:'12',cy:'12',r:'3'}],...(!record.hidden ? [['path',{d:'M3 3l18 18'}]] : [])]) {
    const node=document.createElementNS(svg.namespaceURI,shape[0]);for(const [key,value] of Object.entries(shape[1]))node.setAttribute(key,value);svg.append(node);
  }
  button.append(svg);
  button.onclick=async()=>{
    if(button.disabled)return;const focused=document.activeElement===button;button.disabled=true;
    const page=source,dialogItem=currentItem,hidden=!!record.hidden,text=hidden?'Shown again.':'Hidden.';
    let result;try{
    result=session?(await mutateChatMetadata('chats.visibility',{id:record.id,hidden:!hidden})?{kind:'accepted',presentation:'ready'}:null):await mutate({action:'override',id:record.id,value:{hidden:!hidden}});
    if(result?.kind==='accepted'){
      if(result.presentation==='ready'&&source===page&&$('#item-dialog').open&&currentItem===dialogItem&&dialogItem){
        try{const item=allItems().find(x=>x.id===dialogItem.id);if(item)renderItemSessions(item);}
        catch(error){result={kind:'accepted',presentation:'pending',error:error.message||'The item view could not refresh.'};}
      }
      const undo=result.presentation==='ready'?async()=>{
        if(session)return mutateChatMetadata('chats.visibility',{id:record.id,hidden});
        const restored=await mutate({action:'override',id:record.id,value:{hidden}});toast(localWriteMessage(restored,hidden?'Hidden again.':'Shown again.'));return restored.kind==='accepted'&&restored.presentation==='ready';
      }:undefined;
      toast(localWriteMessage(result,text),undo);
      if(result.presentation==='ready'&&source===page&&focused&&(document.activeElement===button||document.activeElement===document.body||!document.activeElement?.isConnected)){
        const target=hidden?[...document.querySelectorAll('[data-focus-key]')].find(x=>x.dataset.focusKey===button.dataset.focusKey):$('#hidden-summary');target?.focus({preventScroll:true});
      }
    }else if(result)toast(localWriteMessage(result,text));
    }finally{button.disabled=false;}
    if(result?.presentation!=='ready'&&source===page&&focused&&button.isConnected&&(document.activeElement===document.body||!document.activeElement?.isConnected))button.focus({preventScroll:true});
  };
  return button;
}
function renderHiddenItems(items) {
  const section=$('#hidden-items'),list=$('#hidden-list'),query=$('#search').value.trim().toLowerCase();
  const work=items.filter(x=>x.hidden&&!x.archivedAt&&matchesSource(x)&&matchesScope(x)&&[x.title,workReference(x),x.project,x.detail].join(' ').toLowerCase().includes(query));
  const sessions=['all','sessions'].includes(source)?(state.sessions||[]).filter(x=>x.hidden&&!x.archivedAt&&[x.title,x.cwd,...x.items.map(i=>i.title+' '+workReference(i))].join(' ').toLowerCase().includes(query)):[];
  section.hidden=source==='archive'||!work.length&&!sessions.length;
  section.dataset.source=source;
  section.open=hiddenSectionsOpen.has(source);
  $('#hidden-summary').textContent=`Hidden · ${work.length+sessions.length}`;
  list.replaceChildren();
  if(section.hidden||!section.open)return;
  if(sessions.length){list.append(el('h3','group-header','Agent sessions'));sessions.forEach(x=>list.append(sessionCard(x)));}
  if(work.length){list.append(el('h3','group-header','Work items'));work.forEach(x=>list.append(renderRow(x)));}
}
$('#hidden-items').ontoggle=()=>{
  const section=$('#hidden-items');
  if(section.open){hiddenSectionsOpen.add(section.dataset.source);if(state)renderHiddenItems(allItems());}
  else{hiddenSectionsOpen.delete(section.dataset.source);$('#hidden-list').replaceChildren();}
};
// Ordered subsequence search, with a preference for contiguous words and boundaries.
function fuzzyScore(query, text) {
  const words=String(query).toLowerCase().trim().split(/\s+/).filter(Boolean);
  const value=String(text||'').toLowerCase();
  let score=0;
  for(const word of words){
    const exact=value.indexOf(word);
    if(exact>=0){score+=100+word.length*8-(exact===0||/[^a-z0-9]/.test(value[exact-1])?0:25)-Math.min(exact,100)*.1;continue;}
    const compactWord=word.replace(/[^\p{L}\p{N}]/gu,''),compactValue=value.replace(/[^\p{L}\p{N}]/gu,'');
    if(compactWord&&compactValue.includes(compactWord)){score+=90+compactWord.length*8;continue;}
    let cursor=0,last=-1,part=0;
    for(const char of word){
      const at=value.indexOf(char,cursor);if(at<0)return -Infinity;
      part+=5+(at===last+1?8:0)+(at===0||/[^a-z0-9]/.test(value[at-1])?8:0)-Math.min(at-cursor,30);
      last=at;cursor=at+1;
    }
    score+=part;
  }
  return score;
}
function workSearchScore(query,item){
  const normalize=value=>String(value??'').replace(/[^\p{L}\p{N}]/gu,'').toLowerCase();
  const words=query.trim().split(/\s+/).map(normalize).filter(Boolean);
  if(!words.length)return 0;
  const fields=[['title',40],['reference',35],['project',20],['sourceStatus',15],['detail',5]].map(([name,weight])=>({value:normalize(item[name]),weight,fuzzy:name!=='detail'}));
  fields.push({value:normalize((item.relations||[]).filter(value=>typeof value==='string').join(' ')),weight:15,fuzzy:true});
  let score=0;
  for(const word of words){
    let best=-Infinity;
    for(const field of fields){
      const at=field.value.indexOf(word);
      if(at>=0)best=Math.max(best,100+field.weight+(at===0?10:0));
      else if(field.fuzzy&&Number.isFinite(fuzzyScore(word,field.value)))best=Math.max(best,field.weight);
    }
    if(!Number.isFinite(best))return -Infinity;
    score+=best;
  }
  return score;
}
function workViewAllows(item,page=source){
  if(item.archivedAt||item.hidden||item.snoozed)return false;
  if(page!=='all'&&(page==='pinned'?!item.pinned:item.source!==page))return false;
  if(typeof pageSettingsAllows==='function'&&!pageSettingsAllows(item,page))return false;
  if(scopePrefs[page]&&scopeKey(item)!==scopePrefs[page])return false;
  if(!matchesSentry(item,page))return false;
  return typeof savedViewAllows==='function'?savedViewAllows(item,page):item.status!=='done';
}
function workViewCount(items,page){
  const draft=typeof currentSavedViewState==='function'?currentSavedViewState(page)?.draft:null;
  return items.filter(item=>workViewAllows(item,page)&&(!draft||typeof savedViewSearchAllows!=='function'||savedViewSearchAllows(draft.query,item))).length;
}
function workGroupingKey(item,groupBy){return groupBy==='project'?scopeKey(item):groupBy==='status'&&!Object.hasOwn(labels,item.status)?'unknown':item[groupBy];}
function workGroupingKeys(items,groupBy){
  if(groupBy==='status')return [...order,...(items.some(item=>!Object.hasOwn(labels,item.status))?['unknown']:[])];
  if(groupBy==='source')return [...new Set(['linear','sentry','github','notes',...items.map(item=>item.source)])];
  return groupBy==='project'?[...new Set(items.map(scopeKey))].sort((a,b)=>a.localeCompare(b)):['all'];
}
const renderHooks={before:new Map(),after:new Map(),finally:new Map()};
function registerRenderHook(name,run,{phase='after',order=0}={}){
  const hooks=Object.hasOwn(renderHooks,phase)?renderHooks[phase]:null;
  if(!hooks||typeof name!=='string'||!name||typeof run!=='function'||!Number.isFinite(order))throw new Error('Invalid render hook.');
  if(hooks.has(name))throw new Error('Render hook already registered: '+name);
  hooks.set(name,{name,run,order});
}
function runRenderHooks(phase,context){
  const hooks=[...renderHooks[phase].values()].sort((a,b)=>a.order-b.order||a.name.localeCompare(b.name));
  for(const hook of hooks){try{hook.run(context);}catch(error){if(context.error===null)context.error=error;}}
}
function render() {
  if(!state)return;
  const context={source,error:null};let result;
  runRenderHooks('before',context);
  try{if(context.error===null)result=renderWorkspacePage();}catch(error){context.error=error;}
  finally{runRenderHooks('after',context);runRenderHooks('finally',context);}
  if(context.error!==null)throw context.error;
  return result;
}
function renderWorkPage() {
  if (!state) return;
  visibleWorkItems=[];
  const focusKey = document.activeElement?.dataset.focusKey;
  const items = allItems();
  buildBundleIndex(items);
  $('.launcher').hidden=['logs','newsletters','artifacts'].includes(source);
  $('#search').placeholder=source==='logs'?'Search sync logs…':'Fuzzy find work…';
  $('#project-only').hidden = source !== 'notes';
  $('#sentry-note').hidden=source!=='sentry';
  $('#sentry-note').textContent=state.sentryScope?`Unresolved errors · ${state.sentryScope.org} · seen in the last ${state.sentryScope.period.replace('d',' days')} · all accessible projects · most events first`:'Configure Sentry in Connections & settings to load the direct issue feed.';
  updateGroupingControl();
  renderScopeFilters(items);
  renderFolderSelector();
  renderBoardSessions();
  renderHiddenItems(items);
  $('#demo-banner').hidden = !state.demo;
  const savedView=typeof currentSavedViewState==='function'?currentSavedViewState():null;
  const savedName=typeof savedViewsDocument!=='undefined'?savedViewsDocument?.views.find(view=>view.id===savedView?.id)?.name:'';
  const viewLabel=savedName?sourceNames[source]+' · '+savedName:source==='linear'&&scopePrefs.linear?'Linear · '+scopePrefs.linear:sourceNames[source];
  $('#page-title').replaceChildren(document.createTextNode(viewLabel), el('span', 'title-dot', '.'));
  for (const button of document.querySelectorAll('[data-source]')) button.classList.toggle('active', button.dataset.source === source);
  for (const name of ['all', 'pinned', 'linear', 'sentry', 'github', 'notes']) {
    $('#count-' + name).textContent = workViewCount(items,name);
  }
  $('#count-sessions').textContent = (state.sessions || []).filter(x => !x.archivedAt && !x.hidden && sessionWorkState(x) !== 'done').length;
  $('#count-archive').textContent = items.filter(x => x.archivedAt).length + (state.sessions || []).filter(x => x.archivedAt).length;
  renderSnoozed(items);
  renderSentryControls();
  const special = ['sessions', 'archive','move','changes','briefing','logs','newsletters','artifacts'].includes(source);
  $('.metrics').hidden = special;
  $('#status-filter').hidden = special||!!savedView;
  $('#group-by').hidden = special;
  $('#session-filter-bar').hidden = source !== 'sessions';
  if (renderFeatureView(items)) { renderSync(); return; }
  if (source === 'sessions') { renderSessionView(focusKey); renderSync(); return; }
  if (source === 'archive') { renderArchiveView(items,focusKey); renderSync(); return; }
  const scope = items.filter(item=>workViewAllows(item));
  for (const [id, status] of [['active', null], ['review', 'needs-review'], ['ready', 'ready-to-merge'], ['progress', 'in-progress'], ['blocked', 'blocked']]) {
    $('#metric-' + id).textContent = scope.filter(x => status ? x.status === status : x.status !== 'done').length;
  }
  for (const metric of document.querySelectorAll('[data-metric]')) metric.classList.toggle('selected',savedView?metric.dataset.metric==='active'&&savedView.draft.lifecycle==='active':metric.dataset.metric===statusFilter.value);
  const query = $('#search').value.trim().toLowerCase();
  const scores=new Map();
  const visible = scope.filter(x => {
    if(!query)return true;
    const score=workSearchScore(query,x);
    scores.set(x.id,score);return Number.isFinite(score);
  });
  visible.sort((a,b)=>(query?scores.get(b.id)-scores.get(a.id):0)||Number(b.pinned)-Number(a.pinned)||(source==='sentry'?Number(b.eventCount||0)-Number(a.eventCount||0):0)||a.title.localeCompare(b.title));
  visibleWorkItems=visible.slice();
  const list = $('#work-list');
  list.replaceChildren();
  if (!visible.length) {
    const empty = el('div', 'empty');
    const filtered=!!query||!!scopePrefs[source]||!!savedView&&(!!savedView.id||savedView.draft.lifecycle!=='active'||savedView.draft.filter.op!=='all'||savedView.draft.filter.conditions.length>0);
    const title=filtered?'No items match these filters.':'No items in this view.';
    empty.append(el('h2','',title));
    if(!items.length)empty.append(el('p','','Connect GitHub or Linear to load your work.'));
    let add;
    if(filtered){
      add=el('button','button','Clear filters');
      add.onclick=()=>{$('#search').value='';scopePrefs[source]='';if(typeof resetSavedViewState==='function')resetSavedViewState();persist();render();};
    }else if(source==='notes'){
      add=el('button','button','＋ Create a project');add.onclick=()=>openNote();
    }else if(!items.length){
      add=el('button','button','Connect your accounts');add.onclick=openSettings;
    }
    if(add)empty.append(add);list.append(empty);
  }
  const groupBy = $('#group-by').value;
  const pageKey = JSON.stringify([source,query,savedView?.draft,groupBy,scopePrefs[source],sentryFilters,sentryView]);
  if (pageKey !== workPageKey) { workPageKey = pageKey; workPageLimit = workPageSize; }
  let remaining = workPageLimit;
  if(groupBy==='stack'&&source==='github'){appendGithubStacks(list,visible,items,workPageLimit,false);remaining=0;}
  const keys=workGroupingKeys(visible,groupBy);
  if(query&&groupBy!=='none'){
    const best=new Map();for(const item of visible){const key=workGroupingKey(item,groupBy);best.set(key,Math.max(best.get(key)??-Infinity,scores.get(item.id)));}
    keys.sort((a,b)=>(best.get(b)??-Infinity)-(best.get(a)??-Infinity));
  }
  for (const key of keys) {
    let groupItems = visible.filter(x => groupBy === 'none' || workGroupingKey(x,groupBy)===key);
    if (!groupItems.length || !remaining) continue;
    if(source==='github')groupItems=orderedGithubItems(groupItems,items);
    const pageItems = groupItems.slice(0,remaining);
    remaining -= pageItems.length;
    const group = el('div', 'group');
    const heading = el('h2', 'group-header');
    const dotClass = { 'needs-review': 'review', 'ready-to-merge': 'ready', 'in-progress': 'progress', blocked: 'blocked' }[key] || '';
    heading.append(el('i', 'dot ' + dotClass), document.createTextNode(groupBy === 'status' ? labels[key]||'Unknown' : groupBy === 'source' ? sourceNames[key]||key : groupBy === 'project' ? scopeLabel(key) : 'Everything'), el('span', 'group-count', groupItems.length), el('span', 'group-line'));
    group.append(heading);
    if(source==='github')appendGithubStacks(group,pageItems,items,pageItems.length,true);
    else if(source==='linear')appendLinearTree(group,pageItems);
    else for (const item of pageItems) group.append(renderRow(item));
    list.append(group);
  }
  if (visible.length > workPageLimit) {
    const footer = el('div','group');
    footer.append(el('p','form-hint',`Showing ${workPageLimit} of ${visible.length} matching items. Search and filters include all items.`));
    const more = el('button','button',`Load ${Math.min(workPageSize,visible.length-workPageLimit)} more`);
    more.dataset.focusKey='work:load-more';
    more.onclick=()=>{workPageLimit+=workPageSize;render();};
    footer.append(more);list.append(footer);
  }
  if (focusKey) {
    // Preserve keyboard focus during the small background state refresh.
    for (const node of document.querySelectorAll('#work-list [data-focus-key], #active-sessions [data-focus-key], #hidden-list [data-focus-key]')) if (node.dataset.focusKey === focusKey) node.focus({preventScroll:true});
  }
  renderSync();
}
function renderRow(item) {
  const mixed=['all','pinned','archive','move','chat-results'].includes(source);
  const row = el('article', 'row'+(mixed?' mixed-source':['linear','sentry','notes'].includes(source)?' no-role-column':''));
  row.dataset.workId=item.id;row.dataset.focusKey=item.id+':row';row.tabIndex=-1;
  const codes=el('div','row-codes');
  const role=githubRole(item);
  const status=el('button','status-pill '+item.status,workStatusLabel(item));
  status.title=`Status: ${workStatusLabel(item)} · Click to change`;
  status.setAttribute('aria-label',`${workStatusLabel(item)}: edit status for ${item.title}`);
  status.dataset.focusKey=item.id+':status';
  status.onclick=()=>item.source==='notes'?openNote(item):openItem(item);
  if(mixed)codes.append(sourceMark(item));

  const bar=el('div','row-bar');
  const main = el('div', 'row-main');
  const title = el(item.source==='github'?'span':item.url?'a':'button', 'row-title', item.title);
  if(item.source!=='github')title.dataset.focusKey = item.id + ':title';
  if (item.url&&item.source!=='github') { title.href = item.url; title.target = '_blank'; title.rel = 'noopener noreferrer'; }
  else if(item.source!=='github') title.onclick = () => openItem(item);
  const meta = el('div', 'row-meta');
  if(role){const badge=el('span','item-role '+role.kind,role.kind==='mine'?'Yours':role.label);badge.title=role.detail;meta.append(badge);}
  if (item.reference) meta.append(el('span', 'ref', workReference(item)), el('span', 'meta-sep', '·'));
  meta.append(el('span', '', item.project || sourceNames[item.source]));
  if (item.relations?.includes('reviews')) meta.append(el('span', 'meta-sep', '·'), el('span', '', 'Your review requested'));
  else if (item.sourceStatus && item.sourceStatus !== labels[item.status]) meta.append(el('span', 'meta-sep', '·'), el('span', '', item.sourceStatus));
  if (item.overridden) meta.append(el('span', 'meta-sep', '·'), el('span', '', 'Local status'));
  title.title = item.title;
  styleWorkTitle(title,item);
  if(mixed)main.append(codes);main.append(title);bar.append(main,meta);
  const actions = el('div', 'row-actions'+(item.source==='github'&&item.kind==='Pull request'?' has-pr-review':''));
  const pin = el('button', 'pin-button' + (item.pinned ? ' pinned' : ''), item.pinned ? '★' : '☆');
  pin.setAttribute('aria-label', `${item.pinned ? 'Unpin' : 'Pin'} ${item.title}`);
  pin.setAttribute('aria-pressed', String(item.pinned));
  pin.dataset.focusKey = item.id + ':pin';
  pin.onclick = async () => {
    if(pin.disabled)return;const focused=document.activeElement===pin,page=source;pin.disabled=true;
    const result=await mutate({action:'override',id:item.id,value:{pinned:!item.pinned}});toast(localWriteMessage(result,item.pinned?'Unpinned.':'Pinned.'));pin.disabled=false;
    if(focused&&source===page&&(document.activeElement===document.body||!document.activeElement?.isConnected)){
      const target=[...document.querySelectorAll('[data-focus-key]')].find(node=>node.dataset.focusKey===pin.dataset.focusKey);target?.focus({preventScroll:true});
    }
  };
  const details = el('button', 'row-open icon-button', '⋯');
  details.setAttribute('aria-label', `Details for ${item.title}`);
  details.dataset.focusKey = item.id + ':details';
  details.onclick = () => openItem(item);
  const ciSlot=el('span','ci-slot');
  if (item.kind === 'Pull request') {
    const info = ciInfo(item.ci);
    const ciButton = el('button', 'ci-pill ' + (item.ci?.state || 'unknown'), info.short);
    ciButton.title = info.label + (item.ci?.total ? ` · ${item.ci.total} checks` : '');
    ciButton.setAttribute('aria-label', `CI checks for ${item.title}: ${info.label}`);
    ciButton.dataset.focusKey = item.id + ':ci';
    ciButton.onclick = () => openItem(item);
    ciSlot.append(ciButton);
  }
  if(source==='linear'&&item.sentryLinks?.length){const errorLink=el('a','sentry-link','Error ↗');errorLink.href=item.sentryLinks[0].url;errorLink.target='_blank';errorLink.rel='noopener noreferrer';errorLink.title=item.sentryLinks.length>1?'First Sentry error. Open details for all errors.':'Open Sentry error';errorLink.setAttribute('aria-label','Open Sentry error for '+item.title);ciSlot.append(errorLink);actions.classList.add('has-impact');}
  if(item.source==='sentry'){const impact=el('span','sentry-impact',`${Number(item.eventCount||0).toLocaleString()} ${Number(item.eventCount)===1?'event':'events'}`);impact.title=`${Number(item.userCount||0).toLocaleString()} users · Last seen: ${item.lastSeen||'unknown'}`;ciSlot.append(impact);actions.classList.add('has-impact');}
  if(item.source==='github'&&item.kind==='Pull request'&&typeof githubReviewerChip==='function'){actions.append(githubReviewerChip(item));actions.classList.add('has-github-actions');}
  actions.append(itemSessionChip(item),ciSlot,status,pin);
  if (!item.archivedAt) actions.append(visibilityButton(item));
  else actions.append(el('span','visibility-slot'));
  actions.append(details);bar.append(actions);row.append(bar);
  return row;
}
function renderSync() {
  const entries = Object.entries(state.sources).filter(([,info])=>!info.disabled);
  const infos = entries.map(([,info])=>info);
  const errors = infos.filter(x => x.error);
  const syncing = state.syncing.length > 0;
  const last = infos.map(x => x.lastSuccess).filter(Boolean);
  $('#sync-label').textContent = state.demo ? 'Sample workspace · no external requests' : syncing ? `Syncing ${state.syncing.map(x => sourceNames[x]).join(' & ')}…` : errors.length ? 'Cached locally · connections need attention' : last.length ? `Synced ${relative(Math.min(...last))}` : 'Ready to connect';
  $('#sync-dot').classList.toggle('error-dot', errors.length > 0);
  const next = Math.min(...infos.map(x => x.nextAttempt));
  const cooling = Number.isFinite(next) && Date.now() / 1000 < next;
  $('#sync').disabled = !infos.length || syncing || cooling || state.demo;
  const forceCooling = Date.now()/1000 < (state.forceRefreshAfter||0);
  $('#force-refresh').disabled = !infos.length || syncing || forceCooling || state.demo;
  $('#force-refresh .refresh-label').textContent = syncing ? 'Refreshing…' : forceCooling ? 'Refresh cooling down…' : 'Force refresh';
  $('#force-refresh').setAttribute('aria-label', syncing ? 'Refreshing sources' : 'Force refresh');
  $('#sync span').textContent = state.demo ? 'Sample workspace' : !infos.length ? 'No connections enabled' : syncing ? 'Syncing…' : cooling ? `Next sync in ${Math.max(1, Math.ceil((next - Date.now() / 1000) / 60))}m` : 'Sync now';
  const notice = $('#connection-notice');
  const focused=document.activeElement,noticeFocused=!!focused&&notice.contains(focused),focusKey=noticeFocused?focused.dataset.noticeFocus:null;
  if(notice.parentElement!==$('#sidebar-notices'))$('#sidebar-notices').append(notice);
  if(!errors.length)localStorage.removeItem('dismissed-connection-warning');
  const warningKey=JSON.stringify(entries.filter(([,info])=>info.error).map(([name,info])=>[name,info.error]));
  notice.hidden = !errors.length || state.demo || localStorage.getItem('dismissed-connection-warning')===warningKey;
  if(notice.hidden&&noticeFocused)$('#activity-open').focus();
  if(!errors.length||state.demo){notice.replaceChildren();delete notice.dataset?.warningKey;return;}
  if(notice.dataset.warningKey===warningKey)return;
  notice.dataset.warningKey=warningKey;notice.classList.add('ui-notice');notice.dataset.variant='warning';
  const header=el('div','ui-notice-header'),title=el('strong','ui-notice-title','Connections need attention');
  const dismiss=el('button','ui-control ui-notice-dismiss connection-notice-dismiss','×');dismiss.type='button';dismiss.dataset.variant='secondary';dismiss.dataset.size='icon';dismiss.setAttribute('aria-label','Dismiss connection warning');dismiss.title='Dismiss until the warning changes';dismiss.onclick=()=>{localStorage.setItem('dismissed-connection-warning',warningKey);notice.hidden=true;if(notice.contains(document.activeElement))$('#activity-open').focus();};dismiss.dataset.noticeFocus='dismiss';header.append(title,dismiss);
  const body=el('div','ui-notice-body');body.dataset.noticeFocus='body';body.tabIndex=0;body.setAttribute('role','region');body.setAttribute('aria-label','Connection errors');
  for(const [name,info]of entries.filter(([,info])=>info.error))body.append(el('p','',`${sourceNames[name]}: ${info.error}`));
  const actions=el('div','ui-actiongroup ui-notice-actions');actions.setAttribute('role','group');actions.setAttribute('aria-label','Connection warning actions');
  const button=el('button','ui-control','View connections');button.type='button';button.dataset.variant='secondary';button.dataset.noticeFocus='connections';button.onclick=()=>openPageSettingsCategory('connections');
  const logs=el('button','ui-control','View logs');logs.type='button';logs.dataset.variant='secondary';logs.dataset.noticeFocus='logs';logs.onclick=()=>{source='logs';persist();render();};actions.append(button,logs);
  notice.replaceChildren(header,body,actions);
  if(noticeFocused&&!notice.hidden)([dismiss,body,button,logs].find(node=>node.dataset.noticeFocus===focusKey)||body).focus();
}
const projectEditableFields=['title','detail','url','status','cwd'];
let projectEditor=null,projectStatusAction=null;
function projectNotice(host,label='Project recovery actions'){
  host.querySelector('[data-project-notice]')?.remove();
  const region=el('div');region.dataset.projectNotice='true';const message=el('p','form-hint');message.setAttribute('role','status');message.tabIndex=-1;const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label',label);
  const button=label=>{const control=el('button','ui-control',label);control.type='button';control.dataset.variant='secondary';control.dataset.projectRecovery='true';control.hidden=true;actions.append(control);return control;};
  const reload=button('Discard draft and reload'),refresh=button('Refresh view'),inspect=button('Close and inspect Projects');region.append(message,actions);host.append(region);return {region,message,reload,refresh,inspect};
}
function holdFormFocus(host,current){
  const active=document.activeElement;if(active&&host.contains(active)&&!active.closest('#app-chat-panel,.xterm')&&(!active.classList.contains('close-dialog')||current.pending)){current.focusReturn=active;current.ui.message.focus({preventScroll:true});}
}
function restoreFormFocus(current,target){
  if(!current.isCurrent?.()||document.activeElement!==current.ui.message||!current.ui.message.isConnected||!target?.isConnected||target.disabled||target.hidden)return;
  const dialog=current.host.closest('dialog');if([...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog&&!open.contains(dialog)))return;
  target.focus({preventScroll:true});
}
function projectCorrectionTarget(current,form){return current.focusReturn?.matches('input,textarea,select')&&!current.focusReturn.disabled?current.focusReturn:form.elements.title;}
function projectControls(host,current,locked){
  current.host=host;current.controls??=new Map();for(const control of host.querySelectorAll('input,textarea,select,button')){
    if(control.dataset.projectRecovery||control.closest('#app-chat-panel,.xterm'))continue;if(!current.controls.has(control))current.controls.set(control,control.disabled);
    control.disabled=control.classList.contains('close-dialog')?!!current.pending:locked||current.controls.get(control);
  }
  host.setAttribute('aria-busy',String(!!(current.pending||current.loading||current.refreshing)));
}
function releaseProjectControls(current){for(const [control,disabled]of current?.controls||[])control.disabled=disabled;current?.host?.setAttribute('aria-busy','false');}
function disposeProjectStatusAction(){const current=projectStatusAction;projectStatusAction=null;releaseProjectControls(current);const dialog=$('#item-dialog');dialog.querySelector('[data-project-notice]')?.remove();dialog.setAttribute('aria-busy','false');}
function projectUnknown(error){return ![400,401,403,404,409].includes(error.status);}
function inspectProjects(dialog,isCurrent){if(!isCurrent())return;dialog.close();source='notes';persist();render();}
async function openNote(item=null){
  const dialog=$('#note-dialog');if(projectEditor?.pending&&dialog.open)return;if(!requestDialogNavigation())return false;releaseProjectControls(projectEditor);
  const form=$('#note-form');form.reset();
  const current=projectEditor={id:item?.id||'',page:source,baseline:null,pending:false,loading:!!item,accepted:null,unknown:false,conflicted:false};
  const isCurrent=()=>projectEditor===current&&dialog.open&&source===current.page;current.isCurrent=isCurrent;
  form.elements.id.value=current.id;for(const key of projectEditableFields)form.elements[key].value=key==='status'?'todo':'';
  $('#note-dialog-title').textContent=item?'Edit project':'New project';const remove=$('#delete-note');remove.hidden=!item;remove.textContent='Delete project';remove.dataset.confirm='';
  current.ui=projectNotice(form);current.ui.inspect.onclick=()=>inspectProjects(dialog,isCurrent);
  current.ui.refresh.onclick=()=>refreshProjectEditor(current);
  current.ui.reload.onclick=()=>{if(isCurrent()&&!current.pending)void openNote(current.id?{id:current.id}:null);};
  current.ui.message.textContent=item?'Loading current project…':'';projectControls(form,current,!!item);dialog.showModal();
  if(!item){form.elements.title.focus();return;}
  current.ui.message.focus({preventScroll:true});
  try{const baseline=await api('/api/commands/invoke',{name:'projects.read',args:{id:current.id}});if(!isCurrent())return;current.baseline=baseline;
    for(const key of projectEditableFields)form.elements[key].value=baseline.project[key]??'';
    current.ui.message.textContent='';
  }catch(error){if(isCurrent()){current.ui.message.textContent=error.message+' Reload the project before editing.';current.ui.reload.textContent='Reload project';current.ui.reload.hidden=false;}}
  finally{current.loading=false;if(isCurrent()){projectControls(form,current,!current.baseline);restoreFormFocus(current,current.baseline?form.elements.title:current.ui.reload);}}
}
async function refreshProjectEditor(current){
  if(!current.isCurrent()||current.pending||current.refreshing)return;holdFormFocus($('#note-form'),current);current.refreshing=true;projectControls($('#note-form'),current,true);current.ui.refresh.disabled=true;
  try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('The dashboard changed while refreshing.');if(!current.isCurrent())return;render();
    if(current.unknown){current.ui.message.textContent='The save outcome is unknown. The dashboard data was refreshed. Inspect it before creating or saving again; this draft has not been resubmitted.';return;}
    $('#note-dialog').close();toast(current.accepted?(current.acceptedText||'Project saved on this computer.'):'No project changes.');
  }catch(error){if(current.isCurrent()){current.ui.message.textContent=current.localWriteResult?localWriteMessage(current.localWriteResult,current.acceptedText):current.unknown?'The save outcome is unknown. Inspect Projects before creating or saving again.':'Project saved on this computer. The dashboard could not refresh. Use Refresh view; do not save again.';current.ui.refresh.hidden=false;current.ui.inspect.hidden=false;}}
  finally{current.refreshing=false;if(current.isCurrent()){projectControls($('#note-form'),current,true);current.ui.refresh.disabled=false;restoreFormFocus(current,current.unknown?current.ui.inspect:current.ui.refresh);}}
}
async function saveProjectEditor(event){
  event.preventDefault();const current=projectEditor;if(!current||!current.isCurrent()||current.pending||current.loading||current.accepted||current.unknown||current.conflicted||current.id&&!current.baseline)return;
  const form=$('#note-form'),values=Object.fromEntries(projectEditableFields.map(key=>[key,form.elements[key].value]));
  const patch=current.id?Object.fromEntries(Object.entries(values).filter(([key,value])=>value!==current.baseline.project[key])):values;
  if(current.id&&!Object.keys(patch).length){$('#note-dialog').close();toast('No project changes.');return;}
  current.pending=true;holdFormFocus(form,current);projectControls(form,current,true);current.ui.message.textContent='Saving project…';
  try{const args=current.id?{id:current.id,patch,expected:Object.fromEntries(Object.keys(patch).map(key=>[key,current.baseline.expected[key]]))}:values;
    current.accepted=await api('/api/commands/invoke',{name:current.id?'projects.update':'projects.create',args});
    if(!current.isCurrent()){toast('Project saved on this computer. Refresh Projects to see it; do not save again.');return;}
  }catch(error){if(!current.isCurrent()){toast(projectUnknown(error)?'The project save outcome is unknown. Inspect Projects before saving again.':error.message);return;}
    if(error.status===409){current.conflicted=true;current.ui.message.textContent='This project changed. Your draft is kept. Discard draft and reload before reapplying your changes.';current.ui.reload.hidden=false;}
    else if(projectUnknown(error)){current.unknown=true;current.ui.message.textContent='The project save outcome is unknown. Inspect Projects before creating or saving again. This draft will not be resubmitted.';current.ui.refresh.hidden=false;current.ui.inspect.hidden=false;}
    else current.ui.message.textContent=error.message;
  }finally{current.pending=false;if(projectEditor===current&&$('#note-dialog').open){projectControls(form,current,!!(current.accepted||current.unknown||!current.isCurrent()));if(current.conflicted){$('#note-form button[type=submit]').disabled=true;$('#delete-note').disabled=true;}if(!current.accepted)restoreFormFocus(current,current.conflicted?current.ui.reload:current.unknown?current.ui.inspect:projectCorrectionTarget(current,form));}}
  if(current.accepted&&current.isCurrent())await refreshProjectEditor(current);
}
async function changeProjectStatus(item,status,{close=false}={}){
  const dialog=$('#item-dialog');if(projectStatusAction)return;
  const current=projectStatusAction={item,status,page:source,pending:true,accepted:false,unknown:false,refreshing:false},isCurrent=()=>projectStatusAction===current&&dialog.open&&currentItem?.id===item.id&&source===current.page;
  current.isCurrent=isCurrent;if(!isCurrent()){projectStatusAction=null;return;}const ui=current.ui=projectNotice(dialog);holdFormFocus(dialog,current);projectControls(dialog,current,true);ui.message.textContent='Reading current project status…';
  ui.inspect.onclick=()=>inspectProjects(dialog,isCurrent);
  const present=async()=>{if(!isCurrent()||current.pending||current.refreshing)return;holdFormFocus(dialog,current);current.refreshing=true;projectControls(dialog,current,true);ui.refresh.disabled=true;try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('Dashboard changed.');if(!isCurrent())return;render();if(current.unknown){ui.message.textContent='The status save outcome is unknown. Projects were refreshed. Inspect them before changing status again.';return;}if(close)dialog.close();else{const latest=allItems().find(row=>row.id===item.id);if(latest){currentItem=latest;$('#item-title').textContent=latest.title;$('#item-detail').textContent=latest.detail||'';$('#item-meta').textContent=(latest.project||'Personal')+' · Source status: '+(latest.sourceStatus||labels[latest.sourceStatusCode||latest.status]);$('#item-link').hidden=!latest.url;$('#item-link').href=latest.url||'#';$('#item-status').value=latest.status;$('#reset-status').disabled=!latest.overridden;renderItemSessions(latest);}}releaseProjectControls(current);if(!close)restoreFormFocus(current,$('#item-status'));ui.region.remove();toast(status==='done'?'Project marked done. Agent chats stay open.':'Project status saved locally.');projectStatusAction=null;
    }catch(_){if(isCurrent()){ui.message.textContent=current.unknown?'The status save outcome is unknown. Inspect Projects before changing status again.':'Project status saved locally. The dashboard could not refresh. Use Refresh view; do not change status again.';ui.refresh.hidden=false;ui.inspect.hidden=false;}}
    finally{current.refreshing=false;ui.refresh.disabled=false;if(isCurrent()){projectControls(dialog,current,true);restoreFormFocus(current,current.unknown?ui.inspect:ui.refresh);}}};
  ui.refresh.onclick=present;
  try{const baseline=await api('/api/commands/invoke',{name:'projects.read',args:{id:item.id}});if(!isCurrent())return;ui.message.textContent='Saving project status…';
    if(baseline.project.status!==status){current.writeStarted=true;await api('/api/commands/invoke',{name:'projects.update',args:{id:item.id,patch:{status},expected:{status:baseline.expected.status}}});}current.accepted=true;
    if(!isCurrent())toast('Project status saved locally. Refresh Projects to see it; do not change status again.');
  }catch(error){if(!isCurrent()){if(current.writeStarted)toast(projectUnknown(error)?'The project status save outcome is unknown. Inspect Projects before changing status again.':error.message);return;}if(error.status===409&&current.writeStarted){ui.message.textContent='This project status changed. No status was saved. Reload the project before choosing again.';ui.reload.textContent='Reload project';ui.reload.hidden=false;ui.reload.onclick=async()=>{if(!isCurrent()||current.refreshing)return;holdFormFocus(dialog,current);current.refreshing=true;projectControls(dialog,current,true);ui.reload.disabled=true;try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('Dashboard changed while refreshing.');if(isCurrent()){const latest=allItems().find(row=>row.id===item.id);if(latest){releaseProjectControls(current);projectStatusAction=null;openItem(latest);}else ui.message.textContent='This project is unavailable in the refreshed dashboard.';}}catch(_){if(isCurrent())ui.message.textContent='The project could not reload. Your newer changes were kept. Try Reload project again.';}finally{current.refreshing=false;if(isCurrent()){ui.reload.disabled=false;restoreFormFocus(current,ui.reload);}}};}
    else if(current.writeStarted&&projectUnknown(error)){current.unknown=true;ui.message.textContent='The project status save outcome is unknown. Inspect Projects before changing status again.';ui.refresh.hidden=false;ui.inspect.hidden=false;}
    else{ui.message.textContent=error.message;releaseProjectControls(current);restoreFormFocus(current,$('#item-status'));projectStatusAction=null;}
  }finally{current.pending=false;if(isCurrent()){projectControls(dialog,current,!!(current.accepted||current.unknown||ui.reload.hidden===false));if(!current.accepted)restoreFormFocus(current,current.unknown?ui.inspect:ui.reload);}else if(projectStatusAction===current){releaseProjectControls(current);projectStatusAction=null;}}
  if(current.accepted&&isCurrent())await present();
}
function openItem(item) {
  if(!requestDialogNavigation())return false;
  disposeProjectStatusAction();
  currentItem = item;
  $('#item-reference').textContent = `${sourceNames[item.source]} · ${workReference(item)}`;
  $('#item-title').textContent = item.title;
  $('#item-meta').textContent = `${item.project}${githubRole(item)?' · '+githubRole(item).detail:''} · Source status: ${item.sourceStatus || labels[item.sourceStatusCode]}`;
  $('#item-detail').textContent = item.detail || '';
  renderItemSessions(item);
  renderRelatedWork(item);
  renderIssueFamily(item);
  const sentryList=$('#item-sentry-links');sentryList.replaceChildren();sentryList.hidden=!item.sentryLinks?.length;
  if(item.sentryLinks?.length){sentryList.append(el('h3','','Sentry errors'));for(const error of item.sentryLinks){const link=el('a','linear-ticket-link',error.title+' ↗');link.href=error.url;link.target='_blank';link.rel='noopener noreferrer';sentryList.append(link);}}
  renderCI(item);
  $('#item-status').value = item.status;
  $('#item-status').closest('label').hidden = !!item.archivedAt && item.source!=='notes';
  $('#reset-status').disabled = !item.overridden;
  $('#reset-status').hidden = item.source === 'notes' || !!item.archivedAt;
  $('#override-hint').textContent = item.archivedAt && item.source!=='notes' ? 'Archived in the source. Restore it there to return it to active work.' : item.source === 'notes' ? 'Project status is saved locally. Agent conversations stay open.' : 'A local override only. Source status stays unchanged.';
  $('#item-link').hidden = !item.url;
  $('#item-link').href = item.url || '#';
  $('#item-dialog').showModal();
}
function renderIssueFamily(item){
  const section=$('#issue-family');section.hidden=item.source!=='linear';section.replaceChildren();if(section.hidden)return;
  if(item.parent){const line=el('p','form-hint','Parent: '),link=el('a','linear-ticket-link',(item.parent.reference||'')+' · '+item.parent.title);link.href=item.parent.url;link.target='_blank';link.rel='noopener noreferrer';line.append(link);section.append(line);}
  if(item.children?.length){section.append(el('h3','',`Sub-issues (${item.children.length}${item.moreChildren?'+':''})`));for(const child of item.children){const line=el('div','family-child'),link=el('a','linear-ticket-link',child.reference+' · '+child.title);link.href=child.url;link.target='_blank';link.rel='noopener noreferrer';line.append(link,el('span','status-pill '+child.status,workStatusLabel(child)));section.append(line);}}
  if(!item.parent&&!item.children?.length)section.append(el('p','form-hint','No parent or sub-issues.'));
}
function ciInfo(ci) {
  switch (ci?.state) {
    case 'passed': return { short: '✓ CI', label: 'Checks passed' };
    case 'failed': return { short: '× CI', label: 'Checks failing' };
    case 'pending': return { short: '◷ CI', label: 'Checks running' };
    case 'none': return { short: '– CI', label: 'No checks reported' };
    default: return { short: '? CI', label: ci ? 'Check state unknown' : 'CI not synced' };
  }
}
function renderCI(item) {
  const section = $('#ci-section');
  section.hidden = item.kind !== 'Pull request';
  if (section.hidden) return;
  const ci = item.ci;
  $('#ci-title').textContent = ciInfo(ci).label;
  $('#ci-count').textContent = ci?.total ? `${ci.total} checks` : '';
  const mergeState = item.mergeState ? item.mergeState.replaceAll('_', ' ').toLowerCase() : 'not synced';
  $('#ci-summary').textContent = `Merge state: ${mergeState}.` + (ci && ci.total > ci.checks.length ? ` Showing ${ci.checks.length} of ${ci.total} checks; the indicator includes all checks.` : '');
  const list = $('#ci-checks'); list.replaceChildren();
  const marks = { passed: '✓', skipped: '–', failed: '×', pending: '◷', unknown: '?' };
  for (const check of [...(ci?.checks || [])].sort((a, b) => {
    const priority = { failed: 0, pending: 1, unknown: 2, passed: 3, skipped: 4 };
    return priority[a.state] - priority[b.state] || a.name.localeCompare(b.name);
  })) {
    const row = el('div', 'ci-check');
    row.append(el('span', 'ci-mark ' + check.state, marks[check.state]));
    const name = el(check.url ? 'a' : 'span', 'ci-check-name', check.name);
    if (check.url) { name.href = check.url; name.target = '_blank'; name.rel = 'noopener noreferrer'; }
    row.append(name, el('span', 'ci-result', check.result.replaceAll('_', ' ').toLowerCase()));
    if (check.url) row.append(el('span', 'ci-link-arrow', '↗'));
    list.append(row);
  }
}

function openSettings() {
  $('#connections').replaceChildren();
  if (state) for (const source of ['github', 'linear', 'sentry']) {
    const info = state.sources[source] || {error:'Not configured'};
    const row = el('div', 'connection-row', sourceNames[source]);
    row.append(el('span', 'live-dot' + (info.error || !info.lastSuccess ? ' warning' : '')));
    row.append(el('small', '', state.demo ? 'Sample data · no connection attempted' : info.disabled ? 'Not configured' : info.error ? `${info.error} Last successful sync: ${relative(info.lastSuccess)}.` : `Last successful sync: ${relative(info.lastSuccess)}`));
    $('#connections').append(row);
  }
  const sentry=state?.sources?.sentry;
  const configured=!!sentry&&!sentry.disabled;
  $('#sentry-credential-status').textContent=configured?`Credentials are already configured${state.sentryScope?.org?' for '+state.sentryScope.org:''}. The connection status above shows whether the latest sync succeeded.`:'Sentry is not configured on this server yet.';
  $('#sentry-setup-help').open=!configured;
  $('#settings-dialog').showModal();
}
$('#sources').onclick = event => {
  const button = event.target.closest('[data-source]');
  if (!button) return;
  source=button.dataset.source;if(typeof resetSavedViewState==='function'&&typeof currentSavedViewState==='function'&&currentSavedViewState()?.id)resetSavedViewState();if(source==='linear')scopePrefs.linear='';if(typeof savedViewUsePage==='function')savedViewUsePage(source);if(source==='agents'){showAgentWorkspace();return;}persist();render();
};
$('#search').oninput=()=>{if(typeof savedViewCapturePresentation==='function')savedViewCapturePresentation();render();};
statusFilter.onchange=()=>{persist();render();};
$('#group-by').onchange=()=>{groupingPrefs[source]=$('#group-by').value;if(typeof savedViewCapturePresentation==='function')savedViewCapturePresentation();persist();render();};
for (const metric of document.querySelectorAll('[data-metric]')) metric.onclick=()=>{if(typeof setSavedViewStatus==='function'&&currentSavedViewState())setSavedViewStatus(metric.dataset.metric);else statusFilter.value=metric.dataset.metric;persist();render();};
$('#new-note').onclick = () => quickStart();
$('#settings').onclick = openSettings;
const dialogReturnFocus=new WeakMap();
function captureDialogReturn(dialog,{trigger=document.activeElement,scope,isCurrent=()=>true,fallbackKey=null,preserve=false}={}){
  const previous=dialogReturnFocus.get(dialog);if(preserve&&previous?.scope===scope)return;
  const external=trigger&&!dialog.contains(trigger)&&trigger!==document.body?trigger:null;
  const parent=external?.closest('dialog');
  dialogReturnFocus.set(dialog,{trigger:external?.isConnected?external:null,key:external?.dataset.focusKey||fallbackKey,fallbackKey,scope,isCurrent,parent:parent?.open?parent:null});
}
function restoreDialogReturn(dialog){
  const token=dialogReturnFocus.get(dialog);if(!token||dialog.open)return;dialogReturnFocus.delete(dialog);
  const active=document.activeElement;if(active&&active!==document.body&&active.isConnected&&!dialog.contains(active))return;
  if(!token.isCurrent()||token.parent&&(!token.parent.isConnected||!token.parent.open))return;
  if([...document.querySelectorAll('dialog[open]')].some(open=>open!==token.parent&&!open.contains(token.parent)))return;
  const visible=node=>node?.isConnected&&!node.disabled&&!node.matches(':disabled')&&!node.closest('[hidden],dialog:not([open])')&&node.getClientRects().length>0&&getComputedStyle(node).visibility==='visible';
  const root=token.parent||document,controls=[...root.querySelectorAll('[data-focus-key]')];
  const target=visible(token.trigger)?token.trigger:controls.find(node=>node.dataset.focusKey===token.key&&visible(node))||controls.find(node=>node.dataset.focusKey===token.fallbackKey&&visible(node));
  target?.focus({preventScroll:true});
}
function requestDialogClose(dialog){if(dialog.dispatchEvent(new Event('cancel',{cancelable:true})))dialog.close();}
function captureDialogNavigation(){
  const page=source,dialogs=[...document.querySelectorAll('dialog[open]')];let live=true;
  const changed=records=>{if(records.some(record=>record.target.matches('dialog')))live=false;};
  const observer=new MutationObserver(changed);observer.observe(document.body,{attributes:true,attributeFilter:['open'],subtree:true});
  return {isCurrent:()=>{changed(observer.takeRecords());const open=[...document.querySelectorAll('dialog[open]')];return live&&source===page&&open.length===dialogs.length&&open.every(dialog=>dialogs.includes(dialog));},dispose:()=>{changed(observer.takeRecords());observer.disconnect();}};
}
function requestDialogNavigation(){
  const original=new Set(document.querySelectorAll('dialog[open]'));
  while(true){
    const open=[...document.querySelectorAll('dialog[open]')];if(!open.length)return true;
    // Native modal focus identifies the topmost owner even when DOM order differs.
    const focused=document.activeElement?.closest?.('dialog[open]');
    const dialog=open.includes(focused)?focused:open.at(-1);
    if(!original.has(dialog))return false;
    requestDialogClose(dialog);if(dialog.open)return false;
  }
}
for (const button of document.querySelectorAll('.close-dialog')) button.onclick = () => requestDialogClose(button.closest('dialog'));
function dismissOpenDialog(event){
  if(event.key!=='Escape'||event.isComposing)return;
  if(globalThis.truffleGuidance?.dismiss()){event.preventDefault();event.stopImmediatePropagation();return;}
  const dialog=event.target.closest?.('dialog[open]')||[...document.querySelectorAll('dialog[open]')].at(-1);
  if(!dialog)return;
  if(dialog.id==='settings-dialog'&&typeof settingsDirty!=='undefined'&&settingsDirty){event.preventDefault();event.stopImmediatePropagation();$('#settings-error').textContent='Save your changes, or use Discard before closing.';return;}
  event.preventDefault();event.stopImmediatePropagation();if(!dialog.dismissLayer?.())requestDialogClose(dialog);
}
document.addEventListener('keydown',dismissOpenDialog,true);

for (const dialog of document.querySelectorAll('dialog')) dialog.addEventListener('click', event => { if (event.target === dialog) { if(dialog.id==='settings-dialog'&&typeof settingsDirty!=='undefined'&&settingsDirty)return; const rect = dialog.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) requestDialogClose(dialog); } });
$('#note-form').onsubmit=saveProjectEditor;
$('#note-dialog').addEventListener('cancel',event=>{if(projectEditor?.pending)event.preventDefault();});
$('#note-dialog').addEventListener('close',()=>{if(!$('#note-dialog').open){releaseProjectControls(projectEditor);projectEditor=null;}});
$('#item-dialog').addEventListener('cancel',event=>{if(projectStatusAction?.pending)event.preventDefault();});
$('#item-dialog').addEventListener('close',()=>{if(!$('#item-dialog').open)disposeProjectStatusAction();});
$('#delete-note').onclick = async () => {
  const button=$('#delete-note'),current=projectEditor;if(!current||!current.isCurrent()||current.pending||current.loading||current.accepted||current.unknown||current.conflicted)return;
  if(!button.dataset.confirm){button.dataset.confirm='yes';button.textContent='Click again to delete';return;}
  current.pending=true;holdFormFocus($('#note-form'),current);projectControls($('#note-form'),current,true);
  try{
    const result=await mutate({action:'delete-note',id:current.id});current.localWriteResult=result;current.acceptedText='Project deleted.';
    if(result.kind==='accepted')current.accepted=result;else if(result.kind==='unknown')current.unknown=true;
    if(!current.isCurrent()){toast(localWriteMessage(result,current.acceptedText));return;}
    if(result.kind==='accepted'&&result.presentation==='ready'){$('#note-dialog').close();toast(localWriteMessage(result,current.acceptedText));}
    else{current.ui.message.textContent=localWriteMessage(result,current.acceptedText);if(result.kind!=='rejected'){current.ui.refresh.hidden=false;current.ui.inspect.hidden=false;}}
  }finally{current.pending=false;if(current.isCurrent()){projectControls($('#note-form'),current,!!(current.accepted||current.unknown));restoreFormFocus(current,current.accepted||current.unknown?current.ui.refresh:button);}}
};
async function changeLocalItemStatus(item,status){
  const dialog=$('#item-dialog');if(projectStatusAction)return;
  const acceptedText=status===null?'Using source status.':'Dashboard status updated locally.';
  const current=projectStatusAction={item,page:source,pending:true,refreshing:false},isCurrent=()=>projectStatusAction===current&&dialog.open&&currentItem?.id===item.id&&source===current.page;
  current.isCurrent=isCurrent;if(!isCurrent()){projectStatusAction=null;return;}
  const ui=current.ui=projectNotice(dialog,'Status recovery actions');holdFormFocus(dialog,current);projectControls(dialog,current,true);ui.message.textContent='Saving dashboard status…';
  const presentationPending=error=>{
    if(!isCurrent())return;current.result={kind:'accepted',presentation:'pending',error:error?.message||'The status view could not refresh.'};ui.message.textContent=localWriteMessage(current.result,acceptedText);projectControls(dialog,current,true);ui.refresh.hidden=false;restoreFormFocus(current,ui.refresh);
  };
  const showReady=()=>{
    try{if(!isCurrent())return;const latest=allItems().find(row=>row.id===item.id);releaseProjectControls(current);
    if(latest){currentItem=latest;$('#item-status').value=latest.status;$('#reset-status').disabled=!latest.overridden;renderItemSessions(latest);}
    restoreFormFocus(current,$('#item-status'));ui.region.remove();projectStatusAction=null;toast(localWriteMessage(current.result,acceptedText));
    }catch(error){presentationPending(error);}
  };
  ui.refresh.onclick=async()=>{
    if(!isCurrent()||current.pending||current.refreshing)return;holdFormFocus(dialog,current);current.refreshing=true;projectControls(dialog,current,true);ui.refresh.disabled=true;
    try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('Dashboard changed while refreshing.');if(!isCurrent())return;render();if(current.result.kind==='accepted'){current.result={kind:'accepted',presentation:'ready'};showReady();}else ui.message.textContent=localWriteMessage(current.result,acceptedText)+' The dashboard was refreshed; this action was not repeated.';}
    catch(error){if(current.result.kind==='accepted')presentationPending(error);else if(isCurrent())ui.message.textContent=localWriteMessage(current.result,acceptedText);}
    finally{current.refreshing=false;if(isCurrent()){projectControls(dialog,current,true);ui.refresh.disabled=false;restoreFormFocus(current,ui.refresh);}}
  };
  current.result=await mutate({action:'override',id:item.id,value:{status}});current.pending=false;
  if(!isCurrent()){toast(localWriteMessage(current.result,acceptedText));return;}
  if(current.result.kind==='accepted'&&current.result.presentation==='ready'){showReady();return;}
  ui.message.textContent=localWriteMessage(current.result,acceptedText);
  if(current.result.kind==='rejected'){releaseProjectControls(current);restoreFormFocus(current,$('#item-status'));projectStatusAction=null;}
  else{projectControls(dialog,current,true);ui.refresh.hidden=false;restoreFormFocus(current,ui.refresh);}
}
$('#item-status').onchange = async () => {
  const item=currentItem,status=$('#item-status').value;if(!item)return;
  if(item.source==='notes'){await changeProjectStatus(item,status);return;}
  await changeLocalItemStatus(item,status);
};
$('#reset-status').onclick = async () => {const item=currentItem;if(item)await changeLocalItemStatus(item,null);};
async function refreshSources(force,button){
  button.disabled=true;let receipt;
  try{
    receipt=await api('/api/commands/invoke',{name:'sources.refresh',args:{force}});
    if(Number.isFinite(receipt.forceRefreshAfter)&&receipt.forceRefreshAfter>=0)state={...state,forceRefreshAfter:Math.max(state.forceRefreshAfter||0,receipt.forceRefreshAfter)};
    const result=await loadStateSnapshot({attempts:2});render();
    toast(receipt.message+(result.accepted?'':' Dashboard changed while refreshing. Your newer changes were kept; the next refresh will update source status.'));
  }
  catch(error){toast(receipt?receipt.message+' Dashboard could not refresh: '+error.message:error.message);render();}
}
$('#sync').onclick=()=>refreshSources(false,$('#sync'));
$('#force-refresh').onclick=()=>refreshSources(true,$('#force-refresh'));
$('#export').onclick = async () => {
  try {
    const data = await api('/api/export');
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
    const link = el('a'); link.href = url; link.download = `truffle-${new Date().toISOString().slice(0, 10)}.json`; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch (error) { toast(error.message); }
};
document.addEventListener('keydown', event => {
  if (event.key === '/' && ['all','linear','github','sentry','notes'].includes(source) && !['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement.tagName) && !document.activeElement.isContentEditable && !document.querySelector('dialog[open]')) { event.preventDefault(); $('#search').focus(); }
});
document.addEventListener('DOMContentLoaded', () => { void bootstrapAgentWorkspace(); });
setInterval(() => { if (!document.hidden && !document.querySelector('dialog[open]')) refresh(); }, 10000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
