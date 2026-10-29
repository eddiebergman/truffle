'use strict';
let actionEntries=[],actionSelected=0;
function currentAgentRecord(){const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);return candidate?recordForCandidate(candidate):(state?.sessions||[]).find(s=>selectedAgentKey==='record:'+s.id);}
function openCommandLink(url){const link=el('a');link.href=url;link.target='_blank';link.rel='noopener noreferrer';document.body.append(link);link.click();link.remove();}
const refreshingClaudeSessions=new Set();
async function refreshClaudeSession(record){
  if(refreshingClaudeSessions.has(record.id))return;
  if(record.agent!=='claude'||!record.sessionId)throw new Error('Wait for this Claude chat to register its conversation before refreshing it.');
  refreshingClaudeSessions.add(record.id);
  try{
    await loadInventory();const candidate=candidateForRecord(record);
    if(candidate?.host!=='embedded'||!candidate.generation)throw new Error('Select a running embedded Claude chat to refresh.');
    const focused=selectedAgentKey===candidateKey(candidate);
    await api('/api/terminal/close',{id:record.id,generation:candidate.generation});
    const terminal=terminalViews.get(record.id);if(terminal)terminal.stopped=true;
    if(focused)selectedAgentKey='record:'+record.id;
    await loadInventory();await openSavedSession(record);await loadInventory();
    const resumed=candidateForRecord(record);
    if(focused&&selectedAgentKey==='record:'+record.id&&resumed)selectAgent(record,resumed);
  }finally{refreshingClaudeSessions.delete(record.id);}
}
function commandEntries(){
  const entries=[],record=currentAgentRecord(),candidate=record&&candidateForRecord(record);
  const add=(verb,noun,detail,run,extra={})=>entries.push({title:verb+': '+noun,detail,run,...extra});
  if(record){
    for(const item of agentWork(record))if(item.url)add('open',item.source+' · '+(workReference(item)||item.title),item.title,()=>openCommandLink(item.url),{item});
    for(const tree of gitDiffDirectories(record)){
      const label=tree.branch||tree.path.split('/').pop(),refs=(tree.itemIds||[]).map(id=>(record.items||[]).find(x=>x.id===id)).filter(Boolean).map(workReference).join(' · ');
      add('review','changes · '+label,[refs,tree.path].filter(Boolean).join(' · '),()=>{selectAgent(record,candidate);openDiffReview(record,undefined,tree.path);});
      if(serviceClient.isDesktop)add('open','editor diff · '+label,tree.path,()=>serviceClient.openDiff(tree.path));
    }
    for(const result of (artifactLibrary?.artifacts||state.artifacts||[]).filter(x=>(x.sessionIds||[]).includes(record.id)))add('open','result · '+result.title,[result.format,result.path||result.url].filter(Boolean).join(' · '),()=>viewArtifact(result));
    for(const resource of record.resources||[])for(const ref of resourceLinks(resource)){
      try{const url=new URL(ref.value);if(!['https:','http:'].includes(url.protocol)||url.username||url.password)continue;add('open','resource · '+(resource.name||resource.id)+' · '+ref.category,resource.state||'Unknown',()=>openCommandLink(url.href));}catch(_){}
    }
    add('copy','work links',record.title,()=>serviceClient.copyText(agentWork(record).filter(x=>x.url).map(x=>'['+(workReference(x)||x.title)+']('+x.url+')').join('\n')));
    if(candidate){
      if(record.agent==='claude'&&candidate.host==='embedded')add('refresh','claude session','Close and resume this chat in '+record.cwd+' · interrupts running commands and subagents',()=>refreshClaudeSession(record));
      add('rename','chat',record.title,()=>{selectAgent(record,candidate);beginAgentRename();},{shortcut:'⌘R'});
      add('snooze','chat',record.title,()=>snoozeAgentTab(candidate));
      add('close','chat',record.title,()=>{selectAgent(record,candidate);return runTabCommand('close');},{shortcut:'⌘W'});
      add('toggle','terminal focus',terminalFocus?'Exit focus':'Expand terminal',()=>{selectAgent(record,candidate);terminalFocus=!terminalFocus;render();},{shortcut:'⌘⇧↵'});
    }
  }
  for(const entry of liveCandidates()){
    const linked=recordForCandidate(entry),snoozed=isAgentSnoozed(entry);add(snoozed?'restore':'switch','agent · '+(linked?.title||entry.title),runtimeInfo(entry,linked).summary+' · '+(linked?.cwd||entry.cwd||''),()=>selectAgent(linked,entry));
  }
  for(const [key,entry]of Object.entries(snoozedAgentTabs))if(!liveCandidates().some(c=>candidateKey(c)===key))add('restore','agent · '+entry.title,'Saved conversation',()=>restoreSnoozedTab(key,{disabled:false}));
  for(const agent of ['claude','codex'])add('new',agent+' chat','Choose a local folder',()=>{$('#quick-agent').value=agent;updateQuickAgent();return runTabCommand('new');});
  if(closedAgentTabs.length)add('reopen','closed chat','Restore the last closed tab',()=>runTabCommand('reopen'),{shortcut:'⌘⇧T'});
  for(const [view,label]of [['agents','agents'],['all','all items'],['linear','linear'],['github','github'],['sentry','sentry'],['notes','projects'],['skills','skills'],['sessions','activity']])add('go',label,'Dashboard view',()=>{source=view;if(typeof currentSavedViewState==='function'&&currentSavedViewState()?.id)resetSavedViewState();if(typeof savedViewUsePage==='function')savedViewUsePage(view);if(view==='agents'){showAgentWorkspace();return;}persist();render();},{shortcut:shortcutLabel(setting('keybindings',{})['view-'+view])});
  for(const view of (typeof savedViewsDocument!=='undefined'?savedViewsDocument?.views||[]:[]))add('go',view.name,(sourceNames[view.parent]||view.parent)+' saved view · '+savedViewSummary(view.filter),()=>activateSavedView(view.id));
  add('toggle','left sidebar',leftSidebarCollapsed?'Show navigation':'Hide navigation',()=>toggleSidebar('left'),{shortcut:'⌘H'});
  add('toggle','right sidebar',agentSidebarCollapsed?'Show agent context':'Hide agent context',()=>toggleSidebar('right'),{shortcut:'⌘L'});
  add('change','theme','Choose a light or dark theme',()=>openThemeChooser());
  if(source==='agents'&&record)add('toggle','chat terminal','Shell in this chat’s working folder',()=>toggleChatTerminal(),{shortcut:'⌘J'});
  add('find','chats','Search live, snoozed and saved chats',()=>showChatResults());
  add('find','work items','Search tickets, PRs, projects, and saved chats',()=>openPalette(),{shortcut:'⌘P'});
  add('refresh','work','Sync connected sources; respects cooldowns',()=>$('#force-refresh').click());
  add('open','sharing and inbox','Share a chat or import a context',()=>{$('#sharing-menu').open=true;});
  const recovery=chatPreparationContext();
  if(recovery.availability==='unavailable'||recovery.total>0)add('open','chat recovery',recovery.availability==='unavailable'?'Local request status unavailable. Inspect saved recovery data.':recovery.total+' pending local chat '+(recovery.total===1?'request':'requests'),()=>openChatRecovery());
  if(typeof presentationCommands==='function')for(const entry of presentationCommands())add(entry.verb,entry.noun,entry.detail,entry.run);
  if(typeof setting==='function'){
    add('open','settings','Appearance, shortcuts, agents, quick actions and connections',openSettings,{shortcut:shortcutLabel(setting('keybindings',{}).settings)});
    for(const action of setting('quickActions',[])){const identity={revision:appSettings.revision,contentHash:appSettings.contentHash};add('run',action.title,action.command,()=>chooseQuickAction(action,identity));}
    const ids={'toggle: left sidebar':'toggle-left','toggle: right sidebar':'toggle-right','find: work items':'find-work','rename: chat':'agent-rename','close: chat':'agent-close','reopen: closed chat':'agent-reopen','toggle: terminal focus':'terminal-focus'};
    const views={agents:'agents','all items':'all',linear:'linear',github:'github',sentry:'sentry',projects:'notes',skills:'skills',activity:'sessions'};
    for(const entry of entries){const id=ids[entry.title]||(entry.title.startsWith('go: ')?'view-'+views[entry.title.slice(4)]:null);if(id)entry.shortcut=shortcutLabel(setting('keybindings',{})[id]);}
  }
  return entries;
}
function openCommands(){
  if(!state)return;if($('#action-palette').open){requestDialogClose($('#action-palette'));return;}
  if(!requestDialogNavigation())return false;
  $('#action-query').value='';$('#action-context').textContent=currentAgentRecord()?.title||'Dashboard';$('#action-palette').showModal();$('#action-query').setAttribute('aria-expanded','true');paintCommands(true);$('#action-query').focus();
}
function paintCommandSelection({scroll=false}={}){
  const rows=$('#action-results').querySelectorAll('.action-command');
  for(const [index,row]of rows.entries()){const selected=index===actionSelected;row.classList.toggle('selected',selected);row.setAttribute('aria-selected',String(selected));}
  const query=$('#action-query'),selected=rows[actionSelected];
  if(selected){query.setAttribute('aria-activedescendant',selected.id);if(scroll)selected.scrollIntoView({block:'nearest'});}
  else query.removeAttribute('aria-activedescendant');
}
function paintCommands(reset){
  const query=$('#action-query').value.trim().toLowerCase().replaceAll(':',' '),words=query.split(/\s+/).filter(Boolean);
  const matches=commandEntries().map((entry,index)=>{
    const text=(entry.title+' '+entry.detail).toLowerCase().replaceAll(':',' ');
    return {entry,index,direct:words.every(word=>text.includes(word)),score:query?fuzzyScore(query,text):0};
  }).filter(x=>Number.isFinite(x.score));
  const direct=matches.filter(x=>x.direct);
  actionEntries=(direct.length?direct:matches).sort((a,b)=>b.score-a.score||a.index-b.index).map(x=>x.entry);
  if(reset)actionSelected=0;actionSelected=Math.max(0,Math.min(actionSelected,actionEntries.length-1));
  const list=$('#action-results');list.replaceChildren();
  for(const [index,entry]of actionEntries.entries()){
    const button=featureButton('',()=>runPaletteCommand(index),'action-command');button.id='action-command-'+index;button.tabIndex=-1;button.setAttribute('role','option');
    button.onmousedown=event=>event.preventDefault();
    button.onfocus=()=>{actionSelected=index;paintCommandSelection();};
    const heading=el('div','action-command-heading');heading.append(el('strong','',entry.title));if(entry.shortcut)heading.append(el('kbd','',entry.shortcut));button.append(heading,el('small','',entry.detail));if(entry.item)button.append(paletteMetadata({item:entry.item}));list.append(button);
  }
  if(!actionEntries.length)list.append(el('p','form-hint','No matching commands.'));
  if(reset)list.scrollTop=0;
  paintCommandSelection();
}
async function runPaletteCommand(index){const entry=actionEntries[index];if(!entry)return;$('#action-palette').close();try{await entry.run();}catch(error){toast(error.message||String(error));}}
$('#action-query').oninput=event=>{if(!event.isComposing)paintCommands(true);};
$('#action-query').addEventListener('compositionend',()=>paintCommands(true));
$('#action-palette').addEventListener('close',()=>{
  $('#action-query').value='';$('#action-query').setAttribute('aria-expanded','false');$('#action-query').removeAttribute('aria-activedescendant');actionEntries=[];actionSelected=0;
});
$('#action-palette').addEventListener('keydown',event=>{
  if(event.isComposing||event.keyCode===229||event.metaKey||event.ctrlKey||event.altKey||event.shiftKey)return;
  if(event.target!==$('#action-query')&&!event.target.closest('.action-command'))return;
  if(['ArrowDown','ArrowUp'].includes(event.key)){
    event.preventDefault();event.stopPropagation();
    if(!actionEntries.length)return;
    actionSelected=(actionSelected+(event.key==='ArrowDown'?1:-1)+actionEntries.length)%actionEntries.length;
    $('#action-query').focus({preventScroll:true});paintCommandSelection({scroll:true});
  }else if(event.key==='Enter'){event.preventDefault();event.stopPropagation();void runPaletteCommand(actionSelected);}
});
$('#command-open').onclick=openCommands;
window.addEventListener('truffle-ui-command',event=>{
  if(event.detail==='settings')openSettings();else if(event.detail==='commands')openCommands();else if(event.detail==='find-work'){if($('#command-dialog').open)requestDialogClose($('#command-dialog'));else openPalette();}
  else if(event.detail==='toggle-left')toggleSidebar('left');else if(event.detail==='toggle-right')toggleSidebar('right');
});
document.addEventListener('keydown',event=>{if(!(typeof appSettings!=='undefined'&&appSettings)&&(/Mac|iPhone|iPad/.test(navigator.platform)?event.metaKey:event.ctrlKey)&&!event.shiftKey&&!event.altKey&&!event.isComposing&&event.key.toLowerCase()==='k'){event.preventDefault();event.stopImmediatePropagation();openCommands();}},true);
