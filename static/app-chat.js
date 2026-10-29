'use strict';
function appChatCommand(value){
  if(value?.type==='settings-proposal'){
    if(Object.keys(value).some(key=>!['type','revision','contentHash','patch'].includes(key))||!Number.isSafeInteger(value.revision)||value.revision<0||typeof value.contentHash!=='string'||!/^[a-f0-9]{64}$/.test(value.contentHash)||!value.patch||typeof value.patch!=='object'||Array.isArray(value.patch)||new TextEncoder().encode(JSON.stringify(value.patch)).length>65536)throw new Error('The assistant returned an invalid settings proposal.');
    return {type:'settings-proposal',revision:value.revision,contentHash:value.contentHash,patch:structuredClone(value.patch)};
  }
  if(value?.type==='navigate'){
    const destinations=['skills','artifacts','logs','settings','sharing-inbox','setup','chat-recovery'],categories=['appearance','review','keyboard','agents','actions','connections','sharing','data','help'];
    if(!destinations.includes(value.destination)||Object.keys(value).some(key=>!['type','destination','category'].includes(key))||(Object.hasOwn(value,'category')&&(value.destination!=='settings'||!categories.includes(value.category))))throw new Error('The assistant returned an unsupported view.');
    return {type:'navigate',destination:value.destination,...(Object.hasOwn(value,'category')?{category:value.category}:{})};
  }

  if(value?.type==='saved-view'&&typeof value.id==='string'&&/^[A-Za-z0-9_-]{1,80}$/.test(value.id)&&typeof value.name==='string'&&value.name.length<=120)return {type:'saved-view',id:value.id,name:value.name};
  if(value?.type==='chat-results'&&typeof value.query==='string'&&value.query.length<=500&&Array.isArray(value.chats)&&value.chats.length<=40&&value.chats.every(c=>typeof c.id==='string'&&c.id.length<=160&&typeof c.title==='string'&&typeof c.cwd==='string')&&typeof value.recommendedId==='string'&&(!value.recommendedId||value.chats.some(c=>c.id===value.recommendedId))&&typeof value.reason==='string'&&value.reason.length<=300)return {type:'chat-results',query:value.query,chats:value.chats,total:value.total,recommendedId:value.recommendedId,reason:value.reason};
  if(value?.type==='open-chat'&&typeof value.id==='string'&&value.id.length>0&&value.id.length<=160){const command={type:'open-chat',id:value.id};if(value.results){if(value.results.type!=='chat-results')throw new Error('The assistant returned an unsupported view.');command.results=appChatCommand(value.results);}return command;}
  if(value?.type==='issue-draft'&&typeof value.title==='string'&&value.title.length>0&&value.title.length<=200&&typeof value.body==='string'&&value.body.length<=4000)return {type:'issue-draft',title:value.title,body:value.body};
  if(value?.type==='display-items'&&typeof value.title==='string'&&value.title.length<=120&&Array.isArray(value.items)&&value.items.length>0&&value.items.length<=40&&value.items.every(row=>['chat','work'].includes(row.type)&&typeof row.id==='string'&&row.id.length<=160&&row.item?.id===row.id&&typeof row.item.title==='string')&&(!value.recommended||value.items.some(row=>row.type===value.recommended.type&&row.id===value.recommended.id))&&typeof value.reason==='string'&&value.reason.length<=300)return {type:'display-items',title:value.title,items:value.items,recommended:value.recommended,reason:value.reason};
  const pages=['all','linear','github','sentry','notes','agents','sessions'];
  const statuses=['all','active','todo','in-progress','blocked','needs-review','ready-to-merge','draft','done'];
  if(!value||value.type!=='show'||!pages.includes(value.source)||typeof value.query!=='string'||value.query.length>500||!statuses.includes(value.status))throw new Error('The assistant returned an unsupported view.');
  return {type:'show',source:value.source,query:value.query,status:value.status};
}
function appChatOpenSettingsProposal(value){
  const command=appChatCommand(value);
  if(command.type!=='settings-proposal')throw new Error('The assistant returned an unsupported settings proposal.');
  if(typeof settingsDirty!=='undefined'&&settingsDirty)throw new Error('Save or discard your Settings changes before reviewing this proposal.');
  if(document.querySelector('dialog[open]'))throw new Error('Close the open dialog before reviewing this proposal.');
  const dialog=el('dialog','settings-proposal-dialog');dialog.setAttribute('aria-label','Review settings proposal');
  const heading=el('header','settings-proposal-heading'),close=el('button','ui-control','Close');close.type='button';close.dataset.variant='secondary';close.setAttribute('aria-label','Close settings proposal');close.onclick=()=>dialog.close();
  heading.append(el('h2','','Review settings changes'),close);const content=el('div','settings-proposal-content');dialog.append(heading,content);document.body.append(dialog);
  const preview=renderSettingsProfilePreview(content,{initialProfile:{format:'truffle-settings',version:1,values:command.patch},expectedRevision:command.revision,expectedContentHash:command.contentHash});
  dialog.addEventListener('close',()=>{preview.disposeProfileImport();dialog.remove();},{once:true});dialog.showModal();close.focus();return dialog;
}
async function appChatRefreshSavedViews(){
 await loadSavedViews();
 if(savedViewsError)throw new Error('Saved views could not refresh. '+savedViewsError);
}
async function appChatNavigate(value){
  const command=appChatCommand(value);if(command.type!=='navigate')throw new Error('The assistant returned an unsupported destination.');
  switch(command.destination){
    case 'skills':case 'artifacts':case 'logs':
      if(typeof savedViewCapturePresentation==='function')savedViewCapturePresentation(source);
      source=command.destination;persist();render();return;
    case 'setup':{
      const host=$('#app-chat-setup');
      if(!host||$('#app-chat-panel')?.hidden)throw new Error('Open App chat, then choose Show results to resume setup.');
      if(host.hidden){
        if(typeof openOnboarding!=='function')throw new Error('Setup is unavailable. Reopen Truffle and try again.');
        await openOnboarding({host});
      }
      if(host.hidden)throw new Error('Setup could not open. Try again.');
      return;
    }
    case 'chat-recovery':
      if(typeof openChatRecovery!=='function'||openChatRecovery()!==true)throw new Error('Chat recovery could not open. Close the current dialog, then try again.');
      return;
    case 'settings':
      if(await openPreferences({category:command.category})!==true)throw new Error('Settings could not open. Close the current dialog or finish saving, then try again.');
      return;
    case 'sharing-inbox':{
      const menu=$('#sharing-menu');if(!menu)throw new Error('The sharing inbox is unavailable.');
      if(!menu.open)await new Promise(resolve=>{menu.addEventListener('toggle',resolve,{once:true});menu.open=true;});
      if(!menu.open)throw new Error('The sharing menu was closed before navigation finished.');
      const inbox=[...menu.querySelectorAll('.sharing-tabs button')].find(button=>button.textContent==='Inbox');
      if(!inbox)throw new Error('The sharing inbox is unavailable.');
      if(inbox.getAttribute('aria-pressed')!=='true')inbox.click();return;
    }
  }
}
function appChatAssistant(value){
  const labels={claude:'Claude',codex:'Codex'};return typeof value?.id==='string'&&Object.hasOwn(labels,value.id)?{id:value.id,label:labels[value.id]}:null;
}
function appChatFailureMessage(job){
  const assistant=appChatAssistant(job?.assistant),message=typeof job?.error==='string'&&job.error?job.error:'App chat could not finish.';
  return (assistant&&!message.startsWith(assistant.label)?assistant.label+' · ':'')+message+(/message is saved/i.test(message)?'':' Your message is saved in this chat.');
}
function appChatDiagnosticRows(job){
  const diagnostics=job?.diagnostics&&typeof job.diagnostics==='object'&&!Array.isArray(job.diagnostics)?job.diagnostics:{},rows=[];
  const requestId=[diagnostics.requestId,job?.id].find(value=>typeof value==='string'&&/^[A-Za-z0-9_-]{1,160}$/.test(value));
  const assistant=appChatAssistant(job?.assistant);if(assistant)rows.push(['Assistant',assistant.label]);
  if(requestId)rows.push(['Request ID',requestId]);
  const enums={stage:['launch','process','cli-envelope','assistant-response','complete'],code:['spawn_failed','process_failed','process_wait_failed','timed_out','cancelled','oversized_output','invalid_utf8','invalid_cli_json','invalid_codex_jsonl','codex_turn_failed','unsupported_codex_item','unsupported_codex_event','incomplete_codex_turn','invalid_codex_wrapper','cli_reported_error','missing_result','invalid_assistant_json','invalid_assistant_shape','tool_limit','failed'],stdoutShape:['empty','object','array','string','number','boolean','null','json-lines','non-json'],responseShape:['empty','object','array','string','number','boolean','null','json-lines','non-json'],jsonErrorCategory:['io','syntax','data','eof']},labels={stage:'Stage',code:'Code',stdoutShape:'CLI output shape',responseShape:'Response shape',jsonErrorCategory:'JSON error category'};
  for(const [field,allowed]of Object.entries(enums))if(allowed.includes(diagnostics[field]))rows.push([labels[field],diagnostics[field]]);
  if(Number.isSafeInteger(diagnostics.exitCode))rows.push(['Exit code',String(diagnostics.exitCode)]);
  if(Number.isFinite(diagnostics.durationMs)&&diagnostics.durationMs>=0&&diagnostics.durationMs<=86400000)rows.push(['Duration',(diagnostics.durationMs/1000).toFixed(2)+' s']);
  for(const [field,label]of [['stdoutBytes','CLI output bytes'],['stderrBytes','CLI error bytes'],['jsonErrorLine','JSON error line'],['jsonErrorColumn','JSON error column']])if(Number.isSafeInteger(diagnostics[field])&&diagnostics[field]>=0)rows.push([label,String(diagnostics[field])]);
  return rows;
}
function appChatFailureActions(job,onOpenLogs){
  const root=el('div','app-chat-failure'),actions=el('div','ui-actiongroup app-chat-failure-actions');actions.setAttribute('role','group');actions.setAttribute('aria-label','App chat failure actions');
  const toggle=el('button','ui-control','View details');toggle.type='button';toggle.dataset.variant='secondary';toggle.setAttribute('aria-expanded','false');toggle.setAttribute('aria-controls','app-chat-failure-details');
  const details=el('div','app-chat-diagnostics');details.id='app-chat-failure-details';details.hidden=true;
  if(!job?.diagnostics)details.append(el('p','form-hint','No diagnostic metadata was retained for this request.'));
  const fields=el('dl');for(const [label,value]of appChatDiagnosticRows(job))fields.append(el('dt','',label),el('dd','',value));details.append(fields);
  toggle.onclick=()=>{details.hidden=!details.hidden;toggle.setAttribute('aria-expanded',String(!details.hidden));toggle.textContent=details.hidden?'View details':'Hide details';};
  const logs=el('button','ui-control','Open logs');logs.type='button';logs.dataset.variant='secondary';logs.onclick=onOpenLogs;actions.append(toggle,logs);root.append(actions,details);return root;
}
async function appChatOpenFailureLogs(job){
  if(typeof settingsDirty!=='undefined'&&settingsDirty)throw new Error('Save or discard your Settings changes before opening logs.');
  if(document.querySelector('dialog[open]'))throw new Error('Close the open dialog before opening logs.');
  await appChatNavigate({type:'navigate',destination:'logs'});
  if(typeof logFilters!=='undefined'){logFilters.source='';logFilters.outcome='failure';}
  const request=appChatDiagnosticRows(job).find(([label])=>label==='Request ID');$('#search').value=request?request[1]:'app-chat';persist();render();
}
if(typeof module!=='undefined')module.exports={appChatAssistant,appChatFailureMessage,appChatCommand,appChatNavigate,appChatDiagnosticRows,appChatFailureActions,appChatOpenFailureLogs,appChatOpenSettingsProposal,appChatRefreshSavedViews};
if(typeof document!=='undefined')(()=>{
  const icon='<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M4 4h16v12H9l-5 4V4Z"/></svg>';
  const button=el('button','app-chat-launcher');button.id='app-chat-toggle';button.innerHTML=icon;button.title='App chat';button.setAttribute('aria-label','App chat');button.setAttribute('aria-expanded','false');button.setAttribute('aria-controls','app-chat-panel');
  document.body.append(button);
  const panel=el('section','app-chat-panel');panel.id='app-chat-panel';panel.hidden=true;panel.setAttribute('aria-label','App chat');
  const heading=el('div','app-chat-heading'),name=el('strong',null,'App chat'),badge=el('span','app-chat-provider','Checking…'),close=el('button','text-button','×');close.title='Close App chat';close.setAttribute('aria-label','Close App chat');const activity=el('span','app-chat-activity','Working…');activity.setAttribute('role','status');activity.hidden=true;const fresh=el('button','text-button app-chat-new','+');fresh.title='New chat';fresh.setAttribute('aria-label','New chat');heading.append(name,badge,activity,fresh,close);
  const context=el('div','app-chat-context'),messages=el('div','app-chat-messages');messages.setAttribute('role','log');messages.setAttribute('aria-label','App chat messages');
  const conversation=el('div','app-chat-conversation'),setup=el('section','app-chat-setup');setup.id='app-chat-setup';setup.hidden=true;setup.setAttribute('aria-label','Set up Truffle');conversation.append(messages,setup);
  const error=el('p','app-chat-error');error.setAttribute('role','status');error.hidden=true;const failure=el('div','app-chat-failure-slot');failure.hidden=true;
  const form=el('form','app-chat-form'),input=el('textarea');input.rows=2;input.placeholder='Find work, change a shortcut…';input.setAttribute('aria-label','Message App chat');input.maxLength=4000;
  const footer=el('div','app-chat-footer'),hint=el('span',null,'Enter to send · Shift+Enter for a new line'),stop=el('button','text-button','Stop'),send=el('button','button','Send');stop.type='button';stop.hidden=true;send.type='submit';hint.setAttribute('role','status');footer.append(hint,stop,send);form.append(input,footer);panel.append(heading,context,conversation,error,failure,form);document.body.append(panel);
  let open=false,pending=false,timer=null,lastMessages='',request=null,handled=new Set(),deferred=[],assistant=null,jobAssistant=null,assistantChecked=false;
  function snapshot(){
    const dialog=[...document.querySelectorAll('dialog[open]')].at(-1),review=dialog?.id==='diff-review-dialog'&&typeof reviewState!=='undefined'?reviewState:null;
    const inlineSetup=!$('#app-chat-setup')?.hidden&&Boolean($('#app-chat-setup'))&&!$('#app-chat-panel')?.hidden;
    const record=review?.record||(!dialog&&!inlineSetup&&source==='agents'&&typeof currentAgentRecord==='function'?currentAgentRecord():null);
    const dialogTitle=dialog?.querySelector('h1,h2,h3')?.textContent||dialog?.getAttribute('aria-label')||dialog?.id||(inlineSetup?'Set up truffle':'');
    const settingsFilter=dialog?.id==='settings-dialog'&&typeof settingsFilterContext==='function'?settingsFilterContext():null;
    return {...(typeof chatPreparationContext==='function'?{chatRecovery:chatPreparationContext()}:{}),source:dialog?.id==='settings-dialog'?'settings':source,query:source==='chat-results'?($('.chat-search-form input')?.value||''):$('#search').value,resultTargets:source==='chat-results'?[...document.querySelectorAll('#chat-search-matches [data-work-id],#chat-search-matches [data-chat-result]')].map(node=>({type:node.dataset.workId?'work':'chat',id:node.dataset.workId||node.dataset.chatResult})).slice(0,40):[],recordId:record?.id||'',worktree:review?.directory||(record&&typeof selectedGitDirectory==='function'?selectedGitDirectory(record):''),filters:{status:statusFilter.value,group:$('#group-by').value,scope:scopePrefs[source]||'',...(typeof currentSavedViewState==='function'&&currentSavedViewState(source)?{savedView:{id:currentSavedViewState(source).id,lifecycle:currentSavedViewState(source).draft.lifecycle,filterSummary:JSON.stringify(currentSavedViewState(source).draft.filter).slice(0,1500),filterSummaryTruncated:JSON.stringify(currentSavedViewState(source).draft.filter).length>1500}}:{}),...(source==='github'?{githubView}: {}),...(source==='sentry'?{sentryView,sentryFilters}: {})},ui:{...(settingsFilter?{settingsFilter}:{}),timezone:Intl.DateTimeFormat().resolvedOptions().timeZone||'UTC',heading:$('#page-title')?.textContent||$('h1')?.textContent||'',dialog:dialogTitle,settingsCategory:dialog?.id==='settings-dialog'&&typeof settingsCategory!=='undefined'?settingsCategory:'',sharingOpen:Boolean($('#sharing-menu')?.open),sharingInbox:Boolean($('#sharing-menu')?.open&&[...$('#sharing-menu').querySelectorAll('.sharing-tabs button')].some(button=>button.textContent==='Inbox'&&button.getAttribute('aria-pressed')==='true')),reviewFile:review?.activePath||review?.file?.path||''},visibleIds:(typeof visibleWorkItems==='undefined'?[]:visibleWorkItems).slice(0,100).map(item=>item.id)};
  }
  function stamp(){return JSON.stringify(snapshot());}
  function displayContext(page){return 'Context · '+(page.ui.dialog||(page.source==='settings'?'Settings':sourceNames[page.source]||page.source))+(page.recordId&&!page.ui.dialog?' · '+(currentAgentRecord()?.title||page.recordId):'');}
  function updateContext(){const label=displayContext(snapshot());if(context.textContent!==label)context.textContent=label;}
  function updateAssistant(value){assistant=appChatAssistant(value.assistant);jobAssistant=appChatAssistant(value.job?.assistant);assistantChecked=true;}
  function update(){const shown=pending&&jobAssistant?jobAssistant:assistant;badge.textContent=shown?.label||(assistantChecked?'Unavailable':'Checking…');badge.title=shown?'Replies use '+shown.label:assistantChecked?'Install Claude or Codex to send messages':'Checking installed assistants';hint.textContent=pending?(jobAssistant?jobAssistant.label+' is replying…':'Starting reply…'):assistant?'Enter to send · Shift+Enter for a new line':assistantChecked?'Install Claude or Codex to send. You can continue setup without one.':'Checking installed assistants…';button.setAttribute('aria-expanded',String(open));panel.hidden=!open;panel.classList.toggle('has-setup',!setup.hidden);send.disabled=pending||!assistant||(!input.value.trim()&&!window.truffleChatImages?.count());stop.hidden=!pending;activity.hidden=!pending;fresh.disabled=pending;button.classList.toggle('is-working',pending);button.setAttribute('aria-busy',String(pending));button.setAttribute('aria-label',pending?'App chat · Working':'App chat');messages.setAttribute('aria-busy',String(pending));updateContext();}
  function showError(message,job=null){error.textContent=message;error.hidden=!message;failure.replaceChildren();failure.hidden=!job;if(job)failure.append(appChatFailureActions(job,async()=>{try{await appChatOpenFailureLogs(job);update();}catch(problem){showError(problem.message,job);}}));}
  async function apply(commands){
    if(typeof settingsDirty!=='undefined'&&settingsDirty){deferred=commands;showError('Save or discard your Settings changes, then choose Show results.');renderDeferred();return;}
    const command=commands.at(-1);if(!command)return;
    const setupHost=$('#app-chat-setup'),setupOpen=Boolean(setupHost&&!setupHost.hidden);
    if((document.querySelector('dialog[open]')||setupOpen)&&!(command.type==='navigate'&&((command.destination==='settings'&&$('#settings-dialog')?.open)||(command.destination==='setup'&&(setupOpen||$('#onboarding-dialog')?.open))))){deferred=commands;showError(setupOpen?'Close setup, then choose Show results.':'Close the open dialog, then choose Show results.');renderDeferred();return;}
    if(command.type==='settings-proposal'){
      try{appChatOpenSettingsProposal(command);open=false;showError('');update();deferred=[];renderDeferred();}catch(problem){deferred=commands;showError(problem.message);renderDeferred();}return;
    }
    if(command.type==='navigate'){
      try{await appChatNavigate(command);showError('');update();deferred=[];renderDeferred();}catch(problem){deferred=commands;showError(problem.message);renderDeferred();}return;
    }
    if(command.type==='saved-view'||command.type==='display-items'){try{await applyResultPresentation(command);update();deferred=[];renderDeferred();}catch(problem){deferred=commands;renderDeferred();showError(problem.message);}return;}
    if(command.type==='issue-draft'){openFeedbackDraft(command);deferred=[];renderDeferred();return;}
    if(command.type==='chat-results'){showChatResults(command);deferred=[];renderDeferred();return;}
    if(command.type==='open-chat'){
      try{if(command.results)rememberChatResults(command.results);await openChatSearchTarget(command.id);deferred=[];renderDeferred();}
      catch(problem){showError(problem.message);}return;
    }
    source=command.source;$('#search').value=command.query;statusFilter.value=command.status;
    if(typeof applyAgentWorkView==='function')applyAgentWorkView(command);
    if(['linear','github','sentry'].includes(source))scopePrefs[source]='';
    if(source==='github')githubView='all';
    persist();render();update();deferred=[];renderDeferred();
  }
  function renderDeferred(){panel.querySelector('.app-chat-show-results')?.remove();if(!deferred.length)return;const show=el('button','text-button app-chat-show-results','Show results');show.onclick=()=>apply(deferred);form.before(show);}
  function draw(rows){const signature=JSON.stringify(rows);if(signature===lastMessages)return;lastMessages=signature;messages.replaceChildren();if(!rows.length)messages.append(el('p','app-chat-empty','Ask about cached work or change an app setting. The assistant sees this page’s filters and linked work, without terminal text. Paste a screenshot to include it in your next message.'));
    for(const row of rows){const entry=el('article','app-chat-message '+(row.role==='user'?'is-user':'is-assistant'));const body=el('div','app-chat-message-body');if(row.role==='assistant'){body.classList.add('app-chat-markdown');body.innerHTML=renderBriefingMarkdown(row.text,typeof allItems==='function'?allItems():[]);}else body.textContent=row.text;entry.append(el('span','app-chat-author',row.role==='user'?'You':'App assistant'),body);messages.append(entry);}if(setup.hidden)conversation.scrollTop=conversation.scrollHeight;
  }
  async function poll(){
    clearTimeout(timer);
    try{const value=await api('/api/app-chat');updateAssistant(value);draw(value.messages||[]);const job=value.job;pending=job?.state==='running';
      if(job&&!pending&&!handled.has(job.id)){handled.add(job.id);if(job.state==='failed')showError(appChatFailureMessage(job),job);else{
        const commands=(job.commands||[]).map(appChatCommand),applyNow=request?.id===job.id&&request.stamp===stamp();
        deferred=commands;renderDeferred();
        if(commands.at(-1)?.type!=='saved-view'||!applyNow)await appChatRefreshSavedViews();
        if(commands.length&&applyNow)await apply(commands);
      }
        if(typeof settingsDirty==='undefined'||!settingsDirty)await loadAppSettings();else if(job.state!=='failed')showError('The assistant may have changed settings. Your unsaved settings are kept; save or discard them before reloading.');
        if(job.state==='succeeded')await window.truffleGuidance?.refresh();
      }
    }catch(problem){showError(problem.message);}
    update();if(open||pending)timer=setTimeout(poll,pending?1000:5000);
  }
  button.onclick=()=>{open=!open;update();if(open){poll();input.focus();}else{window.closeOnboarding?.({host:setup});if(!pending)clearTimeout(timer);}};
  fresh.onclick=async()=>{if(pending)return;fresh.disabled=true;try{await api('/api/app-chat/new',{});window.closeOnboarding?.({host:setup});window.truffleChatImages?.clear();request=null;deferred=[];renderDeferred();showError('');input.value='';lastMessages='';await poll();input.focus();}catch(problem){showError(problem.message);}finally{update();}};
  close.onclick=()=>{open=false;window.closeOnboarding?.({host:setup});update();if(!pending)clearTimeout(timer);button.focus();};
  input.oninput=update;input.onkeydown=event=>{if(event.key==='Enter'&&!event.shiftKey&&!event.isComposing){event.preventDefault();if(!send.disabled)form.requestSubmit();}else if(event.key==='Escape'){event.preventDefault();close.click();}};
  stop.onclick=async()=>{stop.disabled=true;try{await api('/api/app-chat/cancel',{});}catch(problem){showError(problem.message);}finally{stop.disabled=false;}};
  form.onsubmit=async event=>{event.preventDefault();if(pending||(!input.value.trim()&&!window.truffleChatImages?.count()))return;if(!assistant){showError(assistantChecked?'Install Claude or Codex to send. Your draft and attachments are kept; you can continue setup without an assistant.':'Wait for the installed-assistant check. Your draft and attachments are kept.');return;}if(typeof settingsDirty!=='undefined'&&settingsDirty){showError('Save or discard your Settings changes before asking App chat to change the app.');return;}
    const text=input.value.trim()||'Please look at the attached screenshot.';request={id:crypto.randomUUID(),stamp:stamp()};pending=true;jobAssistant=null;showError('');update();
    try{await api('/api/app-chat/send',{requestId:request.id,message:text,context:snapshot(),images:window.truffleChatImages?.payload()||[]});window.truffleChatImages?.clear();input.value='';await poll();}catch(problem){pending=false;showError(problem.message);update();}
  };
  window.prepareAppChatMessage=text=>{
    if(typeof text!=='string'||!text.trim())throw new Error('Enter setup instructions first.');
    const message=text.trim();if(new TextEncoder().encode(message).length>4000)throw new Error('This guide is too long for one message. Paste a shorter section.');
    if(pending)throw new Error('Wait for the current assistant reply before preparing another message.');
    if(input.value.trim()||window.truffleChatImages?.count())throw new Error('Send or clear your current App chat draft and attachments first.');
    input.value=message;open=true;showError('');update();input.focus();
  };
  window.openAppChatSetup=async()=>{open=true;update();await appChatNavigate({type:'navigate',destination:'setup'});await poll();};
  function placeChat(){const host=[...document.querySelectorAll('dialog[open]')].filter(dialog=>!panel.contains(dialog)).at(-1)||document.body;if(button.parentNode!==host)host.append(button,panel);update();}
  new MutationObserver(changes=>{if(changes.some(change=>change.target.tagName==='DIALOG'))placeChat();}).observe(document.body,{subtree:true,attributes:true,attributeFilter:['open']});
  new MutationObserver(()=>{update();if(!setup.hidden&&open)conversation.scrollTop=setup.offsetTop;}).observe(setup,{attributes:true,attributeFilter:['hidden']});
  window.addEventListener('truffle-service-restarted',()=>{if(open)poll();});
  registerRenderHook('app-chat:context',updateContext,{order:50});
  update();
})();
