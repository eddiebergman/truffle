'use strict';
let inventory = { candidates: [], directories: [] }, selectedWork = new Set(), pickerItems = [];
let sessionFilter = 'active', selectedSession = null, inventoryLoading = null;
const launchForm = $('#launch-form');
let launchLifetime=0;
function restoreSessionFocus(key) { if (key) for (const node of document.querySelectorAll('#work-list [data-focus-key], #hidden-list [data-focus-key]')) if (node.dataset.focusKey === key) node.focus({preventScroll:true}); }
const needsChatLink = session => session.agent === 'codex' && session.destination === 'desktop' && !session.sessionId && !session.launchNew;
const agentName = value => value === 'claude' ? 'Claude' : 'Codex';
function sessionWorkState(session) {
  if (!session.items.length || session.items.some(x => x.sourceUnavailable)) return 'unknown';
  return session.items.every(x => x.status === 'done' || x.archivedAt) ? 'done' : 'active';
}
function sessionRuntime(session) {
  const exact=typeof session.id==='string'&&session.id?inventory.candidates.filter(candidate=>candidate.recordId===session.id):[];
  const matches=exact.length?exact:attachableSessionCandidate(session)?inventory.candidates.filter(candidate=>(candidate.recordId===undefined||candidate.recordId===null||candidate.recordId==='')&&candidate.agent===session.agent&&attachableSessionCandidate(candidate)&&candidate.sessionId===session.sessionId):[];
  const labels={running:'Running',idle:'Idle',needsInput:'Needs input',closed:'Closed',unknown:'Unknown'};
  if(matches.length){const lifecycle=matches.length===1&&matches[0].agent===session.agent&&Object.hasOwn(labels,matches[0].lifecycle)?matches[0].lifecycle:'unknown';return {label:labels[lifecycle],value:lifecycle};}
  if (!session.sessionId && !session.launchNew) return {label:session.destination==='cmux'?'Linking chat':'Awaiting chat link',value:'unknown'};
  return {label: session.launchNew ? 'Not started' : 'Saved session',value:'unknown'};
}
function loadInventory(fresh=false) {
  if (inventoryLoading) return fresh ? inventoryLoading.then(()=>loadInventory()) : inventoryLoading;
  inventoryLoading=(async()=>{
    try {
      const next=await api('/api/agents');
      if(JSON.stringify(next)!==JSON.stringify(inventory)){
        inventory=next;
        $('#directory-options').replaceChildren(...inventory.directories.map(path=>new Option(path,path)));
        if(state&&!document.querySelector('dialog[open]'))render();
      }
      return next;
    } catch (_) { return null; }
    finally { inventoryLoading=null; }
  })();
  return inventoryLoading;
}
function linkedSessions(item) { return [...(linkedSessionIndex.get(item.id) || [])]; }
let quickStarting = false;
$('#quick-agent').value = ['claude','codex'].includes(prefs.agent) ? prefs.agent : 'claude';
function updateQuickAgent() {
  const agent = agentName($('#quick-agent').value);
  $('#new-note').setAttribute('aria-label', `Start ${agent} session`);
  $('#new-note').title = `Start ${agent} in truffle`;
  $('#summarize-work').title = `Summarize all work with ${agent} in cmux. Checks live sources and suggests actions.`;
}
$('#quick-agent').onchange = () => { updateQuickAgent(); persist(); if (state) render(); };
updateQuickAgent();
let reviewStarting=false;
$('#summarize-work').onclick=async()=>{
 if(reviewStarting)return;reviewStarting=true;
 try{const agent=$('#quick-agent').value;await prepareSavedChat('review:'+agent,'review',{agent},{button:$('#summarize-work')});}catch(error){toast(error.message);}finally{reviewStarting=false;}
};
function folderViewKey(){return source+':'+(scopePrefs[source]||'');}
function itemFolderContexts(item){
  const result=[];
  if(item.source==='github'){const repo=item.url?.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\//)?.[1]||item.project;if(repo)result.push('github:'+repo.toLowerCase());}
  if(item.source==='sentry')result.push('sentry-project:'+item.project);
  if(item.source==='linear'){
    let parent=item.parent,top=null,seen=new Set();while(parent&&!seen.has(parent.id)&&seen.size<8){seen.add(parent.id);top=parent;parent=parent.parent;}
    if(!top&&item.children?.length)top=item;
    if(top)result.push('linear-family:'+top.id);
    if(item.projectId){const workspace=item.url?.split('/')[3]||'';result.push('linear-project:'+workspace+':'+item.projectId);}
  }
  result.push('item:'+item.id);return result;
}
function viewFolderContext(){
  if(!scopePrefs[source])return null;
  const matching=state.items.filter(x=>isSource(x,source)&&scopeKey(x)===scopePrefs[source]);
  const keys=[...new Set(matching.flatMap(itemFolderContexts).filter(x=>x.startsWith(source==='github'?'github:':source==='sentry'?'sentry-project:':'linear-project:')))];
  return keys.length===1?keys[0]:null;
}
function folderHint(item){
  // This is configured metadata for compact rows, not filesystem validation.
  if(item.source==='notes'&&item.cwd)return item.cwd;
  const setting=state.itemSettings?.[item.id];
  if(setting?.explicit&&setting.cwd!==inventory.workRoot)return setting.cwd;
  for(const key of itemFolderContexts(item)){const value=state.folderMappings?.[key];if(value)return value.path;}
  if(item.source==='github'){const repo=scopeKey(item).split('/').pop().toLowerCase();const folder=(inventory.folders||[]).find(x=>x.name.toLowerCase()===repo);if(folder)return folder.path;}
  return setting?.cwd&&setting.cwd!==inventory.workRoot?setting.cwd:null;
}
function renderFolderSelector(){
  if($('#launch-options').open)return;
  const select=$('#launch-folder'),key=folderViewKey(),context=viewFolderContext();
  const item=scopePrefs[source]?state.items.find(x=>isSource(x,source)&&scopeKey(x)===scopePrefs[source]):null;
  const automatic=state.folderMappings?.[context]?.path||(source==='github'&&item?folderHint(item):null);
  const choices=inventory.folders||inventory.directories.map(path=>({path,name:path.split('/').pop()}));
  select.replaceChildren(new Option(automatic?'Auto · '+automatic.split('/').pop():'Automatic',''),...choices.map(x=>new Option(x.name,x.path)));
  select.value=choices.some(x=>x.path===folderPrefs[key])?folderPrefs[key]:'';
  select.title=automatic?'Remembered folder: '+automatic:'Choose a folder for a new session. Issue buttons resolve their saved folder or ask once.';
}
$('#launch-folder').onchange=()=>{folderPrefs[folderViewKey()]=$('#launch-folder').value;persist();};
let pendingFolderLaunch=null;
function showFolderDialog(location,item,payload){
  if(!requestDialogNavigation())return false;pendingFolderLaunch={item,payload};
  $('#folder-reason').textContent=location.title+' · '+location.reason+' ('+location.root+')';
  $('#folder-choice').replaceChildren(new Option('Choose an existing folder…',''),...location.folders.map(x=>new Option(x.name,x.path)),new Option('＋ New folder','__new__'));
  $('#folder-choice').value='';$('#new-folder-label').hidden=true;$('#new-folder-name').required=false;$('#new-folder-name').value=location.suggestedName||'';
  $('#remember-folder').replaceChildren(...location.contexts.map(x=>new Option(x.label,x.key)));
  $('#remember-folder-label').hidden=!location.contexts.length;$('#folder-error').hidden=true;$('#folder-dialog').show();$('#folder-choice').focus();
}
$('#folder-choice').onchange=()=>{const create=$('#folder-choice').value==='__new__';$('#new-folder-label').hidden=!create;$('#new-folder-name').required=create;if(create)$('#new-folder-name').focus();};
$('#folder-form').onsubmit=async event=>{
  event.preventDefault();const submit=event.submitter;submit.disabled=true;$('#folder-error').hidden=true;
  const {item,payload}=pendingFolderLaunch,create=$('#folder-choice').value==='__new__';
  try{await quickStart(item,null,{...payload,chooseFolder:false,folderPath:create?'':$('#folder-choice').value,newFolder:create?$('#new-folder-name').value:'',rememberKey:$('#remember-folder').value});}
  finally{submit.disabled=false;}
};
async function quickStart(item=null,button=null,extra={}){
 if(quickStarting)return;$('#launch-options').open=false;quickStarting=true;
 const page=source,scope=scopePrefs[page]||'',manual=$('#launch-folder').value,folderKey=folderViewKey(),payload={agent:$('#quick-agent').value,...(item?{itemId:item.id}:{}),scopeSource:page,scope};
 if(manual&&(!item||scope&&isSource(item,page)&&scopeKey(item)===scope)){payload.folderPath=manual;const context=viewFolderContext();if(context)payload.rememberKey=context;}
 Object.assign(payload,extra);const kind=payload.workflow==='pr-review'?'pr-review':'quick';delete payload.workflow;
 if(extra.chooseFolder){delete payload.folderPath;delete payload.newFolder;delete payload.rememberKey;}
 try{const result=await prepareSavedChat(kind==='quick'?'quick':'pr-review:'+item.id,kind,payload,{button:button||$('#new-note'),onPrepared:session=>{if(payload.folderPath||payload.newFolder){if(!item){folderPrefs[folderKey]=session.cwd;persist();}inventory.folders||=[];if(!inventory.folders.some(folder=>folder.path===session.cwd))inventory.folders.push({name:session.cwd.split('/').pop(),path:session.cwd});}}});if(result.status==='rejected'&&result.error?.data?.location&&result.isCurrent())showFolderDialog(result.error.data.location,item,payload);}
 catch(error){toast(error.message);}finally{quickStarting=false;}
}
function itemSessionChip(item) {
  const wrap = el('div', 'item-agent-links');
  const links = linkedSessions(item).filter(x=>x.workflow!=='pr-review').sort((a,b)=>(b.lastOpenedAt||b.updatedAt)-(a.lastOpenedAt||a.updatedAt));
  const session = links[0];
  const button = el('button', 'agent-link item-agent-action', session ? '↗' : '＋');
  button.dataset.focusKey = item.id + ':agent';
  button.disabled = quickStarting;
  const description = session ? `Continue ${agentName(session.agent)} for ${item.title}` : `Start ${agentName($('#quick-agent').value)} for ${item.title}`;
  const folder=session?.cwd||folderHint(item),folderLabel=item.source==='notes'&&!session?.cwd?(folder?'Configured folder: '+folder:'Folder resolved when launching'):'Folder: '+(folder||'choose on first launch');
  button.setAttribute('aria-label',description);button.title=description+' · '+folderLabel;
  button.onclick = () => session ? (needsChatLink(session) ? openLaunch(session.agent,null,session) : openSavedSession(session,button)) : quickStart(item,button);
  wrap.append(button);
  if(item.source==='github'&&item.kind==='Pull request'){
    const review=el('button','agent-link item-agent-action pr-review-action','◎');
    const existing=(state.sessions||[]).some(session=>session.workflow==='pr-review'&&session.reviewItemId===item.id&&session.agent===$('#quick-agent').value&&!session.archivedAt);
    const label=`${existing?'Continue':'Run'} review with ${agentName($('#quick-agent').value)} for ${workReference(item)}`;
    review.title=label+(existing?' · Reopen the existing review session':' · Start a dedicated review now');review.setAttribute('aria-label',label);review.dataset.focusKey=item.id+':pr-review';review.disabled=quickStarting;
    review.onclick=()=>quickStart(item,review,{workflow:'pr-review'});wrap.append(review);
  }
  if(links.length>1){const more=el('button','agent-link',`+${links.length-1}`);more.dataset.focusKey=item.id+':more-sessions';more.title='Manage attached sessions';more.setAttribute('aria-label',`Manage ${links.length} sessions for ${item.title}`);more.onclick=()=>openItem(item);wrap.append(more);}
  return wrap;
}
function sessionCard(session) {
  const card=el('article','session-card');
  card.append(el('span','agent-icon '+session.agent,session.agent==='claude'?'✳':'⌘'));
  const title=el('button','session-card-title',session.title);title.title=`${session.title} · ${session.cwd}`;title.dataset.focusKey=session.id+':title';title.onclick=()=>showSession(session);card.append(title);
  const linked=el('span','session-work-summary',session.items.length ? session.items.map(x=>workReference(x)||x.title).join(' · ') : 'Paste a link in the chat to connect work');
  linked.title=session.items.map(x=>x.title).join(' · ');card.append(linked);
  const runtime=sessionRuntime(session),done=sessionWorkState(session)==='done';
  card.append(el('span','runtime-pill '+runtime.value,session.archivedAt?'Archived':done?'Work complete':runtime.label));
  const open=el('button','icon-button session-continue',session.launchNew?'＋':'↗');
  open.dataset.focusKey=session.id+':open';open.dataset.savedOpenRecord=session.id;
  const openLabel=savedSessionOpenState(session.id)?savedSessionOpenAction(savedSessionOpenState(session.id)):needsChatLink(session)?'Attach chat link':session.launchNew?'Start session':'Continue session';
  open.setAttribute('aria-label',`${openLabel}: ${session.title}`);open.title=openLabel;
  open.onclick=()=>needsChatLink(session)?openLaunch(session.agent,null,session):openSavedSession(session,open);
  const manage=el('button','icon-button','⋯');manage.title='Manage session';manage.setAttribute('aria-label',`Manage session: ${session.title}`);manage.dataset.focusKey=session.id+':manage';manage.onclick=()=>showSession(session);
  card.append(open);
  if (!session.archivedAt) card.append(visibilityButton(session,true));
  card.append(manage);return card;
}
function renderBoardSessions() {
  const section=$('#active-sessions');
  const query=$('#search').value.trim().toLowerCase();
  const sessions=(state.sessions||[]).filter(x=>!x.archivedAt&&!x.hidden&&!x.launchNew&&sessionWorkState(x)!=='done'&&[x.title,...x.items.map(i=>i.title+' '+workReference(i))].join(' ').toLowerCase().includes(query));
  section.hidden=source!=='all'||!sessions.length;
  section.replaceChildren();
  if(section.hidden)return;
  sessions.sort((a,b)=>(b.lastOpenedAt||b.updatedAt)-(a.lastOpenedAt||a.updatedAt));
  const heading=el('h2','group-header','Agent sessions');heading.append(el('span','group-count',sessions.length),el('span','group-line'));
  section.append(heading,...sessions.map(sessionCard));
}
function renderSessionView(focusKey=document.activeElement?.dataset.focusKey) {
  const query=$('#search').value.trim().toLowerCase();
  const sessions=(state.sessions||[]).filter(x=>!x.archivedAt && !x.hidden && (sessionFilter==='all'||(sessionFilter==='done'?sessionWorkState(x)==='done':sessionWorkState(x)!=='done')) && [x.title,x.cwd,...x.items.map(i=>i.title+' '+workReference(i))].join(' ').toLowerCase().includes(query));
  sessions.sort((a,b)=>(b.lastOpenedAt||b.updatedAt)-(a.lastOpenedAt||a.updatedAt));
  for(const button of document.querySelectorAll('[data-session-filter]'))button.classList.toggle('selected',button.dataset.sessionFilter===sessionFilter);
  const list=$('#work-list');list.replaceChildren(...sessions.map(sessionCard));
  restoreSessionFocus(focusKey);
  if(!sessions.length){const empty=el('div','empty');empty.append(el('div','empty-icon','⇄'),el('h2','',sessionFilter==='done'?'No completed work here yet.':'Keep the conversation with the work.'),el('p','',sessionFilter==='done'?'Sessions appear here when all their linked work is done.':'Start an agent or attach an existing session. One conversation can cover several tickets, PRs, and projects.'));const start=el('button','button','＋ Start a session');start.onclick=()=>quickStart();empty.append(start);list.append(empty);}
}
function renderArchiveView(items,focusKey=document.activeElement?.dataset.focusKey) {
  const query=$('#search').value.trim().toLowerCase();
  const projects=items.filter(x=>x.archivedAt&&x.title.toLowerCase().includes(query));
  const sessions=(state.sessions||[]).filter(x=>x.archivedAt&&[x.title,x.cwd].join(' ').toLowerCase().includes(query));
  const list=$('#work-list');list.replaceChildren();
  if(projects.length){list.append(el('h2','group-header','Archived work'));projects.forEach(x=>list.append(renderRow(x)));}
  if(sessions.length){list.append(el('h2','group-header','Agent sessions'));sessions.forEach(x=>list.append(sessionCard(x)));}
  restoreSessionFocus(focusKey);
  if(!projects.length&&!sessions.length){const empty=el('div','empty');empty.append(el('h2','','Nothing archived.'),el('p','','Archived projects and sessions stay here. Their agent conversations remain untouched.'));list.append(empty);}
}
function renderItemSessions(item) {
  renderItemFolderHint(item);
  $('#item-folder-change').onclick=()=>quickStart(item,null,{chooseFolder:true});
  const local=item.source==='notes';$('#project-controls').hidden=!local;
  $('#complete-project').textContent=item.status==='done'?'Reopen project':'Mark done';
  $('#archive-project').textContent=item.archivedAt?'Restore project':'Archive project';
  const links=linkedSessions(item);$('#item-session-list').replaceChildren(...links.map(sessionCard));
  if(!links.length)$('#item-session-list').append(el('p','form-hint','No session attached. Start one or connect a conversation you already use.'));
  $('#item-add-session').onclick=()=>quickStart(item,$('#item-add-session')); 
  $('#edit-project').onclick=()=>{ $('#item-dialog').close();openNote(item); };
  $('#complete-project').onclick=()=>changeProjectStatus(item,item.status==='done'?'todo':'done',{close:true});
  const archive=$('#archive-project'),dialog=$('#item-dialog'),id=item.id,restore=!!item.archivedAt,page=source;let archiving=false;
  archive.disabled=false;const archiveAction=async()=>{if(archiving)return;archiving=true;archive.disabled=true;const result=await mutate({action:'archive-project',id,restore});const current=()=>dialog.open&&currentItem?.id===id&&source===page&&archive.onclick===archiveAction;
    if(result.kind==='accepted'&&result.presentation==='ready'&&current())dialog.close();
    if(result.kind==='rejected'&&current()){archiving=false;archive.disabled=false;}toast(localWriteMessage(result,restore?'Project restored.':'Project archived in truffle.'));
  };archive.onclick=archiveAction;
}
let itemFolderPreviewRevision=0;
function renderItemFolderHint(item){
  const revision=++itemFolderPreviewRevision,label=$('#item-folder-path'),dialog=$('#item-dialog'),page=source;
  label.setAttribute('role','status');label.setAttribute('aria-busy',String(item.source==='notes'));
  if(item.source!=='notes'){label.textContent='Folder: '+(folderHint(item)||'choose on first launch');label.title='';return;}
  const configured=folderHint(item);label.textContent='Folder: checking saved project folder…';label.title='Launch checks the folder again before starting.';
  const current=()=>revision===itemFolderPreviewRevision&&dialog.open&&label.isConnected&&currentItem?.id===item.id&&source===page;
  void api('/api/commands/invoke',{name:'projects.folder',args:{id:item.id}}).then(result=>{
    if(!current())return;
    if(result.id!==item.id||!(result.requiresSelection===true&&result.cwd===null||result.requiresSelection===false&&typeof result.cwd==='string'&&result.cwd.length>0))throw new Error('Invalid project folder preview.');
    label.textContent=result.requiresSelection?'Folder: choose when launching':'Folder: '+result.cwd;
  }).catch(()=>{if(current())label.textContent='Folder: could not verify.'+(configured?' Configured folder: '+configured:' Launch will check again.');}).finally(()=>{if(current())label.setAttribute('aria-busy','false');});
}
function setRadio(name,value){launchForm.querySelector(`input[name="${name}"][value="${value}"]`).checked=true;}
function connectionMode(){return launchForm.elements.connection.value;}
function scopeMode(){return launchForm.elements.scope.value;}
function updateLaunchForm() {
  const agent=launchForm.elements.agent.value;
  const attach=connectionMode()==='attach',editing=!!launchForm.elements.id.value;
  launchForm.elements.agent.disabled=editing;
  $('#candidate-picker').disabled=editing&&!!selectedSession?.sessionId;
  const desktop=launchForm.elements.destination.querySelector('option[value="desktop"]');desktop.disabled=agent==='claude';
  if(agent==='claude'&&launchForm.elements.destination.value==='desktop')launchForm.elements.destination.value='embedded';
  $('#attach-fields').hidden=!attach;
  launchForm.elements.sessionId.required=attach&&!editing;
  $('#existing-work-fields').hidden=scopeMode()!=='existing';$('#new-project-hint').hidden=scopeMode()!=='new';
  $('#launch-title').textContent=editing?'Manage session':`${attach?'Attach':'Start'} ${agentName(agent)}`;
  launchForm.querySelector('button[value="save"]').textContent=editing?'Save changes':'Save link';
  $('#launch-submit').textContent=editing?'Save & open ↗':attach?'Attach & open ↗':'Start session ↗';
  $('#connection-mode').hidden=editing;
  $('#scope-mode').hidden=editing;
  launchForm.elements.sessionId.readOnly=editing&&!!selectedSession?.sessionId;
  $('#launch-note').textContent=attach?'Attaching keeps the existing conversation history. Opening it does not send a new message.':launchForm.elements.destination.value==='desktop'?'Codex opens a prefilled composer. Send it, then use the truffle skill or paste the chat link to register the conversation.':'The agent receives a local context packet and instructions to register its conversation with Truffle.';
  populateCandidates();renderWorkPicker();
}
function attachableSessionCandidate(candidate){return typeof candidate.sessionId==='string'&&/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(candidate.sessionId);}
function populateCandidates(){const current=$('#candidate-picker').value;const options=[new Option('Choose an embedded session, or enter an ID below…','')];inventory.candidates.filter(x=>x.agent===launchForm.elements.agent.value&&attachableSessionCandidate(x)).forEach(x=>options.push(new Option(`${x.title} · ${x.lifecycle} · ${x.sessionId.slice(0,8)} · ${x.cwd}`,x.sessionId)));$('#candidate-picker').replaceChildren(...options);$('#candidate-picker').value=current;}
function renderWorkPicker(){const query=$('#work-picker-search').value.toLowerCase();const container=$('#work-picker');container.replaceChildren();let count=0;for(const item of pickerItems){if(![item.title,workReference(item),item.project].join(' ').toLowerCase().includes(query))continue;const label=el('label','work-option');const check=el('input');check.type='checkbox';check.value=item.id;check.checked=selectedWork.has(item.id);check.onchange=()=>{if(check.checked)selectedWork.add(item.id);else selectedWork.delete(item.id);};const words=el('div');words.append(el('span','',item.title),el('small','',`${workReference(item)||sourceNames[item.source]} · ${item.project||'Local project'}${item.status==='done'?' · Done':''}`));label.append(check,words);container.append(label);count++;}if(!count)container.append(el('p','form-hint','No matching work. You can import an issue or PR link below.'));}
function openLaunch(agent='claude',item=null,session=null){
  if(!requestDialogNavigation())return false;launchLifetime++;launchEdit=null;delete $('#launch-dialog').dataset.formLayout;disposeFeatureForm($('#launch-dialog'));
  launchForm.reset();selectedSession=session;$('#launch-error').hidden=true;
  launchForm.elements.id.value=session?.id||'';launchForm.elements.agent.value=session?.agent||agent;
  launchForm.elements.destination.value=session?.destination==='desktop'?'desktop':'embedded';
  launchForm.elements.title.value=session?.title||item?.title||'';
  launchForm.elements.cwd.value=session?.cwd||state.itemSettings?.[item?.id]?.cwd||item?.cwd||'';
  launchForm.elements.sessionId.value=session?.sessionId||'';
  launchForm.elements.instruction.value=session?.instruction||'';
  setRadio('connection',session?'attach':'new');setRadio('scope',session||item?'existing':'new');
  selectedWork=new Set(session?session.items.map(x=>x.id):item?[item.id]:[]);
  const byId=new Map((session?.items||[]).map(x=>[x.id,x]));allItems().filter(x=>!x.archivedAt||selectedWork.has(x.id)).forEach(x=>byId.set(x.id,x));
  pickerItems=[...byId.values()].sort((a,b)=>Number(selectedWork.has(b.id))-Number(selectedWork.has(a.id))||a.title.localeCompare(b.title));
  $('#work-picker-search').value='';updateLaunchForm();$('#launch-dialog').showModal();launchForm.elements.title.focus();
  const lifetime=launchLifetime;if(session)beginManagedLaunch(session.id,lifetime);
  loadInventory().then(()=>{if($('#launch-dialog').open&&launchLifetime===lifetime&&!featureForms.get($('#launch-dialog'))?.pending)populateCandidates();});
}
const pendingSessionResumes=new Set(),savedSessionOpens=new Map();
let savedSessionOpenIntent=0,sessionDetailsLifetime=0;
function savedSessionOpenState(id){return savedSessionOpens.get(id);}
function savedSessionOpenReceipt(receipt,id){
 if(!receipt||receipt.recordId!==id||!['accepted','unknown','preview'].includes(receipt.status)||!['embedded','desktop'].includes(receipt.target)||typeof receipt.localSaved!=='boolean')return null;
 const observed=receipt.terminal?.state==='observed';if(observed&&(receipt.status!=='accepted'||receipt.target!=='embedded'||receipt.localSaved!==true||typeof receipt.terminal.generation!=='string'||!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(receipt.terminal.generation)))return null;
 const expected=observed?'observed':receipt.status==='unknown'?'unconfirmed':receipt.status==='preview'?'not-started':receipt.target==='desktop'?'not-applicable':'acknowledged';if(receipt.terminal?.state!==expected||receipt.terminal.generation!==undefined&&(typeof receipt.terminal.generation!=='string'||!receipt.terminal.generation.length||receipt.terminal.generation.length>200))return null;
 return {recordId:id,status:receipt.status,target:receipt.target,localSaved:receipt.localSaved,terminal:{state:expected,...(receipt.terminal.generation!==undefined?{generation:receipt.terminal.generation}:{})}};
}
function adoptSavedSessionOpen(receipt,id){
 const narrowed=savedSessionOpenReceipt(receipt,id);if(!narrowed)return null;
 let entry=savedSessionOpens.get(id);if(entry?.receipt)return entry;
 if(!entry){if(savedSessionOpens.size>=64){const completed=[...savedSessionOpens].find(([,value])=>value.resolved);if(!completed)return null;savedSessionOpens.delete(completed[0]);}entry={pending:false,checking:false,resolved:false};savedSessionOpens.set(id,entry);}
 entry.receipt=narrowed;state={...state};return entry;
}
function savedSessionOpenAction(entry){return entry?.receipt?.target==='desktop'?'Check open status':'Check terminal';}
function savedSessionOpenMessage(entry){
 if(entry.pending)return 'Opening saved chat…';
 if(entry.receipt?.status==='unknown')return entry.receipt.target==='desktop'?'The desktop open request could not be confirmed. Check open status reads local state; it cannot confirm application consumption. No second open request will be sent.':'The open request could not be confirmed. Check terminal to read its current status; no second open request will be sent.';
 if(entry.receipt?.status==='preview')return 'Preview only. No terminal was started.';
 if(entry.receipt?.terminal?.state==='observed')return 'Existing terminal found. No new open request was sent.'+(entry.resolved?'':' Check terminal to reload its observed state.');
 return (entry.receipt?.target==='desktop'?'Desktop open request acknowledged.':'Chat open request accepted.')+(entry.receipt?.localSaved===false?' Its local metadata could not be saved.':'')+(entry.resolved?'':' '+savedSessionOpenAction(entry)+' to refresh the view; do not repeat the open request.');
}
async function openSavedSession(session,button,{target='saved',isCurrent=()=>true,navigate=true,notify=true}={}){
 const id=session.id;let outcome,entry=savedSessionOpens.get(id);if(entry?.pending||entry?.checking)return {status:'pending',presentation:'pending'};
 if(!entry){if(savedSessionOpens.size>=64){const completed=[...savedSessionOpens].find(([,value])=>value.resolved);if(completed)savedSessionOpens.delete(completed[0]);else{toast('Check existing chat open requests before opening another chat.');return {status:'blocked',presentation:'pending'};}}entry={pending:true,checking:false,receipt:null,resolved:false};savedSessionOpens.set(id,entry);}
 const intent=++savedSessionOpenIntent,page=source,key=typeof selectedAgentKey==='string'?selectedAgentKey:null,details=sessionDetailsLifetime,focus=document.activeElement,query=$('#search')?.value;
 const current=()=>intent===savedSessionOpenIntent&&source===page&&(key===null||selectedAgentKey===key)&&sessionDetailsLifetime===details&&$('#search')?.value===query&&isCurrent();
 const mayNavigate=()=>current()&&![...document.querySelectorAll('dialog[open]')].some(dialog=>!dialog.contains(focus))&&(!document.activeElement||document.activeElement===document.body||document.activeElement===focus||document.activeElement===status&&status?.isConnected||!document.activeElement.isConnected);
 const status=button?.parentElement?el('span','form-hint',entry.receipt?'Checking terminal…':'Opening saved chat…'):null,operation={host:button?.closest('dialog')||button?.parentElement,ui:{message:status},pending:true,isCurrent:current};if(status){status.tabIndex=-1;status.setAttribute('role','status');button.parentElement.append(status);if(document.activeElement===button||button.closest('dialog')?.contains(document.activeElement))holdFormFocus(operation.host,operation);button.dataset.savedOpenRecord=id;}
 pendingSessionResumes.add(id);if(button){button.disabled=true;button.textContent=entry.receipt?'Checking terminal…':'Opening…';}
 try{
  if(!entry.receipt){try{const receipt=await api('/api/commands/invoke',{name:'chats.open',args:{recordId:id,...(target==='embedded'?{target}: {})}});if(!adoptSavedSessionOpen(receipt,id))throw new Error('The open response could not be confirmed.');}
   catch(error){if([400,401,403,404,409].includes(error.status)){savedSessionOpens.delete(id);if(current())showSessionRecovery(state.sessions.find(row=>row.id===id)||session,error.message);return {status:'rejected',presentation:'pending',error:error.message};}entry.receipt={status:'unknown',recordId:id,target:target==='embedded'?'embedded':session.destination==='desktop'?'desktop':'embedded',localSaved:false,terminal:{state:'unconfirmed'}};state={...state};}
  }
  entry.pending=false;entry.checking=true;let snapshot,agents;
  try{[snapshot,agents]=await Promise.all([loadStateSnapshot({attempts:2}),loadInventory(true)]);}catch(_){snapshot=null;agents=null;}
  const latest=state.sessions.find(row=>row.id===id),generation=entry.receipt.terminal?.generation;
  const candidate=agents?.candidates?.find(candidate=>candidate.recordId===id&&candidate.host==='embedded'&&candidate.live===true&&(generation===undefined||candidate.generation===generation));
  const ready=!!snapshot?.accepted&&!!latest&&(entry.receipt.target==='desktop'||entry.receipt.status==='preview'||!!candidate);
  entry.resolved=ready&&entry.receipt.status!=='unknown'&&entry.receipt.localSaved!==false;
  const result=outcome={...entry.receipt,presentation:ready?'ready':'pending',record:latest||null,candidate:candidate||null,navigated:false};
  if(navigate&&ready&&candidate&&mayNavigate()&&typeof selectAgent==='function')try{selectAgent(latest,candidate);result.navigated=true;}catch(_){entry.resolved=false;result.presentation='pending';}
  if(notify)toast(savedSessionOpenMessage(entry));return result;
 }finally{entry.pending=false;entry.checking=false;pendingSessionResumes.delete(id);const held=!!status&&(document.activeElement===status||document.activeElement===document.body&&!status.isConnected);if(button?.isConnected){button.disabled=false;button.textContent=!savedSessionOpens.has(id)?'Open session ↗':savedSessionOpenAction(entry);}if(current())try{render();}catch(_){entry.resolved=false;if(outcome)outcome.presentation='pending';if(notify)toast(savedSessionOpenMessage(entry));}const stillHeld=held&&(document.activeElement===status||document.activeElement===document.body||!document.activeElement);status?.remove();if(stillHeld&&current()&&!document.querySelector('dialog[open]')){const replacement=[...document.querySelectorAll('button')].find(target=>target.dataset.savedOpenRecord===id&&!target.disabled&&!target.hidden&&target.getClientRects().length);replacement?.focus({preventScroll:true});}}

}
let recoveryOriginal=null,recoveryPrepared=null,recoveryLifetime=0,recoveryPending=false,recoveryCreationUnknown=false,recoveryContext=null;
function recoveryIsCurrent(lifetime){return recoveryLifetime===lifetime&&$('#recovery-dialog').open&&source===recoveryContext.page&&(recoveryContext.key===null||selectedAgentKey===recoveryContext.key);}
function presentRecoveryOpen(result,lifetime,focus){
 if(!recoveryIsCurrent(lifetime))return;const dialog=$('#recovery-dialog');
 if(result.presentation==='ready'){if([...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog))return;dialog.close();if(result.candidate&&result.record&&(!document.activeElement||document.activeElement===document.body||document.activeElement===focus))selectAgent(result.record,result.candidate);return;}
 const id=(recoveryPrepared||recoveryOriginal).id;$('#recovery-error').textContent=savedSessionOpenMessage(savedSessionOpenState(id)||{});$('#recovery-error').setAttribute('role','status');$('#recovery-start').textContent=savedSessionOpenAction(savedSessionOpenState(id));if(recoveryPrepared){$('#recovery-retry').hidden=true;$('#recovery-edit').hidden=true;for(const hint of dialog.querySelectorAll('.form-hint'))hint.hidden=true;}
}
function showSessionRecovery(session,message){
  if(!requestDialogNavigation())return false;disposeFeatureForm($('#recovery-dialog'));recoveryLifetime++;recoveryOriginal=session;recoveryPrepared=null;recoveryPending=false;recoveryCreationUnknown=false;recoveryContext={page:source,key:typeof selectedAgentKey==='string'?selectedAgentKey:null};$('#recovery-retry').hidden=false;$('#recovery-edit').hidden=false;for(const hint of $('#recovery-dialog').querySelectorAll('.form-hint'))hint.hidden=false;
  $('#recovery-error').textContent=message;
  $('#recovery-title').textContent=session.title;
  $('#recovery-folder').value=session.cwd;$('#recovery-folder').disabled=false;
  $('#recovery-start').textContent='Start new with context ↗';for(const button of $('#recovery-dialog').querySelectorAll('button'))button.disabled=false;
  $('#recovery-dialog').showModal();
}
$('#recovery-retry').onclick=async()=>{
 if(recoveryPending)return;const lifetime=recoveryLifetime,record=recoveryPrepared||recoveryOriginal,button=$('#recovery-retry'),dialog=$('#recovery-dialog'),current=()=>recoveryIsCurrent(lifetime);recoveryPending=true;const operation={host:dialog,ui:{message:$('#recovery-error')},pending:true,isCurrent:current};operation.ui.message.tabIndex=-1;holdFormFocus(dialog,operation);
 try{const result=await openSavedSession(record,button,{isCurrent:current,navigate:false});presentRecoveryOpen(result,lifetime,operation.ui.message);}
 finally{if(recoveryLifetime===lifetime){recoveryPending=false;restoreFormFocus(operation,button);}}
};
$('#recovery-edit').onclick=()=>openLaunch(recoveryOriginal.agent,null,recoveryOriginal);
$('#recovery-dialog').addEventListener('cancel',event=>{if(recoveryPending)event.preventDefault();});
$('#recovery-form').onsubmit=async event=>{
 event.preventDefault();if(recoveryPending||!$('#recovery-dialog').open)return;const lifetime=recoveryLifetime,original=recoveryOriginal,cwd=$('#recovery-folder').value;recoveryPending=true;
 try{await prepareSavedChat('recover:'+original.id,'recover',{id:original.id,cwd},{host:$('#recovery-form'),isCurrent:()=>recoveryLifetime===lifetime,onPrepared:record=>{recoveryPrepared={id:record.id};}});}catch(error){if(recoveryLifetime===lifetime)$('#recovery-error').textContent=error.message;}finally{if(recoveryLifetime===lifetime)recoveryPending=false;}
};
async function showSession(session){
  if(!requestDialogNavigation())return false;sessionDetailsLifetime++;
  selectedSession=session;
  $('#session-agent').textContent=`${agentName(session.agent)} · ${session.destination==='desktop'?'Desktop app':'truffle terminal'} · ${sessionRuntime(session).label}`;
  $('#session-title').textContent=session.title;$('#session-path').textContent=session.cwd;
  const list=$('#session-work-list');list.replaceChildren();
  for(const item of session.items){const row=el('div','linked-work');const title=el(item.url?'a':'button','linked-work-title',`${item.reference?workReference(item)+' · ':''}${item.title}`);if(item.url){title.href=item.url;title.target='_blank';title.rel='noopener noreferrer';}else{title.onclick=()=>{const current=allItems().find(x=>x.id===item.id);if(current){$('#session-dialog').close();openItem(current);}else toast('This project is no longer in the active workspace.');};}row.append(title,el('span','status-pill '+item.status,workStatusLabel(item)));list.append(row);}
  $('#session-context').textContent='Loading context…';
  $('#session-hint').textContent=session.archivedAt?'Archived only in truffle. The conversation is still available.':!session.sessionId?'The agent received instructions to register automatically. If registration is pending, open its terminal; you can also attach its exact UUID in Edit links & context.':'Opening a saved conversation does not send a message. Archive it here when you no longer need it on your dashboard.';
  $('#session-archive').textContent=session.archivedAt?'Restore in truffle':'Archive in truffle';
  $('#session-open').textContent=savedSessionOpenState(session.id)?savedSessionOpenAction(savedSessionOpenState(session.id)):needsChatLink(session)?'Attach chat link':session.launchNew?'Start session ↗':'Open session ↗';
  renderCheckpoint(session);
  renderSessionArtifacts(session);void loadArtifactLibrary();
  $('#session-dialog').showModal();
  try{const data=await api('/api/session/context',{id:session.id});if(selectedSession?.id===session.id)$('#session-context').textContent=data.context;}catch(error){$('#session-context').textContent=error.message;}
}
$('#project-only').onclick=()=>openNote();
for(const radio of launchForm.querySelectorAll('input[type="radio"]'))radio.onchange=updateLaunchForm;
for(const select of [launchForm.elements.agent,launchForm.elements.destination])select.onchange=updateLaunchForm;
$('#work-picker-search').oninput=renderWorkPicker;
$('#candidate-picker').onchange=()=>{if(launchForm.elements.id.value&&(!launchEdit?.isCurrent()||!featureFormMutable(launchEdit)||launchEdit.loading||!launchEdit.baseline||launchEdit.baseline.chat.sessionId))return;const candidate=inventory.candidates.find(x=>x.agent===launchForm.elements.agent.value&&attachableSessionCandidate(x)&&x.sessionId===$('#candidate-picker').value);if(!candidate)return;launchForm.elements.sessionId.value=candidate.sessionId;launchForm.elements.cwd.value=candidate.cwd;if(!launchForm.elements.title.value)launchForm.elements.title.value=candidate.title;};
const launchEditableFields=['title','cwd','instruction','destination','sessionId','itemIds'];
let launchEdit=null;
function managedLaunchCurrent(current){return launchEdit===current&&launchLifetime===current.lifetime&&current.context.current();}
function enforceManagedIdentity(current){
 const locked=current.loading||current.pending||current.refreshing||!current.baseline||['accepted','unknown'].includes(current.result?.kind),attached=!!current.baseline?.chat.sessionId;
 launchForm.elements.agent.disabled=true;launchForm.elements.sessionId.readOnly=attached;$('#candidate-picker').disabled=locked||attached;
}
function revealManagedRecovery(current){
 const active=document.activeElement;if(current.isCurrent()&&[current.reload,current.ui.refresh,current.ui.inspect,current.openResult].includes(active)&&active?.isConnected&&!active.hidden&&!active.disabled)active.scrollIntoView({block:'nearest',inline:'nearest'});
}
function paintManagedLaunch(current){
 if(!current.isCurrent())return;
 if(current.result)presentFeatureFormResult(current);else freezeFeatureForm(current,current.loading||!current.baseline);
 current.reload.hidden=!!current.result&&['accepted','unknown'].includes(current.result.kind)||!current.conflicted&&!!current.baseline;
 current.reload.disabled=current.pending||current.refreshing||current.loading;
 if(current.conflicted){for(const button of launchForm.querySelectorAll('button[type="submit"]'))button.disabled=true;restoreFormFocus(current,current.reload);}
 enforceManagedIdentity(current);revealManagedRecovery(current);
}
async function readManagedLaunch(current){
 if(!current.isCurrent()||current.pending||current.refreshing||current.loading||['accepted','unknown'].includes(current.result?.kind))return;
 current.loading=true;current.baseline=null;current.result=null;holdFormFocus(launchForm,current);freezeFeatureForm(current,true);current.reload.disabled=true;current.ui.message.textContent='Loading current session settings…';
 try{const value=await api('/api/commands/invoke',{name:'chats.metadata.read',args:{recordId:current.target}});if(!current.isCurrent())return;
  if(value?.recordId!==current.target||typeof value.redacted!=='boolean'||!['embedded','desktop','cmux'].includes(value.chat?.destination)||!['claude','codex'].includes(value.chat?.agent)||!launchEditableFields.every(field=>/^[a-f0-9]{64}$/i.test(value.expected?.[field]||''))||!Array.isArray(value.chat.itemIds)||value.chat.itemIds.length>100||value.chat.itemIds.some(id=>typeof id!=='string')||!['title','cwd','instruction','destination'].every(field=>typeof value.chat[field]==='string')||value.chat.sessionId!==null&&typeof value.chat.sessionId!=='string')throw new Error('Current session settings could not be read. Reload before saving.');
  const chat=value.chat;current.baseline={chat:{...chat,itemIds:[...chat.itemIds]},expected:{...value.expected}};current.conflicted=false;
  selectedSession={...selectedSession,id:current.target,agent:chat.agent,sessionId:chat.sessionId};launchForm.elements.id.value=current.target;for(const field of ['agent','title','cwd','instruction','sessionId'])launchForm.elements[field].value=chat[field]||'';launchForm.elements.destination.value=chat.destination==='desktop'?'desktop':'embedded';launchForm.elements.urls.value='';
  current.display={title:chat.title,cwd:chat.cwd,instruction:chat.instruction,destination:launchForm.elements.destination.value,sessionId:chat.sessionId||'',itemIds:[...chat.itemIds].sort()};selectedWork=new Set(chat.itemIds);
  const items=new Map((selectedSession.items||[]).map(item=>[item.id,item]));allItems().forEach(item=>items.set(item.id,item));for(const id of chat.itemIds)if(!items.has(id))items.set(id,{id,title:'Saved work: '+id,source:'notes',reference:id,status:'todo'});pickerItems=[...items.values()];updateLaunchForm();freezeFeatureForm(current,false);enforceManagedIdentity(current);current.ui.message.textContent=value.redacted?'Some saved text is hidden. Unchanged fields will be preserved.':'';restoreFormFocus(current,launchForm.elements.title);
 }catch(error){if(current.isCurrent()){current.conflicted=true;current.ui.message.textContent=error.message;freezeFeatureForm(current,true);restoreFormFocus(current,current.reload);}}
 finally{current.loading=false;if(current.isCurrent())paintManagedLaunch(current);}
}
function beginManagedLaunch(id,lifetime){
 $('#launch-dialog').dataset.formLayout='bounded';
 const dialog=$('#launch-dialog'),context=capturePreparationContext(()=>launchLifetime===lifetime),current=createFeatureForm(dialog,launchForm,id);launchEdit=current;current.lifetime=lifetime;current.context=context;current.baseline=null;current.loading=false;current.conflicted=false;current.targetCurrent=()=>managedLaunchCurrent(current);current.acceptedText='Session changes saved.';
 const reload=featureButton('Reload latest (replaces draft)',()=>readManagedLaunch(current),'ui-control');reload.dataset.variant='secondary';reload.dataset.featureRecovery='true';reload.title='Replace this draft with the latest saved session settings.';current.reload=reload;current.ui.refresh.parentElement.append(reload);void readManagedLaunch(current);return current;
}
async function saveExistingLaunch(values,{lifetime,open,context}){
 const dialog=$('#launch-dialog'),current=launchEdit,id=values.id;
 if(!current||current.target!==id||current.lifetime!==lifetime||!featureFormMutable(current)||current.loading||current.conflicted||!current.baseline)return;
 const patch={},expected={},urls=(values.urls||'').split(/\n/).map(value=>value.trim()).filter(Boolean),next={title:values.title,cwd:values.cwd,instruction:values.instruction,destination:values.destination,sessionId:values.sessionId.trim(),itemIds:[...selectedWork].sort()};
 for(const field of launchEditableFields)if(JSON.stringify(next[field])!==JSON.stringify(current.display[field])||field==='itemIds'&&urls.length){patch[field]=next[field];expected[field]=current.baseline.expected[field];}
 const record=()=>{const saved=state.sessions.find(saved=>saved.id===id);if(!saved)throw new Error('The saved session is not in the loaded view. Refresh or inspect History; do not repeat the save.');return saved;};
 const finish=()=>{current.context.closed.add(dialog);dialog.close();};
 const openSaved=async()=>{
  if(!current.isCurrent()||current.pending||current.refreshing||current.result?.kind!=='accepted'||current.result.presentation!=='ready')return;
  let saved;try{saved=record();}catch(error){current.result={kind:'accepted',presentation:'pending',error:error.message};openButton.hidden=true;presentFeatureFormResult(current);return;}
  const active=document.activeElement,navigate=(!active||active===document.body||launchForm.contains(active))&&!active?.closest('#app-chat-panel,.xterm');current.pending=true;holdFormFocus(launchForm,current);freezeFeatureForm(current,true);finish();const returnFocus=document.activeElement;
  try{const outcome=await openSavedSession(saved,null,{isCurrent:current.context.current,navigate});if(navigate&&outcome?.navigated===true&&outcome.presentation==='ready'&&outcome.record&&outcome.candidate)focusPreparedTerminal(outcome.record,outcome.candidate,returnFocus);}catch(_){toast('Session changes saved. The open outcome could not be confirmed. Inspect this saved chat before another open request.');}
 };
 const openButton=featureButton('Open saved chat',()=>openSaved(),'ui-control');openButton.dataset.variant='primary';openButton.dataset.featureRecovery='true';openButton.hidden=true;current.openResult?.remove();current.openResult=openButton;current.ui.refresh.parentElement.append(openButton);
 current.afterReady=()=>{if(!current.isCurrent())return;record();current.result={kind:'accepted',presentation:'ready'};if(open){openButton.hidden=false;current.ui.message.textContent='Session changes saved. Choose Open saved chat to open it.';}else finish();};
 current.acceptedText=Object.keys(patch).length?'Session changes saved.':'Session settings unchanged.';current.pending=true;holdFormFocus(launchForm,current);freezeFeatureForm(current,true);current.reload.hidden=true;current.ui.message.textContent='Saving session changes…';let failureStatus=null;
 const result=await presentLocalWrite(async()=>{if(!Object.keys(patch).length)return;try{const receipt=await api('/api/commands/invoke',{name:'chats.metadata.update',args:{recordId:id,patch,expected,...(urls.length?{urls}: {})}});if(receipt?.saved!==true||receipt.recordId!==id)throw new Error('The session save response could not be confirmed.');}catch(error){failureStatus=error.status;throw error;}});current.pending=false;current.result=result;
 if(!current.isCurrent()){if(featureForms.get(dialog)===current&&dialog.open)freezeFeatureForm(current,true);toast(localWriteMessage(result,current.acceptedText));return;}
 if(result.kind==='accepted'&&result.presentation==='ready'){try{record();if(open)await openSaved();else{finish();toast(current.acceptedText);}return;}catch(error){current.result={kind:'accepted',presentation:'pending',error:error.message};}}
 if(result.kind==='rejected'&&failureStatus===409){current.conflicted=true;current.ui.message.textContent=result.error;freezeFeatureForm(current,false);for(const button of launchForm.querySelectorAll('button[type="submit"]'))button.disabled=true;current.reload.hidden=false;current.reload.disabled=false;enforceManagedIdentity(current);restoreFormFocus(current,current.reload);revealManagedRecovery(current);return;}
 if(current.isCurrent())paintManagedLaunch(current);
}
launchForm.onsubmit=async event=>{
 event.preventDefault();if(!$('#launch-dialog').open)return;const lifetime=launchLifetime,values=Object.fromEntries(new FormData(launchForm));values.agent=launchForm.elements.agent.value;const context=capturePreparationContext(()=>launchLifetime===lifetime);
 try{
  if(values.connection==='attach'&&!values.sessionId&&!values.id)throw new Error('Choose a saved session or enter its exact UUID.');
  if(values.scope==='existing'&&!values.id&&!selectedWork.size&&!values.urls.trim())throw new Error('Select at least one item, or paste an issue or PR link.');
  const payload={agent:values.agent,destination:values.destination,title:values.title,cwd:values.cwd,instruction:values.instruction,itemIds:values.scope==='existing'?[...selectedWork]:[],projectName:values.scope==='new'?values.title:'',urls:values.scope==='existing'?values.urls.split(/\n/).map(value=>value.trim()).filter(Boolean):[]};
  if(values.id){await saveExistingLaunch(values,{lifetime,open:event.submitter?.value==='open',context});return;}
  const kind=values.connection==='attach'?'attach':'new';const args=kind==='attach'?{agent:values.agent,sessionId:values.sessionId,title:values.title,cwd:values.cwd,destination:values.destination,itemIds:payload.itemIds}:payload;
  await prepareSavedChat('launch:'+kind,kind,args,{host:launchForm,open:event.submitter?.value==='open',isCurrent:()=>launchLifetime===lifetime});
 }catch(error){if(context.current()){$('#launch-error').textContent=error.message;$('#launch-error').hidden=false;}}
};
$('#session-edit').onclick=()=>{const session=selectedSession;$('#session-dialog').close();openLaunch(session.agent,null,session);};
$('#session-open').onclick=()=>needsChatLink(selectedSession)?openLaunch(selectedSession.agent,null,selectedSession):openSavedSession(selectedSession,$('#session-open'));
$('#session-archive').onclick=async()=>{const target=selectedSession,archived=!target.archivedAt,dialog=$('#session-dialog');if(await mutateChatMetadata('chats.archive',{id:target.id,archived})){if(selectedSession===target&&dialog.open)dialog.close();toast(archived?'Archived in truffle. The agent conversation stays available.':'Session restored.');}};
for(const button of document.querySelectorAll('[data-session-filter]'))button.onclick=()=>{sessionFilter=button.dataset.sessionFilter;render();};
setInterval(()=>{if(!document.hidden&&!document.querySelector('dialog[open]'))loadInventory();},5000);

let sessionHashHandled = false;
function handleSessionHash() {
  if (sessionHashHandled || !state) return;
  const hash = location.hash.match(/^#session=([0-9a-f-]{36})$/i);
  if (!hash) return;
  sessionHashHandled = true;
  const record = state.sessions.find(x => x.id === hash[1]);
  if (record) showSession(record);
  else toast('That session association is no longer available.');
}
window.addEventListener('hashchange', () => { sessionHashHandled = false; handleSessionHash(); });

$('#session-correct').onclick = async () => {
  const old = selectedSession;
  if (!old.archivedAt && !(await mutateChatMetadata('chats.archive',{id:old.id,archived:true}))) return;
  openLaunch(old.agent, null, null);
  launchForm.elements.title.value=old.title;
  launchForm.elements.cwd.value=old.cwd;
  setRadio('connection','attach');setRadio('scope','existing');
  selectedWork=new Set(old.items.map(x=>x.id));
  const current=new Map(old.items.map(x=>[x.id,x]));allItems().forEach(x=>current.set(x.id,x));pickerItems=[...current.values()];
  updateLaunchForm();
};

// Inline new-session prompt. Plus remains an immediate launch with saved settings.
let locationChoices=[],locationIndex=0;
function renderLocationChoices(){
  const query=$('#location-query').value,folders=inventory.folders||(inventory.directories||[]).map(path=>({path,name:path.split('/').pop()}));
  locationChoices=[{name:'Automatic',path:''},...folders].map(folder=>({...folder,score:query?Math.max(fuzzyScore(query,folder.name)+20,fuzzyScore(query,folder.path)):0})).filter(folder=>Number.isFinite(folder.score)).sort((a,b)=>b.score-a.score);
  locationIndex=Math.min(locationIndex,Math.max(0,locationChoices.length-1));
  const list=$('#location-results');list.replaceChildren();
  locationChoices.forEach((folder,index)=>{const button=el('button','location-choice'+(index===locationIndex?' selected':''),folder.name);button.type='button';button.id='location-choice-'+index;button.title=folder.path||'Use saved project mapping';button.setAttribute('role','option');button.setAttribute('aria-selected',String(index===locationIndex));button.onclick=()=>launchAtLocation(folder);list.append(button);});
  if(!locationChoices.length)list.append(el('p','form-hint','No matching folder'));
  $('#location-query').setAttribute('aria-activedescendant',locationChoices.length?'location-choice-'+locationIndex:'');
  list.querySelector('.selected')?.scrollIntoView({block:'nearest'});
}
function openLocationPrompt(){
  const record=typeof selectedAgentKey==='string'?(state.sessions||[]).find(session=>'record:'+session.id===selectedAgentKey||'embedded:'+session.id===selectedAgentKey):null;
  if(record)$('#quick-agent').value=record.agent;
  renderFolderSelector();$('#location-prompt').hidden=false;$('#launch-options').open=true;
  $('#location-query').value='';locationIndex=0;renderLocationChoices();$('#location-query').focus();
}
function launchAtLocation(folder){
  $('#location-prompt').hidden=true;$('#launch-options').open=false;
  return quickStart(null,null,folder.path?{folderPath:folder.path}:{});
}
$('#location-query').oninput=()=>{locationIndex=0;renderLocationChoices();};
$('#location-query').onkeydown=event=>{
  if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();locationIndex=Math.max(0,Math.min(locationChoices.length-1,locationIndex+(event.key==='ArrowDown'?1:-1)));renderLocationChoices();}
  if(event.key==='Escape'){event.preventDefault();$('#launch-options').open=false;$('#location-prompt').hidden=true;$('#new-note').focus();}
};
$('#location-prompt').onsubmit=event=>{event.preventDefault();if(locationChoices[locationIndex])void launchAtLocation(locationChoices[locationIndex]);};
$('#launch-options').addEventListener('toggle',()=>{if(!$('#launch-options').open)$('#location-prompt').hidden=true;});
