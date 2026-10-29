/* Agent-first presentation. Durable work/session records remain in the service. */
'use strict';
let selectedAgentKey=typeof prefs.selectedAgentKey==='string'?prefs.selectedAgentKey:'';
let liveBarSignature='',agentViewSignature='',historyArchived=false,workspaceIndexedState=null;
let terminalFocus=false;
let leftSidebarCollapsed=prefs.leftSidebarCollapsed===true;
let agentSidebarCollapsed=prefs.agentContextCollapseVersion===1?prefs.agentSidebarCollapsed!==false:true;
let linkedWorkCollapsed=prefs.agentContextCollapseVersion===1?prefs.linkedWorkCollapsed!==false:true;
let contextSectionsCollapsed=prefs.agentContextCollapseVersion===1&&prefs.contextSectionsCollapsed&&typeof prefs.contextSectionsCollapsed==='object'?{...prefs.contextSectionsCollapsed}:{};
let closedAgentTabs=Array.isArray(prefs.closedAgentTabs)?prefs.closedAgentTabs.filter(id=>typeof id==='string').slice(-20):[],reopeningTab=false,reopenRequests=0;
let snoozedAgentTabs=Object.fromEntries(Object.entries(prefs.snoozedAgentTabs||{}).filter(([key,value])=>key&&value&&Number.isFinite(value.at)));
const terminalViews=new Map();
let agentRename=null;
const terminalPool=el('div','terminal-pool');terminalPool.hidden=true;document.body.append(terminalPool);
function candidateKey(c){return c.host==='embedded'?'embedded:'+c.recordId:c.agent+':'+c.sessionId;}
function liveCandidates(){return (inventory.candidates||[]).filter(c=>c.live===true||(c.live===undefined&&c.running===true&&c.active===true));}
function recordForCandidate(candidate){return (state?.sessions||[]).find(s=>candidate.recordId===s.id||(s.sessionId&&s.agent===candidate.agent&&s.sessionId===candidate.sessionId));}
function candidateForRecord(record){return liveCandidates().find(c=>c.recordId===record.id||(record.sessionId&&record.agent===c.agent&&record.sessionId===c.sessionId));}
function isAgentSnoozed(candidate){return Object.hasOwn(snoozedAgentTabs,candidateKey(candidate));}
function visibleAgentCandidates(){return liveCandidates().filter(c=>!isAgentSnoozed(c));}
function snoozeAgentTab(candidate){
  window.truffleGuidance?.complete('chat.snooze');
  agentRename?.cancel();
  const record=recordForCandidate(candidate);
  snoozedAgentTabs[candidateKey(candidate)]={at:Date.now(),recordId:record?.id||'',title:record?.title||candidate.title||agentName(candidate.agent)};
  if(selectedAgentKey===candidateKey(candidate)){selectedAgentKey='';markAgentNavigation();chooseDefaultAgentSelection();}
  persist();render();toast('Agent snoozed',()=>{delete snoozedAgentTabs[candidateKey(candidate)];selectAgent(record,candidate);});
}
async function restoreSnoozedTab(key,button){
  const entry=snoozedAgentTabs[key];if(!entry)return;
  const candidate=liveCandidates().find(c=>candidateKey(c)===key),record=candidate?recordForCandidate(candidate):state.sessions.find(s=>s.id===entry.recordId);
  if(candidate){selectAgent(record,candidate);$('#snoozed-agents').open=false;return;}
  if(!record){toast('This saved conversation is no longer available.');return;}
  button.disabled=true;
  try{await openSavedSession(record,button);if(candidateForRecord(record)){$('#snoozed-agents').open=false;}}
  catch(error){toast(error.message);}
  finally{button.disabled=false;}
}
function renderSnoozedAgents(){
  const entries=Object.entries(snoozedAgentTabs).sort((a,b)=>Number(snoozeDue(b[1]))-Number(snoozeDue(a[1]))||b[1].at-a[1].at);
  const menu=$('#snoozed-agent-list');menu.replaceChildren();
  const due=entries.filter(([,entry])=>snoozeDue(entry)).length;$('#snoozed-agent-count').textContent=due?due+' due':entries.length||'';$('#snoozed-agents').classList.toggle('has-due',due>0);
  if(!entries.length){menu.append(el('span','snoozed-empty','No snoozed agents'));return;}
  for(const [key,entry]of entries){
    const candidate=liveCandidates().find(c=>candidateKey(c)===key),record=candidate?recordForCandidate(candidate):state.sessions.find(s=>s.id===entry.recordId);
    const row=el('div','snoozed-agent-entry'),open=featureButton('',()=>restoreSnoozedTab(key,open),'snoozed-agent-open');
    open.append(el('strong','',record?.title||entry.title),el('small','',(snoozeDue(entry)?'Due · ':'')+(candidate?'Live · restore tab':record?'Saved · resume chat':'Conversation unavailable')));
    open.title='Snoozed '+new Date(entry.at).toLocaleString();
    const remove=featureButton('×',()=>{delete snoozedAgentTabs[key];persist();render();toast('Removed from snoozed',()=>{snoozedAgentTabs[key]=entry;persist();render();});},'snoozed-agent-forget');
    remove.title='Remove from snoozed · keep conversation in History';remove.setAttribute('aria-label','Remove '+(record?.title||entry.title)+' from snoozed');
    row.append(open,remove);menu.append(row,snoozeScheduleControl(key,entry));
  }
}
function runtimeInfo(candidate,record){
  if(!candidate)return {label:'Closed',kind:'closed',subagents:null,attention:false,summary:'Closed'};
  const report=record?.runtime,fresh=report&&(candidate.host==='embedded'&&candidate.startedAt?report.at>=candidate.startedAt:Date.now()/1000-report.at<600);
  const known={running:['Working','working'],backgroundWorkPending:['Background','background'],background:['Background','background'],idle:['Waiting','waiting'],needsInput:['Needs input','needs-input'],working:['Working','working'],waiting:['Waiting','waiting'],'needs-input':['Needs input','needs-input']};
  const value=fresh?report.state:candidate.lifecycle;
  let [label,kind]=known[value]||['Live · status unknown','unknown'];
  const counts=fresh&&report.backgroundCounts&&typeof report.backgroundCounts==='object'?report.backgroundCounts:{};
  const subagents=Math.max(fresh&&Number.isInteger(report.subagents)?report.subagents:Number.isInteger(candidate.subagents)?candidate.subagents:-1,Number.isInteger(counts.subagent)?counts.subagent:-1);
  if(fresh&&['waiting','needs-input'].includes(kind)){label=({finished:'Finished',question:'Question',approval:'Approval',failed:'Failed',ready:'Ready'})[report.reason]||label;}
  const activity=[];
  if(subagents>0)activity.push(subagents+' sub');
  for(const [type,count]of Object.entries(counts).sort(([a],[b])=>a.localeCompare(b))){if(type!=='subagent'&&Number.isInteger(count)&&count>0)activity.push(count+' '+type+(count===1?'':'s'));}
  if(fresh&&report.scheduledWakeups>0)activity.push(report.scheduledWakeups+' timer'+(report.scheduledWakeups===1?'':'s'));
  if(kind==='waiting'&&activity.length){label='Background';kind='background';}
  return {label,kind,subagents:subagents<0?null:subagents,attention:kind==='needs-input'||kind==='waiting',summary:[label,...activity].join(' · ')};
}
function workSummary(record){const items=record?.items||[];return items.slice(0,2).map(workReference).filter(Boolean).join(' · ')+(items.length>2?' +'+(items.length-2):'');}
function agentIcon(name){
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('class','agent-control-icon');svg.setAttribute('aria-hidden','true');svg.setAttribute('fill','none');svg.setAttribute('stroke','currentColor');svg.setAttribute('stroke-width','1.7');svg.setAttribute('stroke-linecap','round');svg.setAttribute('stroke-linejoin','round');
  const paths={sidebar:'M3 4h18v16H3z M15 4v16',code:'m8 7-5 5 5 5 m8-10 5 5-5 5 M14 4l-4 16',expand:'M8 3H3v5 M16 3h5v5 M3 16v5h5 M21 16v5h-5',restore:'M3 8h5V3 M21 8h-5V3 M8 21v-5H3 M16 21v-5h5',snooze:'M20 15.2A8.4 8.4 0 0 1 8.8 4a8.5 8.5 0 1 0 11.2 11.2Z',plus:'M12 5v14 M5 12h14',chevron:'m9 6 6 6-6 6',investigate:'M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14 M15 15l6 6 M10 7v4 M10 13v.1'};
  if(name==='more'){for(const x of [5,12,19]){const dot=document.createElementNS(svg.namespaceURI,'circle');dot.setAttribute('cx',String(x));dot.setAttribute('cy','12');dot.setAttribute('r','1.2');dot.setAttribute('fill','currentColor');dot.setAttribute('stroke','none');svg.append(dot);}}
  else{const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d',paths[name]||paths.more);svg.append(path);}return svg;
}
function beginAgentRename(){
  if(source!=='agents')return;
  if(agentRename){agentRename.input.focus();return;}
  const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey),record=candidate&&recordForCandidate(candidate);
  const tab=candidate&&$('#live-agent-bar .agent-tab[data-agent-key="'+candidateKey(candidate)+'"]');
  if(!record||!tab){toast('Select a saved agent chat to rename.');return;}
  const input=el('input','agent-title-input');input.value=record.title;input.maxLength=180;input.setAttribute('aria-label','Chat title');input.title='Enter to save · Escape to cancel';
  const editor={input,key:candidateKey(candidate),pending:false,cancel(){if(editor.pending)return;input.remove();agentRename=null;terminalViews.get(candidate.recordId)?.terminal.focus();}};
  agentRename=editor;
  input.onkeydown=async event=>{
    if(event.isComposing)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();editor.cancel();return;}
    if(event.key!=='Enter')return;
    event.preventDefault();event.stopPropagation();if(editor.pending)return;
    const title=input.value.trim();if(!title){input.setCustomValidity('Enter a chat title.');input.reportValidity();return;}
    editor.pending=true;input.disabled=true;
    try{if(!await mutateChatMetadata('chats.rename',{id:record.id,title},{renderAfter:false})){editor.pending=false;input.disabled=false;if(agentRename===editor&&selectedAgentKey===editor.key&&source==='agents')input.focus();return;}const ownsEditor=agentRename===editor;input.remove();if(ownsEditor)agentRename=null;render();if(ownsEditor&&selectedAgentKey===editor.key&&source==='agents')terminalViews.get(candidate.recordId)?.terminal.focus();}
    catch(error){editor.pending=false;input.disabled=false;if(agentRename===editor&&selectedAgentKey===editor.key&&source==='agents')input.focus();toast(error.message);}
  };
  input.oninput=()=>input.setCustomValidity('');tab.append(input);input.focus();input.select();
}
function agentActionsMenu(record){
 const menu=el('details','agent-menu'),summary=el('summary','icon-button');summary.dataset.agentControl='actions';summary.append(agentIcon('more'));summary.title='Agent actions';summary.setAttribute('aria-label','Agent actions');
 const options=el('div','agent-menu-options'),manage=featureButton('Manage session',()=>{menu.open=false;openLaunch(record.agent,null,record);},'text-button');manage.dataset.agentControl='manage';options.append(manage);
 for(const kind of ['work','artifact']){const button=featureButton(kind==='work'?'Link work':'Attach result',()=>{menu.open=false;agentContextReveals.add(record.id+':'+kind);agentSidebarCollapsed=false;contextSectionsCollapsed.results=false;linkedWorkCollapsed=false;render();const section=$(kind==='work'?'.agent-work-section':'.agent-results-section');const attach=$('.inline-attach',section);if(attach)attach.open=true;},'text-button');button.dataset.agentControl='attach-'+kind;options.append(button);}
 menu.append(summary,options);return menu;
}
let agentNavigationEpoch=0,agentNavigationSource=source,agentNavigationKey=selectedAgentKey;
function markAgentNavigation(){agentNavigationEpoch++;agentNavigationSource=source;agentNavigationKey=selectedAgentKey;}
function observeAgentNavigation(){if(agentNavigationSource!==source||agentNavigationKey!==selectedAgentKey)markAgentNavigation();}
function chooseDefaultAgentSelection(){
  if(!state||source!=='agents'||pendingSessionResumes.size||document.querySelector('dialog[open]'))return false;
  const live=visibleAgentCandidates();
  if(live.some(candidate=>candidateKey(candidate)===selectedAgentKey)||(state.sessions||[]).some(record=>selectedAgentKey==='record:'+record.id))return false;
  if(!live.length)return false;
  selectedAgentKey=candidateKey(live[0]);markAgentNavigation();return true;
}
function showAgentWorkspace(){source='agents';markAgentNavigation();chooseDefaultAgentSelection();persist();render();}
async function bootstrapAgentWorkspace(){
  observeAgentNavigation();const epoch=agentNavigationEpoch;
  const [,agents]=await Promise.all([refresh(),loadInventory()]);
  observeAgentNavigation();if(agentNavigationEpoch!==epoch||!agents)return false;
  if(!chooseDefaultAgentSelection())return false;
  persist();render();return true;
}
function selectAgent(record,candidate){
  agentRename?.cancel();
  if(candidate)delete snoozedAgentTabs[candidateKey(candidate)];
  selectedAgentKey=candidate?candidateKey(candidate):'record:'+record.id;source='agents';markAgentNavigation();persist();render();$('#live-agent-bar .selected')?.scrollIntoView({block:'nearest',inline:'nearest'});
}
async function attachCandidate(candidate){
 const existing=recordForCandidate(candidate);if(existing)return existing;
 const result=await prepareSavedChat('attach:'+candidate.agent+':'+candidate.sessionId,'attach',{agent:candidate.agent,sessionId:candidate.sessionId,title:candidate.title||candidate.cwd.split('/').pop(),cwd:candidate.cwd,destination:candidate.host==='embedded'?'embedded':'cmux',itemIds:[]},{open:false});return result.record||null;
}
async function focusTerminal(candidate,record){
  if(candidate?.host==='embedded'){selectAgent(record||recordForCandidate(candidate),candidate);const view=terminalViews.get(candidate.recordId);view?.terminal.focus();return;}
  try{await api('/api/session/focus',{agent:candidate.agent,sessionId:candidate.sessionId});}
  catch(error){toast(error.message);void loadInventory();}
}
function renderTitlebarEditor(){
  let button=$('#titlebar-open-editor');
  if(!button){
    const header=$('#agent-topbar'),launcher=$('.compact-launcher',header);
    if(!header||!launcher)return;
    button=featureButton('',async()=>{
      const directory=button.dataset.directory;if(!directory||button.disabled)return;
      button.disabled=true;
      try{await serviceClient.openDiff(directory);}catch(error){toast(error.message||String(error));}finally{renderTitlebarEditor();}
    },'icon-button titlebar-open-editor');
    button.id='titlebar-open-editor';button.append(agentIcon('code'));
    header.insertBefore(button,launcher);header.append(launcher);
  }
  const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);
  const record=selectedAgentKey.startsWith('record:')?state.sessions.find(s=>s.id===selectedAgentKey.slice(7)):candidate?recordForCandidate(candidate):null;
  const directory=source==='agents'&&record?selectedGitDirectory(record):'';
  button.dataset.directory=directory||'';button.disabled=!serviceClient.isDesktop||!directory;
  const title=directory?'Open chat changes in editor':'Open in editor · select a chat first';
  button.title=title;button.setAttribute('aria-label',title);
}
function renderLiveBar(){
  renderTitlebarEditor();
  const live=visibleAgentCandidates();$('#live-agent-count').textContent=live.length;
  const rows=live.map(c=>{const record=recordForCandidate(c);return {candidate:c,record,runtime:runtimeInfo(c,record)};});
  for(const row of rows)row.runtime.snoozeTipReady=typeof guidanceReadyToSnooze==='function'&&row.candidate.host==='embedded'&&row.runtime.kind==='waiting'&&guidanceReadyToSnooze(row.record?.runtime,row.candidate.startedAt,Date.now()/1000);
  const processKey=c=>typeof c.generation==='string'&&c.generation?JSON.stringify([c.host,c.agent,c.recordId,c.generation]):Number.isFinite(c.startedAt)?JSON.stringify([c.host,c.agent,c.recordId,c.sessionId,c.startedAt]):'';
  const signature=JSON.stringify([source,selectedAgentKey,[...pendingSessionResumes],rows.map(x=>[candidateKey(x.candidate),processKey(x.candidate),x.record?.title,x.record?.items,x.runtime]),snoozedAgentTabs,liveCandidates().filter(isAgentSnoozed).map(c=>[candidateKey(c),recordForCandidate(c)?.title]),inventory.warning]);
  if(signature!==liveBarSignature){
    const bar=$('#live-agent-bar'),focused=document.activeElement,oldTabs=[...bar.querySelectorAll('.agent-tab')],oldTab=focused&&bar.contains(focused)?focused.closest('.agent-tab'):null,page=source,selected=selectedAgentKey;
    const role=agentRename?.input===focused?'rename':focused?.closest?.('[data-agent-control]')?.dataset.agentControl;
    const focus=oldTab&&bar.dataset.agentPage===page&&(bar.dataset.agentSelected===selected||oldTab.dataset.agentKey===selected&&role==='tab')?{node:focused,key:oldTab.dataset.agentKey,process:oldTab.dataset.agentProcess,role,index:oldTabs.indexOf(oldTab)}:null;
    const menus=oldTabs.filter(tab=>bar.dataset.agentPage===page&&bar.dataset.agentSelected===selected&&tab.dataset.agentKey===selected&&tab.dataset.agentProcess&&tab.querySelector('.agent-menu')?.open).map(tab=>[tab.dataset.agentKey,tab.dataset.agentProcess]);
    const targets=[],bind=(node,key,process,role)=>{node.dataset.agentControl=role;targets.push({node,key,process,role});return node;};
    liveBarSignature=signature;bar.dataset.agentPage=page;bar.dataset.agentSelected=selected;bar.replaceChildren();
    for(const [index,{candidate,record,runtime}]of rows.entries()){
      const key=candidateKey(candidate),process=processKey(candidate),tab=el('div','agent-tab');tab.dataset.agentProcess=process;tab.classList.toggle('snooze-tip-ready',runtime.snoozeTipReady);tab.dataset.agentKey=candidateKey(candidate);if(record)tab.dataset.reviewAgent=record.id;
      const button=el('button','live-agent '+runtime.kind+(source==='agents'&&selectedAgentKey===candidateKey(candidate)?' selected':''));
      const words=el('span','live-agent-words');words.append(el('strong','',record?.title||candidate.title||agentName(candidate.agent)));
      if(runtime.attention){const dot=el('i','agent-state-dot attention');dot.setAttribute('aria-hidden','true');button.append(dot);}
      button.append(words,el('span','live-agent-state',runtime.summary));
      if(index<9){button.append(el('kbd','agent-shortcut','⌘'+(index+1)));button.setAttribute('aria-keyshortcuts','Meta+'+(index+1));}
      button.title=`${record?.title||candidate.title} · ${runtime.summary}${runtime.attention?' · Needs your attention':''}${runtime.subagents===null?' · Subagent count unavailable':''} · ${workSummary(record)||candidate.cwd||''} · Rename ⌘R · Tab / Shift+Tab to switch`;
      button.setAttribute('aria-label',button.title);button.setAttribute('aria-pressed',String(source==='agents'&&selectedAgentKey===candidateKey(candidate)));button.onclick=()=>selectAgent(record,candidate);tab.append(bind(button,key,process,'tab'));
      const snooze=featureButton('',()=>snoozeAgentTab(candidate),'agent-tab-snooze');snooze.append(agentIcon('snooze'));
      snooze.title='Snooze tab · keep agent running';snooze.setAttribute('aria-label','Snooze '+(record?.title||candidate.title||agentName(candidate.agent)));tab.append(bind(snooze,key,process,'snooze'));
      if(candidate.host==='embedded'){
        const close=featureButton('×',()=>closeAgentTab(candidate,close),'agent-tab-close');
        close.title='Close terminal · conversation stays in History';close.setAttribute('aria-label','Close terminal for '+(record?.title||candidate.title||agentName(candidate.agent)));tab.append(bind(close,key,process,'close'));
      }
      if(record&&selectedAgentKey===key){tab.classList.add('active-agent-tab');const menu=agentActionsMenu(record);menu.open=!document.querySelector('dialog[open]')&&menus.some(([oldKey,oldProcess])=>oldKey===key&&oldProcess===process);for(const node of menu.querySelectorAll('[data-agent-control]'))bind(node,key,process,node.dataset.agentControl);tab.append(menu);}
      bar.append(tab);
      if(agentRename?.key===key)tab.append(bind(agentRename.input,key,process,'rename'));
    }
    renderSnoozedAgents();
    for(const id of pendingSessionResumes){if(live.some(c=>c.recordId===id))continue;const saved=state.sessions.find(s=>s.id===id),pending=el('span','agent-tab resuming-agent-tab',(saved?.title||'Chat')+' · Resuming…');pending.setAttribute('role','status');bar.append(pending);}
    if(!live.length&&!liveCandidates().length&&!pendingSessionResumes.size)bar.append(el('span','live-bar-empty',inventory.warning?'Live terminal status unavailable · see Diagnostics':'No active agents'));
    if(focus&&source===page&&selectedAgentKey===selected&&bar.isConnected&&$('#live-agent-bar')===bar&&!document.querySelector('dialog[open]')&&(document.activeElement===focus.node||document.activeElement===document.body||!document.activeElement?.isConnected)){
      const visible=node=>{if(!node?.isConnected||node.disabled||node.closest('[hidden]'))return false;for(let child=node,parent=node.parentElement;parent;child=parent,parent=parent.parentElement)if(parent.tagName==='DETAILS'&&!parent.open&&child!==[...parent.children].find(entry=>entry.tagName==='SUMMARY'))return false;return node.getClientRects().length>0;};
      const same=targets.filter(target=>focus.process&&target.key===focus.key&&target.process===focus.process);
      const nearby=[...oldTabs.slice(focus.index+1),...oldTabs.slice(0,focus.index).reverse()].map(tab=>targets.find(target=>target.role==='tab'&&tab.dataset.agentProcess&&target.key===tab.dataset.agentKey&&target.process===tab.dataset.agentProcess));
      const target=same.find(target=>target.role===focus.role&&visible(target.node))?.node||same.find(target=>target.role==='tab'&&visible(target.node))?.node||nearby.find(target=>target&&visible(target.node))?.node||$('#new-note');
      if(visible(target))target.focus({preventScroll:true});
    }
  }
  if(typeof paintReviewDraftBadges==='function')paintReviewDraftBadges();
  updateTabDensity();
  const unread=skillInboxCounts(state);
  const badge=$('#skills-unread');badge.hidden=unread.total===0;badge.textContent=unread.total;
}
function rememberClosedTab(id){closedAgentTabs.push(id);closedAgentTabs=closedAgentTabs.slice(-20);}
async function runTabCommand(command){
  if(document.querySelector('dialog[open]'))return;
  if(command==='rename'){beginAgentRename();return;}
  if(command==='new'){
    if(terminalFocus){terminalFocus=false;render();}
    openLocationPrompt();return;
  }
  if(command==='close'&&$('#launch-options').open){
    $('#launch-options').open=false;$('#location-prompt').hidden=true;
    const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);
    const terminal=terminalViews.get(candidate?.recordId);if(terminal)terminal.terminal.focus();else $('#new-note').focus();return;
  }
  if(command==='close'){
    const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);
    if(source!=='agents'||!candidate){toast('Select an agent tab to close.');return;}
    const button=$('#live-agent-bar .agent-tab[data-agent-key="'+candidateKey(candidate)+'"] .agent-tab-close');
    if(button)await closeAgentTab(candidate,button);return;
  }
  if(command!=='reopen')return;
  reopenRequests++;
  if(reopeningTab)return;
  reopeningTab=true;
  try{
    while(reopenRequests>0){
      reopenRequests--;
      while(closedAgentTabs.length&&!state.sessions.some(s=>s.id===closedAgentTabs.at(-1)))closedAgentTabs.pop();
      const id=closedAgentTabs.at(-1),record=state.sessions.find(s=>s.id===id);
      if(!record){toast('No recently closed agent tabs.');persist();reopenRequests=0;break;}
      const existing=candidateForRecord(record);
      if(existing)selectAgent(record,existing);else await openSavedSession(record);
      await loadInventory();
      if(candidateForRecord(record)){closedAgentTabs.pop();persist();}
      else{reopenRequests=0;break;}
    }
  }finally{reopeningTab=false;}

}
async function closeAgentTab(candidate,button){
  if(button.disabled)return;button.disabled=true;
  try{
    await api('/api/terminal/close',{id:candidate.recordId,generation:candidate.generation});
    inventory={...inventory,candidates:inventory.candidates.filter(c=>candidateKey(c)!==candidateKey(candidate)||c.generation!==candidate.generation)};
    if(selectedAgentKey===candidateKey(candidate)){selectedAgentKey='';markAgentNavigation();chooseDefaultAgentSelection();}
    rememberClosedTab(candidate.recordId);persist();render();await loadInventory();toast('Terminal closed',()=>runTabCommand('reopen'));
  }catch(error){toast(error.message);button.disabled=false;await loadInventory();}
}
function cycleAgentTab(event){
  if(event.key!=='Tab'||event.metaKey||event.ctrlKey||event.altKey||event.isComposing||source!=='agents')return false;
  if(document.querySelector('dialog[open],#agent-topbar details[open],.agent-menu[open]'))return false;
  const target=event.target,terminal=target.closest?.('.xterm');
  if(target!==document.body&&(!terminal||target!==terminal&&!target.matches('textarea.xterm-helper-textarea')))return false;
  const tabs=visibleAgentCandidates();if(!tabs.length)return false;
  const current=tabs.findIndex(c=>candidateKey(c)===selectedAgentKey);
  const index=current<0?(event.shiftKey?tabs.length-1:0):(current+(event.shiftKey?-1:1)+tabs.length)%tabs.length;
  event.preventDefault();event.stopImmediatePropagation();
  const candidate=tabs[index];selectAgent(recordForCandidate(candidate),candidate);terminalViews.get(candidate.recordId)?.terminal.focus();return true;
}
function updateTabDensity(){
  const header=$('#agent-topbar'),count=visibleAgentCandidates().length;
  // Reserve native controls, the launcher, and a drag area before sizing tabs.
  const usable=header.clientWidth-(serviceClient.isDesktop?110:16)-137;
  const width=count?usable/count:Infinity;
  header.classList.toggle('tabs-condensed',width<210);
  header.classList.toggle('tabs-tight',width<145);
  header.classList.toggle('tabs-crowded',width<95);
  if(typeof document!=='undefined'&&document.createElement){
    const tab=$('#live-agent-bar .active-agent-tab');
    if(tab){const strong=$('strong',tab),status=$('.live-agent-state',tab),canvas=document.createElement('canvas'),measure=canvas.getContext('2d');measure.font=getComputedStyle(strong).font;const titleWidth=measure.measureText(strong.textContent).width;const statusWidth=status?.scrollWidth||0;const available=$('#live-agent-bar').clientWidth-Math.max(0,count-1)*54;tab.style.flexBasis=Math.min(Math.max(120,available),Math.ceil(titleWidth+statusWidth+132))+'px';}
  }

}
const titlebarObserver=new ResizeObserver(updateTabDensity);titlebarObserver.observe($('#agent-topbar'));
function parkTerminals(){for(const view of terminalViews.values())if(view.node.isConnected&&!terminalPool.contains(view.node))terminalPool.append(view.node);}
function terminalMultilineKey(terminal,event){
  if(event.isComposing||event.key!=='Enter'||!event.shiftKey||event.ctrlKey||event.metaKey||event.altKey)return true;
  event.preventDefault();
  // ESC + Return is the multiline shortcut supported by Claude and Codex.
  // Keep it on xterm's onData path so it stays ordered with typed/pasted text.
  if(event.type==='keydown')terminal.input('\x1b\r',true);
  return false;
}
function ensureTerminal(record){
  const endpoint=record.terminalRole==='shell'?'/api/shell-terminal':'/api/terminal';
  if(terminalViews.has(record.id))return terminalViews.get(record.id);
  for(const [id,old] of terminalViews){if(terminalViews.size<16)break;if(old.stopped){old.observer.disconnect();old.terminal.dispose();old.node.remove();terminalViews.delete(id);}}
  const node=el('div','embedded-terminal'),status=el('p','terminal-status','Connecting to the local terminal…');node.append(status);
  const screen=el('div','terminal-screen');node.append(screen);
  const terminal=new Terminal({fontFamily:'Menlo, Monaco, monospace',fontSize:12,lineHeight:1.2,scrollback:3000,convertEol:false,cursorBlink:false,allowProposedApi:false,theme:{background:'#191724',foreground:'#e0def4',cursor:'#e0def4',cursorAccent:'#191724',selectionBackground:'#403d52',selectionForeground:'#e0def4',black:'#26233a',red:'#eb6f92',green:'#31748f',yellow:'#f6c177',blue:'#9ccfd8',magenta:'#c4a7e7',cyan:'#ebbcba',white:'#e0def4',brightBlack:'#6e6a86',brightRed:'#eb6f92',brightGreen:'#31748f',brightYellow:'#f6c177',brightBlue:'#9ccfd8',brightMagenta:'#c4a7e7',brightCyan:'#ebbcba',brightWhite:'#e0def4'}});
  if(typeof terminalSettings==='function')Object.assign(terminal.options,terminalSettings());
  const fit=new FitAddon.FitAddon();terminal.loadAddon(fit);terminal.open(screen);
  const view={node,terminal,fit,cursor:0,generation:'',alive:true,stopped:false,cols:0,rows:0};terminalViews.set(record.id,view);
  let inputQueue=Promise.resolve();
  terminal.attachCustomKeyEventHandler(event=>terminalMultilineKey(terminal,event));
  terminal.onData(data=>{inputQueue=inputQueue.then(()=>serviceClient.request(endpoint+'/input',{id:record.id,data})).catch(error=>{status.textContent=error.message;status.hidden=false;});});
  // OSC 52 clipboard and hyperlinks are deliberately not wired to host capabilities.
  const resize=()=>{if(!node.isConnected||screen.clientWidth<20||screen.clientHeight<20)return;fit.fit();const cols=Math.max(10,Math.min(500,terminal.cols)),rows=Math.max(2,Math.min(200,terminal.rows));if(cols===view.cols&&rows===view.rows)return;view.cols=cols;view.rows=rows;void serviceClient.request(endpoint+'/resize',{id:record.id,cols,rows}).catch(()=>{});};
  view.resize=resize;view.observer=new ResizeObserver(resize);view.observer.observe(screen);
  void(async()=>{
    while(!view.stopped){
      try{
        const packet=await serviceClient.request(endpoint+'/poll',{id:record.id,cursor:view.cursor,generation:view.generation});
        if(packet.reset){terminal.reset();if(packet.truncated)terminal.writeln('[Earlier terminal scrollback was discarded.]');}
        view.generation=packet.generation;view.cursor=packet.cursor;view.alive=packet.alive;
        if(packet.bytes.length)await new Promise(resolve=>terminal.write(new Uint8Array(packet.bytes),resolve));
        status.hidden=packet.alive;status.textContent=record.terminalRole==='shell'?'Shell exited. Use Restart shell to open another.':'Agent exited. Resume this conversation from History.';
        if(!packet.alive&&!packet.hasMore){view.stopped=true;void loadInventory();break;}
      }catch(error){status.hidden=false;if(error.message.includes('no longer in memory')){status.textContent=error.message;view.stopped=true;view.alive=false;void loadInventory();break;}status.textContent=error.message+' Reconnecting…';await new Promise(resolve=>setTimeout(resolve,1500));}
    }
  })();
  return view;
}
function agentWork(record){
  if(!record)return [];
  const catalog=new Map([...(state.relatedItems||[]),...allItems(),...record.items].map(x=>[x.id,x]));
  // Prefer full current source records over saved link summaries.
  for(const item of allItems())catalog.set(item.id,item);
  const ids=new Set(record.items.map(x=>x.id));
  return [...ids].map(id=>catalog.get(id)).filter(Boolean);
}
function linkedWorkCI(record,item){
  const group=el('span','context-work-ci'),info=ciInfo(item.ci),badge=featureButton(info.short,()=>openItem(item),'ci-pill '+(item.ci?.state||'unknown'));
  const checks=item.ci?.checks||[],failed=checks.filter(check=>check.state==='failed');
  badge.dataset.chatCi=item.id;badge.title=chatCIHint(item);badge.setAttribute('aria-label',chatCIBadgeLabel(item));group.append(badge);
  const selected=selectedAgentKey,links=(record.items||[]).map(row=>row.id).sort().join('\0');let pending=false,investigate,clear;
  const canPresent=()=>group.isConnected&&source==='agents'&&selectedAgentKey===selected&&(state.sessions.find(row=>row.id===record.id)?.items||[]).map(row=>row.id).sort().join('\0')===links;
  const recovery=chatDispatchRecovery(record.id,{canPresent,onChange:()=>{if(!group.isConnected)return;if(investigate)investigate.disabled=pending||chatDispatchBlocked(record.id);if(clear)clear.disabled=chatDispatchBlocked(record.id);}});
  // One record-level recovery panel, independent of whichever PR owns the receipt.
  queueMicrotask(()=>{if(!group.isConnected)return;const section=group.closest('.agent-work-section');if(section&&!section.querySelector('[data-dispatch-record]'))section.append(recovery);});
  const investigation=record.ciInvestigations?.[item.id],active=['queued','investigating'].includes(investigation?.state);
  if(active){const label=el('small','context-ci-progress',Date.now()/1000-investigation.updatedAt>1800?'Unverified':investigation.state==='queued'?'Queued':'Investigating');label.title='CI investigation · '+relative(investigation.updatedAt);clear=featureIconButton('×','Clear CI investigation indicator',async()=>{if(!canPresent()||chatDispatchBlocked(record.id))return;try{await api('/api/session/ci-investigation',{id:record.id,itemId:item.id,requestId:investigation.id,state:'cancelled'});const snapshot=await loadStateSnapshot({attempts:2});if(snapshot.accepted&&canPresent())render();}catch(error){if(canPresent())toast(error.message);}},'text-button');clear.disabled=chatDispatchBlocked(record.id);group.append(label,clear);}
  if((item.ci?.state==='failed'||failed.length)&&!active&&candidateForRecord(record)?.host==='embedded'){
    investigate=featureButton('',async()=>{
      if(pending||!canPresent())return;pending=true;investigate.disabled=true;
      try{const receipt=await submitChatDispatch('chats.ci.investigate','ci-investigation',{recordId:record.id,itemId:item.id});if(receipt.finalized){await refreshChatDispatch(receipt,{canPresent});if(canPresent())window.truffleGuidance?.complete('ci.investigate');}}
      catch(error){if(canPresent()){const message=el('small','context-ci-error',error.message);message.setAttribute('role','status');group.closest('.context-work-row')?.append(message);}}
      finally{pending=false;if(canPresent())investigate.disabled=chatDispatchBlocked(record.id);}
    },'icon-button investigate-ci');investigate.append(agentIcon('investigate'));investigate.title='Ask this chat to investigate CI failures';investigate.setAttribute('aria-label','Investigate CI failures for '+workReference(item));investigate.disabled=chatDispatchBlocked(record.id);group.append(investigate);
  }
  return group;
}
function renderAgentWorkspace(){
  const live=visibleAgentCandidates();let candidate=live.find(c=>candidateKey(c)===selectedAgentKey);
  let record=selectedAgentKey.startsWith('record:')?state.sessions.find(s=>s.id===selectedAgentKey.slice(7)):candidate?recordForCandidate(candidate):null;
  const list=$('#work-list'),key=candidate?candidateKey(candidate):record?'record:'+record.id:'empty';
  const focusedTerminal=document.activeElement?.classList.contains('xterm-helper-textarea');
  if(agentViewSignature!==key||!$('#agent-workspace')){
    parkTerminals();list.replaceChildren();agentViewSignature=key;
    const pane=el('section','agent-workspace');pane.id='agent-workspace';
    pane.append(el('div','agent-heading'),el('div','agent-conversation'),el('div','agent-context-grid'));list.append(pane);
  }
  document.body.classList.toggle('agent-sidebar-collapsed',agentSidebarCollapsed);
  const pane=$('#agent-workspace'),heading=$('.agent-heading',pane),conversation=$('.agent-conversation',pane),context=$('.agent-context-grid',pane);
  const contextFocused=document.activeElement,contextFocusKey=context.contains(contextFocused)?contextFocused?.dataset.focusKey:null;
  const editing=context.querySelector('.inline-attach[open]');
  const expandedResources=new Set([...context.querySelectorAll('.resource-entry[open]')].map(x=>x.dataset.resourceKey));
  const expandedTrees=new Set([...context.querySelectorAll('.worktree-entry[open]')].map(x=>x.dataset.path));
  heading.replaceChildren();if(!editing)context.replaceChildren();
  if(!candidate&&!record){heading.hidden=false;terminalFocus=false;document.body.classList.remove('terminal-focused');conversation.replaceChildren();heading.append(el('h2','',live.length?'Choose a live chat':Object.keys(snoozedAgentTabs).length?'Agents snoozed':'No active agents'));
    if(Object.keys(snoozedAgentTabs).length)conversation.append(featureButton('Snoozed agents',()=>{$('#snoozed-agents').open=true;},'text-button'));
    conversation.append(featureButton('History',()=>{source='sessions';persist();render();},'text-button'));
    return;
  }
  heading.hidden=true;
  const directory=record?.cwd||candidate?.cwd;
  const embedded=candidate?.host==='embedded';
  if(embedded&&record){
    const existing=terminalViews.get(record.id);
    if(existing?.stopped&&candidate.generation!==existing.generation){existing.observer.disconnect();existing.terminal.dispose();terminalViews.delete(record.id);existing.node.remove();}
    const terminal=ensureTerminal(record);if(!conversation.contains(terminal.node))conversation.replaceChildren(terminal.node);requestAnimationFrame(()=>{terminal.resize();if(focusedTerminal&&!$('#launch-options').open&&(document.activeElement===document.body||terminal.node.contains(document.activeElement)))terminal.terminal.focus();});
  }else{
    const shellPanel=$('.chat-shell-panel',conversation);if(shellPanel){for(const child of [...conversation.children])if(child!==shellPanel)child.remove();}else conversation.replaceChildren();const gateway=el('div','terminal-gateway');
    gateway.append(el('span','terminal-gateway-mark','>_'),el('span','',candidate?'This conversation is running in cmux.':'Saved conversation · no terminal in memory'));
    const opening=record&&savedSessionOpenState(record.id);if(opening&&!opening.resolved){const status=el('p','form-hint',savedSessionOpenMessage(opening));status.setAttribute('role','status');gateway.append(status);}
    const open=featureButton(candidate?'Open terminal ↗':opening?savedSessionOpenAction(opening):'Resume in Truffle',async()=>{
      if(candidate){await focusTerminal(candidate,record);return;}
      try{if(inventory.warning&&record.destination==='cmux'&&!savedSessionOpenState(record.id))throw new Error('Cannot confirm whether this cmux agent is still running. Restore the connection before resuming it.');await openSavedSession(record,open,{target:'embedded'});}catch(error){toast(error.message);}
    },'button primary');if(record)open.dataset.savedOpenRecord=record.id;if(pendingSessionResumes.has(record?.id)){open.disabled=true;open.textContent='Resuming…';}gateway.append(open);if(record&&!candidate&&!opening)gateway.append(featureButton('Start new with context',()=>showSessionRecovery(record,'Use this if the saved conversation is missing or cannot be resumed. Work links and launch instructions are copied; chat history is not.'),'button'));if(shellPanel)shellPanel.before(gateway);else conversation.append(gateway);
    if(candidate){const hint=el('p','form-hint','Existing cmux processes stay in their terminal. New sessions open embedded here. Closed conversations can resume in Truffle with their saved context.');if(shellPanel)shellPanel.before(hint);else conversation.append(hint);}
  }
  if(record&&typeof renderChatTerminal==='function')renderChatTerminal(record,conversation);
  if(editing)return;
  const items=agentWork(record),work=el('section','agent-work-section'),workHead=el('div','section-heading'),workBody=el('div','linked-work-items');
  workBody.id='linked-work-items';workBody.hidden=linkedWorkCollapsed;
  const workToggle=el('button','work-section-toggle'),chevron=el('span','work-section-chevron');chevron.append(agentIcon('chevron'));chevron.setAttribute('aria-hidden','true');
  workToggle.type='button';workToggle.setAttribute('aria-controls',workBody.id);workToggle.setAttribute('aria-expanded',String(!linkedWorkCollapsed));workToggle.setAttribute('aria-label',linkedWorkCollapsed?'Expand linked work':'Collapse linked work');
  workToggle.append(chevron,el('span','','Work'),el('small','',String(items.length)));
  workToggle.onclick=()=>{linkedWorkCollapsed=!linkedWorkCollapsed;workBody.hidden=linkedWorkCollapsed;workToggle.setAttribute('aria-expanded',String(!linkedWorkCollapsed));workToggle.setAttribute('aria-label',linkedWorkCollapsed?'Expand linked work':'Collapse linked work');persist();};
  const workTitle=el('h3');workTitle.append(workToggle);workHead.append(workTitle);if(record)workHead.append(inlineAttach(record,'work'));work.append(workHead,workBody);
  for(const item of items){const row=el('article','context-work-row'),title=el('a','context-work-title',item.title);title.href=item.url||'#';title.target='_blank';title.rel='noopener noreferrer';const meta=el('div','context-work-meta'),ref=el('a','',workReference(item)||'Project');ref.href=item.url||'#';ref.target='_blank';ref.rel='noopener noreferrer';if(!item.url){title.removeAttribute('target');title.onclick=e=>{e.preventDefault();openItem(item);};ref.onclick=e=>{e.preventDefault();openItem(item);};}meta.append(ref);meta.append(el('span','status-pill '+item.status,workStatusLabel(item)),featureIconButton('⋯','Open '+(workReference(item)||item.title),()=>openItem(item)));meta.append(detachLinkButton(record,'itemIds',item.id));const titleRow=el('div','context-work-heading');title.title=item.title;titleRow.append(title);if(item.source==='github'&&item.kind==='Pull request')titleRow.append(linkedWorkCI(record,item));row.append(titleRow,meta);workBody.append(row);}

  const results=el('section','agent-results-section'),resultHead=el('div','section-heading');resultHead.append(el('h3','','Results'));
  if(record)resultHead.append(inlineAttach(record,'artifact'));results.append(resultHead);
  const outputs=(artifactLibrary?.artifacts||state.artifacts||[]).filter(x=>(x.sessionIds||[]).includes(record?.id));
  for(const artifact of outputs){const row=libraryRow({...artifact,available:artifact.available!==false});row.append(detachLinkButton(record,'artifactIds',artifact.id));results.append(row);}

  if(record&&directory)context.append(agentGitSection(record,directory));
  if(record?.resources?.length)context.append(collapsibleContextSection(resourceSection(record,expandedResources),'resources'));
  if(items.length||agentContextReveals.has(record?.id+':work'))context.append(work);
  const trees=(record?.worktrees||[]).filter(x=>x.primary||x.isWorktree);
  if(trees.length){const section=el('section','agent-worktree-section'),head=el('div','section-heading');head.append(el('h3','','Worktrees'));section.append(head);
    const byId=new Map(items.map(item=>[item.id,item]));
    for(const tree of trees){const details=el('details','worktree-entry'),summary=el('summary'),branch=el('span','',`⎇ ${tree.branch}`);details.dataset.path=tree.path;details.open=expandedTrees.has(tree.path);summary.title=tree.path;summary.append(branch,el('small','',tree.repoPath.split('/').pop()+(tree.primary?' · current':'')),detachLinkButton(record,'worktrees',tree.path));details.append(summary);
      const path=el('input','worktree-path');path.readOnly=true;path.value=tree.path;path.setAttribute('aria-label','Worktree path');path.onclick=()=>path.select();details.append(path);if(tree.updatedAt){const stamp=el('small','worktree-observed','Reported '+relative(tree.updatedAt));stamp.title=new Date(tree.updatedAt*1000).toLocaleString();details.append(stamp);}
      const refs=el('div','worktree-items');for(const id of tree.itemIds||[]){const item=byId.get(id);if(item)refs.append(featureButton(workReference(item)||item.title,()=>openItem(item),'text-button'));}details.append(refs);section.append(details);
    }context.append(collapsibleContextSection(section,'worktrees',trees.length));
  }
  if(outputs.length||agentContextReveals.has(record?.id+':artifact'))context.append(collapsibleContextSection(results,'results',outputs.length));void loadArtifactLibrary();
  if(contextFocusKey&&(document.activeElement===contextFocused||document.activeElement===document.body||!document.activeElement?.isConnected)){
    [...context.querySelectorAll('[data-focus-key]')].find(node=>node.dataset.focusKey===contextFocusKey)?.focus({preventScroll:true});
  }
}
function collapsibleContextSection(section,key,count){
  const heading=$('h3',section),head=heading.parentElement,body=el('div','context-section-body');body.id='context-section-'+key;
  const label=heading.textContent,toggle=el('button','work-section-toggle'),chevron=el('span','work-section-chevron');chevron.append(agentIcon('chevron'));chevron.setAttribute('aria-hidden','true');toggle.type='button';toggle.setAttribute('aria-controls',body.id);
  const existingCount=head.querySelector('small');if(count===undefined&&existingCount)count=existingCount.textContent;existingCount?.remove();
  toggle.append(chevron,el('span','',label));if(count!==undefined)toggle.append(el('small','',String(count)));heading.replaceChildren(toggle);
  for(const child of [...section.children])if(child!==head)body.append(child);section.append(body);
  const paint=()=>{const closed=contextSectionsCollapsed[key]!==false;body.hidden=closed;toggle.setAttribute('aria-expanded',String(!closed));toggle.setAttribute('aria-label',(closed?'Expand ':'Collapse ')+label.toLowerCase());};
  toggle.onclick=()=>{contextSectionsCollapsed[key]=contextSectionsCollapsed[key]===false;paint();persist();};paint();return section;
}
function inlineAttach(record,kind){
  const details=el('details','inline-attach'),summary=el('summary');summary.append(agentIcon('plus'));summary.title=kind==='work'?'Link work':'Attach result';summary.setAttribute('aria-label',summary.title);details.append(summary);
  summary.dataset.focusKey=record.id+':attach:'+kind;
  details.dataset.attachRecordId=record.id;details.dataset.attachKind=kind;
  const form=el('form','inline-attach-form'),input=el('input','ui-field'),title=el('input','ui-field'),error=el('p','form-hint');error.setAttribute('role','status');input.required=true;input.placeholder=kind==='work'?'Issue or PR URL':'File path or HTTPS URL';input.setAttribute('aria-label',input.placeholder);form.append(input);
  if(kind==='artifact'){title.placeholder='Title (optional)';title.setAttribute('aria-label','Result title');form.append(title);}
  let attachmentPending=false,attachmentReceipt=null;
  const closeAndRestore=()=>{
    const restore=details.contains(document.activeElement);details.open=false;if(details.isConnected)render();
    if(restore){const opener=details.isConnected?details:[...document.querySelectorAll('.inline-attach')].find(node=>node.dataset.attachRecordId===record.id&&node.dataset.attachKind===kind);opener?.querySelector('summary')?.focus({preventScroll:true});}
  };
  const dismiss=()=>{if(!attachmentPending)closeAndRestore();};
  const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label',summary.title+' actions');
  const submit=el('button','button ui-control','Attach');submit.type='submit';submit.dataset.variant='primary';
  const cancel=featureButton('Cancel',dismiss,'ui-control');cancel.dataset.variant='secondary';
  const retry=featureButton(kind==='artifact'?'Refresh results':'Refresh links',()=>attachSavedLink(true),'ui-control');retry.dataset.variant='secondary';retry.hidden=true;error.hidden=true;actions.append(submit,cancel,retry);form.append(actions,error);details.append(form);
  summary.onclick=event=>{if(details.open){event.preventDefault();dismiss();}};
  details.onkeydown=event=>{if(event.key==='Escape'&&!event.isComposing){event.preventDefault();event.stopPropagation();dismiss();}};
  details.ontoggle=()=>{if(attachmentPending){if(!details.open)details.open=true;return;}if(details.open)input.focus();else if(details.isConnected)render();};
  const savedLabel=()=>kind==='artifact'?'Artifact '+attachmentReceipt.artifactId:'Work links for chat '+attachmentReceipt.recordId;
  async function attachSavedLink(refreshOnly=false){
    if(attachmentPending||(!refreshOnly&&attachmentReceipt))return;
    const focused=details.contains(document.activeElement);
    attachmentPending=true;details.setAttribute('aria-busy','true');summary.setAttribute('aria-disabled','true');submit.disabled=true;input.disabled=true;title.disabled=true;cancel.disabled=true;retry.disabled=true;error.hidden=false;error.textContent=attachmentReceipt?'Refreshing saved links…':kind==='work'?'Linking work…':'Attaching result…';
    if(focused)summary.focus({preventScroll:true});
    try{
      if(!refreshOnly){const target=input.value.trim();const command=kind==='work'?{name:'chats.link-work-url',args:{recordId:record.id,url:target}}:{name:target.startsWith('https://')?'artifacts.attach-url':'artifacts.attach-file',args:{recordId:record.id,title:title.value,...(target.startsWith('https://')?{url:target}:{path:target})}};attachmentReceipt=await api('/api/commands/invoke',command);}
      const [snapshot,library]=await Promise.all([loadStateSnapshot({attempts:2}),kind==='artifact'?loadArtifactLibrary(true):Promise.resolve({accepted:true})]);
      if(!snapshot.accepted||!library.accepted)throw new Error(library.error||'Dashboard changed while refreshing. Your newer changes were kept.');
      if(details.isConnected)closeAndRestore();
    }catch(failure){
      const message=attachmentReceipt?savedLabel()+' saved locally. '+failure.message+' Use '+retry.textContent+' to update this view; do not attach it again.':failure.message;
      if(details.isConnected){error.textContent=message;error.hidden=false;if(retry)retry.hidden=!attachmentReceipt;}else if(attachmentReceipt)toast(savedLabel()+' saved locally. '+failure.message+' Refresh the dashboard to see the saved link; do not attach it again.');
    }finally{
      attachmentPending=false;
      if(details.isConnected){details.setAttribute('aria-busy','false');summary.setAttribute('aria-disabled','false');submit.disabled=!!attachmentReceipt;input.disabled=!!attachmentReceipt;title.disabled=!!attachmentReceipt;cancel.disabled=false;retry.disabled=false;}
    }
  }
  form.onsubmit=async event=>{event.preventDefault();await attachSavedLink();};
  return details;
}
function renderHistory(){
  const list=$('#work-list');list.replaceChildren();const query=$('#search').value.toLowerCase();
  const controls=$('#feature-controls'),focused=document.activeElement,focusKey=controls.contains(focused)?focused?.dataset.historyAction:null;controls.hidden=false;
  const archive=featureButton(historyArchived?'Show current history':'Show archived',()=>{historyArchived=!historyArchived;render();},'text-button');archive.dataset.historyAction='archive';const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label','History actions');actions.append(archive);controls.replaceChildren(actions);
  const recovery=chatPreparationContext();let requests;
  if(recovery.availability==='unavailable'||recovery.total>0){
    const label=recovery.availability==='unavailable'?'Pending chat requests · Unavailable':'Pending chat requests · '+recovery.total;
    requests=featureButton(label,()=>{try{return openChatRecovery();}catch(error){toast(error.message||String(error));}},'ui-control');requests.dataset.variant='secondary';requests.dataset.historyAction='recovery';actions.append(requests);
  }
  if(focusKey&&(document.activeElement===document.body||!document.activeElement?.isConnected))(focusKey==='recovery'&&requests?requests:archive).focus({preventScroll:true});
  const rows=state.sessions.filter(s=>!candidateForRecord(s)&&(historyArchived?!!s.archivedAt:!s.archivedAt)&&[s.title,s.cwd,workSummary(s)].join(' ').toLowerCase().includes(query)).sort((a,b)=>(b.lastOpenedAt||b.updatedAt)-(a.lastOpenedAt||a.updatedAt));
  for(const session of rows){const row=el('article','history-row'),resume=featureButton(savedSessionOpenState(session.id)?savedSessionOpenAction(savedSessionOpenState(session.id)):'Resume',()=>openSavedSession(session,resume),'text-button');resume.dataset.savedOpenRecord=session.id;row.append(el('span','agent-type',agentName(session.agent)),featureButton(session.title,()=>selectAgent(session,null),'history-title'),el('span','history-work',workSummary(session)),el('span','history-time',relative(session.lastOpenedAt||session.updatedAt)),resume,featureIconButton('⋯','Chat options for '+session.title,()=>showSession(session)));list.append(row);}
  if(!rows.length)list.append(el('p','feature-empty','No saved conversations match. Live agents stay in the bar above.'));
}
function editSchedule(preset){return openSkillSchedule(preset);}
async function runSkill(preset,button){
 if(button.disabled)return;button.disabled=true;let receipt;const page=source,key=selectedAgentKey;
 try{
  receipt=await api('/api/commands/invoke',{name:'skills.run',args:{presetId:preset.id}});
  if(receipt.openReceipt){if(!adoptSavedSessionOpen(receipt.openReceipt,receipt.recordId))throw new Error('The skill open receipt could not be confirmed.');await openSavedSession({id:receipt.recordId},null,{isCurrent:()=>source===page&&selectedAgentKey===key,notify:false});toast(savedSessionOpenMessage(savedSessionOpenState(receipt.recordId))+(receipt.runSaved===false?' Run history could not be saved. Inspect the chat; do not run the skill again.':''));return;}
  const result=await loadStateSnapshot({attempts:2});if(!result.accepted)throw new Error('Dashboard changed while refreshing. Your newer changes were kept.');
  const record=state.sessions.find(session=>session.id===receipt.recordId);if(!record)throw new Error('The skill chat is unavailable in the refreshed dashboard.');
  if(receipt.status==='preview'){if(source===page&&selectedAgentKey===key)selectAgent(record,null);toast('Preview prepared. No agent was started.');return;}
  const agents=await loadInventory(true),candidate=agents?.candidates?.find(candidate=>candidate.recordId===record.id&&candidate.host==='embedded'&&candidate.live===true);if(candidate&&source===page&&selectedAgentKey===key)selectAgent(record,candidate);else toast('Skill request accepted. Check its chat in History; do not run the skill again.');
 }catch(error){toast(receipt?(receipt.status==='unknown'?'Skill open outcome unknown':receipt.status==='preview'?'Skill preview prepared':'Skill request accepted')+' for chat '+receipt.recordId+'. '+error.message+' Refresh Skills to inspect it; do not run the skill again.':error.message);}
 finally{button.disabled=false;}
}
function skillOutputs(preset,artifacts){return artifacts.filter(a=>isSkillOutput(a)&&a.presetId===preset.id).sort((a,b)=>b.at-a.at);}
function renderSkills(){
  const list=$('#work-list');list.replaceChildren();const controls=$('#feature-controls');controls.hidden=false;
  controls.replaceChildren(featureButton('＋ Add skill',()=>openSkillEditor(),'button'),el('span','form-hint','On demand or recurring · schedules run locally, without replaying a backlog'));
  if(!artifactLibrary){list.append(el('p','form-hint',libraryError||'Loading skills…'));void loadArtifactLibrary();return;}
  $('#search').placeholder='Search skills and results…';const query=$('#search').value.toLowerCase();
  for(const preset of artifactLibrary.presets){
    const results=skillOutputs(preset,artifactLibrary.artifacts);if(![preset.title,preset.command,...results.map(a=>a.title)].join(' ').toLowerCase().includes(query))continue;
    const schedule=state.skillSchedules?.[preset.id],row=el('article','skill-row'),words=el('div','skill-name');words.append(el('strong','',preset.title),el('small','',preset.command+' · '+agentName(preset.agent)+(preset.available?'':' · Folder or skill unavailable')));
    const when=schedule?.enabled?(schedule.mode==='daily'?'Daily '+schedule.time:'Every '+schedule.everyMinutes+' min'):'On demand';
    const timing=el('span','skill-timing',when);timing.title=schedule?.enabled?'Next: '+dated(schedule.nextRun)+(schedule.missed==='skip'?' · Skip missed runs':' · Catch up once'):'';
    const run=featureButton('Run now',()=>runSkill(preset,run),'button');run.disabled=!preset.available;
    row.append(words,timing,featureButton('Schedule',()=>editSchedule(preset),'text-button'),run);const group=el('section','skill-group');group.append(row);
    const runs=(state.skillRuns||[]).filter(r=>r.presetId===preset.id);
    if(results.length){const output=el('div','skill-results');for(const artifact of results.slice(0,20))output.append(libraryRow(artifact));group.append(output);}else group.append(el('p','form-hint skill-empty','No results yet. Reports from this skill will appear here.'));
    if(runs.length){const history=el('details','skill-history'),summary=el('summary','',runs.length+' recent runs');history.append(summary);for(const entry of runs.slice(0,20)){const record=state.sessions.find(r=>r.id===entry.sessionId),line=el('div','skill-run-row');line.append(el('time','',dated(entry.at)),el('span','run-outcome '+entry.outcome,entry.outcome),record?featureButton('Open run',()=>selectAgent(record,candidateForRecord(record)),'text-button'):el('span','',entry.message||''));history.append(line);}group.append(history);}list.append(group);
  }
  if(!list.children.length)list.append(el('p','form-hint','No skills match. Add an installed skill to run it on demand or schedule it.'));
  void loadArtifactLibrary();
}
$('#skills-inbox').onclick=(()=>{
 let pending=false;
 return async()=>{
  try{
   source='skills';persist();render();if(pending)return;
   pending=true;const button=$('#skills-inbox'),busy=button.getAttribute('aria-busy');button.setAttribute('aria-busy','true');
   try{const result=await presentLocalWrite(()=>api('/api/skills/read',{}));if(result.kind!=='accepted'||result.presentation!=='ready')toast(localWriteMessage(result,'Skills inbox marked read.'));}
   finally{pending=false;if(busy===null)button.removeAttribute('aria-busy');else button.setAttribute('aria-busy',busy);}
  }catch(error){toast(error.message);}
 };
})();
function parseCSV(text){
  const rows=[];let row=[],value='',quoted=false;
  for(let i=0;i<text.length;i++){const c=text[i];if(c==='"'){if(quoted&&text[i+1]==='"'){value+='"';i++;}else if(quoted||!value)quoted=!quoted;else value+=c;}
    else if(!quoted&&(c===','||c==='\n'||c==='\r')){row.push(value);value='';if(c!==','){rows.push(row);row=[];if(c==='\r'&&text[i+1]==='\n')i++;}}else value+=c;
    if(rows.length>10000)throw new Error('Tables support up to 10,000 rows.');}
  if(quoted)throw new Error('CSV has an unterminated quoted field.');if(value||row.length){row.push(value);rows.push(row);}return rows;
}
function appendResultTable(node,text,format){
  const data=format==='json'?JSON.parse(text):null;
  if(data&&(!['truffle.table/v1','mydash.table/v1'].includes(data.schema)||!Array.isArray(data.columns)||!Array.isArray(data.rows)))throw new Error('Unsupported table schema.');
  const values=data?[data.columns,...data.rows]:parseCSV(text);if(!values.length)return;
  const table=el('table','result-table'),head=el('thead'),header=el('tr');for(const value of values[0])header.append(el('th','',String(value)));head.append(header);table.append(head);
  const body=el('tbody');for(const row of values.slice(1,1001)){const tr=el('tr');for(const value of row)tr.append(el('td','',value===null?'':String(value)));body.append(tr);}table.append(body);node.append(table);
  if(values.length>1001)node.append(el('p','form-hint',`Showing 1,000 of ${values.length-1} rows. The saved file contains the full table.`));
}
const workSessionControls=itemSessionChip;
itemSessionChip=function(item){const control=workSessionControls(item),record=linkedSessions(item).find(s=>s.workflow!=='pr-review'&&candidateForRecord(s));if(record){const button=control.querySelector('button');button.title='Show agent workspace · '+record.title;button.onclick=()=>selectAgent(record,candidateForRecord(record));}return control;};
let subnavSignature='';
function renderSourceSubmenus(items){
  const linear=$('#linear-subnav');linear.hidden=source!=='linear';
  const projects=[...new Set(items.filter(item=>item.source==='linear'&&!item.archivedAt).map(scopeKey))].sort((a,b)=>a.localeCompare(b));
  if(scopePrefs.linear&&!projects.includes(scopePrefs.linear))projects.push(scopePrefs.linear);
  const counts=new Map();for(const item of items.filter(item=>workViewAllows(item,'linear')))counts.set(scopeKey(item),(counts.get(scopeKey(item))||0)+1);
  const signature=JSON.stringify([source==='linear',scopePrefs.linear,projects,[...counts]]);
  if(signature!==subnavSignature){
    subnavSignature=signature;const focused=document.activeElement?.dataset.linearProject,scroll=linear.scrollTop;
    linear.replaceChildren(...projects.map(project=>{
      const button=el('button','source-project'+(source==='linear'&&scopePrefs.linear===project?' active':''));button.type='button';button.dataset.linearProject=project;button.title=project;button.setAttribute('aria-label','Filter Linear project: '+project);button.setAttribute('aria-pressed',String(source==='linear'&&scopePrefs.linear===project));button.append(el('span','',project),el('b','',counts.get(project)||0));
      button.onclick=()=>{source='linear';scopePrefs.linear=scopePrefs.linear===project?'':project;if(typeof setSavedViewScope==='function')setSavedViewScope(scopePrefs.linear,'linear');persist();render();};return button;
    }));
    linear.scrollTop=scroll;if(focused!==undefined)for(const button of linear.children)if(button.dataset.linearProject===focused)button.focus({preventScroll:true});
  }
}
function prepareWorkspaceRender(){
  if(typeof prepareSavedViewPage==='function')prepareSavedViewPage();observeAgentNavigation();$('#search').placeholder=source==='skills'?'Search skills and results…':'Fuzzy find work…';if(typeof maybeOfferOnboarding==='function')void maybeOfferOnboarding();document.body.classList.toggle('agent-page',source==='agents');if(source!=='agents')terminalFocus=false;document.body.classList.toggle('terminal-focused',terminalFocus);if(workspaceIndexedState!==state){buildBundleIndex(allItems());workspaceIndexedState=state;}const navigationItems=allItems();for(const name of ['all','linear','github','sentry','notes'])$('#count-'+name).textContent=workViewCount(navigationItems,name);$('#count-sessions').textContent=state.sessions.filter(x=>!candidateForRecord(x)&&!x.archivedAt).length;$('#count-changes').textContent=(state.changes||[]).filter(x=>!x.read).length;renderLiveBar();renderSourceSubmenus(navigationItems);
}
registerRenderHook('workspace:prepare',prepareWorkspaceRender,{phase:'before'});
function renderWorkspacePage(){
  if(['agents','skills','sessions','chat-results'].includes(source)){
    $('#page-title').textContent=source==='agents'?'Agents':source==='skills'?'Skills':'History';
    for(const button of document.querySelectorAll('[data-source]'))button.classList.toggle('active',button.dataset.source===source);
    for(const selector of ['#active-sessions','.metrics','#sentry-note','#sentry-controls','#scope-filters','#hidden-items','#snoozed-items','#session-filter-bar','#project-only','#feature-controls'])$(selector).hidden=true;
    $('.toolbar').hidden=source==='agents'||source==='chat-results';$('#status-filter').hidden=true;$('#group-by').hidden=true;
    $('.launcher').hidden=false;renderFolderSelector();
    if(source==='agents'){renderAgentWorkspace();renderLiveBar();}else{parkTerminals();if(source==='skills')renderSkills();else if(source==='chat-results')renderChatSearchResults();else renderHistory();}
    renderSync();return;
  }
  parkTerminals();$('.toolbar').hidden=false;renderWorkPage();paintWorkSelection();$('#active-sessions').hidden=true;$('#count-sessions').textContent=state.sessions.filter(x=>!candidateForRecord(x)&&!x.archivedAt).length;renderLiveBar();
}
registerRenderHook('workspace:controls',()=>{
  if(typeof renderSavedViewControls==='function')renderSavedViewControls();
  if(typeof renderSavedViewSidebar==='function')renderSavedViewSidebar();
  if(typeof renderPageSettingsGear==='function')renderPageSettingsGear();
},{order:10});
for(const button of document.querySelectorAll('[data-source]'))if(['move','pinned','briefing','newsletters','artifacts','archive'].includes(button.dataset.source))button.hidden=true;
$('#summarize-work').hidden=true;
$('.workspace').hidden=true;$('.brand').setAttribute('href','#');$('.brand').onclick=event=>{event.preventDefault();showAgentWorkspace();};

// App shortcuts use Command on macOS, preserving terminal Control sequences.
const sidebarKeys={g:'github','shift+p':'notes','shift+l':'linear',s:'sentry','shift+a':'agents',i:'all','shift+s':'skills','shift+h':'sessions','shift+c':'changes'};
const workSearchSources=new Set(['all','linear','github','sentry','notes']);
let keyboardWorkId='',keyboardWorkSource='',workKeyboardMode=false;
function workRows(){return [...document.querySelectorAll('#work-list .row[data-work-id]')];}
function paintWorkSelection(){
  if(keyboardWorkSource!==source){keyboardWorkSource=source;keyboardWorkId='';workKeyboardMode=false;}
  const rows=workRows();
  if(!rows.some(row=>row.dataset.workId===keyboardWorkId))keyboardWorkId=rows[0]?.dataset.workId||'';
  for(const row of rows)row.classList.toggle('keyboard-selected',workKeyboardMode&&row.dataset.workId===keyboardWorkId);
}
function chooseWorkRow(row,focus=true){
  if(!row)return;keyboardWorkId=row.dataset.workId;workKeyboardMode=true;paintWorkSelection();
  if(focus){row.focus({preventScroll:true});row.scrollIntoView({block:'nearest'});}
}
function openKeyboardWork(external){
  const item=allItems().find(x=>x.id===keyboardWorkId);if(!item)return;
  if(external&&item.url){const link=el('a');link.href=item.url;link.target='_blank';link.rel='noopener noreferrer';document.body.append(link);link.click();link.remove();}
  else if(item.source==='notes')openNote(item);else openItem(item);
}
const originalSearchInput=$('#search').oninput;
$('#search').oninput=event=>{keyboardWorkId='';workKeyboardMode=workSearchSources.has(source);originalSearchInput?.(event);};
for(const [key,name]of Object.entries(sidebarKeys)){
  const button=$('#sources [data-source="'+name+'"]');if(!button)continue;
  const hint=el('kbd','nav-shortcut');hint.hidden=true;button.append(hint);
}
$('#new-note').title='Start session';
document.addEventListener('keydown',event=>{
  if(event.isComposing||event.altKey)return;
  if(cycleAgentTab(event))return;
  const primary=/Mac|iPhone|iPad/.test(navigator.platform)?event.metaKey:event.ctrlKey;
  const consume=()=>{event.preventDefault();event.stopImmediatePropagation();};
  if(primary&&!(typeof appSettings!=='undefined'&&appSettings)){
    if(event.shiftKey&&event.key==='Enter'&&source==='agents'&&$('#agent-workspace .embedded-terminal')){consume();terminalFocus=!terminalFocus;render();return;}
    const key=event.key.toLowerCase();
    if(!event.shiftKey&&['h','l'].includes(key)&&!document.querySelector('dialog[open]')){consume();toggleSidebar(key==='h'?'left':'right');return;}
    if(key==='r'&&!event.shiftKey&&source==='agents'&&!document.querySelector('dialog[open]')){consume();beginAgentRename();return;}
    if(key==='t'||(key==='w'&&!event.shiftKey)){consume();void runTabCommand(key==='w'?'close':event.shiftKey?'reopen':'new');return;}
    const chord=(event.shiftKey?'shift+':'')+key;
    if(sidebarKeys[chord]&&!document.querySelector('dialog[open]')){consume();source=sidebarKeys[chord];if(typeof currentSavedViewState==='function'&&currentSavedViewState()?.id)resetSavedViewState();if(source==='linear')scopePrefs.linear='';if(typeof savedViewUsePage==='function')savedViewUsePage(source);persist();render();return;}
    if(event.shiftKey)return;
    if(key==='n'&&!document.querySelector('dialog[open]')){consume();if(terminalFocus){terminalFocus=false;render();}openLocationPrompt();return;}
    if(document.querySelector('dialog[open]'))return;
    if(/^[1-9]$/.test(key)){const candidate=visibleAgentCandidates()[Number(key)-1];if(candidate){consume();selectAgent(recordForCandidate(candidate),candidate);terminalViews.get(candidate.recordId)?.terminal.focus();}return;}
    return;
  }
  if(event.metaKey||event.ctrlKey||!workSearchSources.has(source)||document.querySelector('dialog[open]'))return;
  const target=event.target,search=target===$('#search');
  const editing=target.closest('input,textarea,select,[contenteditable="true"],.xterm');
  if(event.key==='/'&&!editing){consume();workKeyboardMode=true;$('#search').focus();$('#search').select();paintWorkSelection();return;}
  if(editing&&!search)return;
  if(event.key==='ArrowDown'||event.key==='ArrowUp'){
    const rows=workRows();if(!rows.length)return;consume();
    const current=rows.findIndex(row=>row.dataset.workId===keyboardWorkId);
    // First arrow leaves the search field on the currently highlighted match.
    const index=search?Math.max(0,current):Math.max(0,Math.min(rows.length-1,current+(event.key==='ArrowDown'?1:-1)));
    if(!search&&current===rows.length-1&&event.key==='ArrowDown'){
      const more=$('#work-list [data-focus-key="work:load-more"]');
      if(more){more.click();chooseWorkRow(workRows()[current+1]);return;}
    }
    chooseWorkRow(rows[index]);return;
  }
  if(search&&(event.key==='Enter'||event.key==='Escape')){consume();chooseWorkRow(workRows().find(row=>row.dataset.workId===keyboardWorkId));return;}
  if(!search&&workKeyboardMode&&(event.key.toLowerCase()==='o'||event.key==='Enter')){consume();openKeyboardWork(event.key.toLowerCase()==='o');}
},true);
document.addEventListener('click',event=>{const snoozed=$('#snoozed-agents');if(snoozed.open&&!snoozed.contains(event.target))snoozed.open=false;const options=$('#launch-options');if(options.open&&!options.contains(event.target))options.open=false;const folder=$('#folder-dialog');if(folder.open&&!folder.contains(event.target)&&!event.target.closest('#launch-options'))folder.close();});
document.addEventListener('keydown',event=>{if(event.key==='Escape'){$('#snoozed-agents').open=false;$('#launch-options').open=false;if($('#folder-dialog').open)$('#folder-dialog').close();}});

$('#choose-session-folder').onclick=()=>quickStart(null,null,{chooseFolder:true});
