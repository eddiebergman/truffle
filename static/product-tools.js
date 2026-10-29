'use strict';
const chatShells=new Map(),openChatShells=new Set(),chatShellRequests=new Map();
function chatTerminalCurrent(record,key,host){return host?.isConnected&&$('#agent-workspace')===host&&source==='agents'&&selectedAgentKey===key&&currentAgentRecord()?.id===record.id;}
function focusChatTerminal(record,key,view,origin,host,fallback=null){
 requestAnimationFrame(()=>{const target=view?.node||fallback;if(!chatTerminalCurrent(record,key,host)||!target?.isConnected||target.closest('[hidden]')||document.querySelector('dialog[open]'))return;const active=document.activeElement;if(active!==origin&&active!==document.body&&active?.isConnected&&!target.contains(active))return;if(view){view.resize();view.terminal.focus();}else fallback.focus({preventScroll:true});});
}
async function showChatTerminal(record,{restart=false}={}){
 if(chatShellRequests.has(record.id))return;const key=selectedAgentKey,origin=document.activeElement,host=$('#agent-workspace'),request={};chatShellRequests.set(record.id,request);
 try{
  let shell=chatShells.get(record.id);
  if(shell&&(restart||terminalViews.get(shell.id)?.stopped)){const view=terminalViews.get(shell.id);if(restart&&view?.alive)await api('/api/shell-terminal/close',{id:shell.id,generation:view.generation});if(view){view.stopped=true;view.observer.disconnect();view.terminal.dispose();view.node.remove();terminalViews.delete(shell.id);}chatShells.delete(record.id);shell=null;}
  if(!shell||terminalViews.get(shell.id)?.stopped){shell=await api('/api/shell-terminal/open',{recordId:record.id});chatShells.set(record.id,{...shell,terminalRole:'shell'});}
  window.truffleGuidance?.complete('chat.shell');openChatShells.add(record.id);
  if(chatTerminalCurrent(record,key,host)){render();focusChatTerminal(record,key,terminalViews.get(shell.id),origin,host);}
 }catch(error){toast(error.message);}
 finally{if(chatShellRequests.get(record.id)===request)chatShellRequests.delete(record.id);}
}
async function toggleChatTerminal(){
 if(source!=='agents')return;const record=currentAgentRecord();if(!record||chatShellRequests.has(record.id))return;
 if(openChatShells.has(record.id)){const key=selectedAgentKey,origin=document.activeElement,host=$('#agent-workspace');openChatShells.delete(record.id);render();focusChatTerminal(record,key,terminalViews.get(record.id),origin,host,$('#chat-terminal-toggle'));return;}
 return showChatTerminal(record);
}
function renderChatTerminal(record,conversation){
 let toggle=$('#chat-terminal-toggle');if(!toggle){toggle=el('button','icon-button');toggle.id='chat-terminal-toggle';toggle.title='Toggle chat terminal · '+shortcutLabel(setting('keybindings',{})['chat-terminal']);toggle.setAttribute('aria-label','Toggle chat terminal');toggle.innerHTML='<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m6 9 3 3-3 3m6 0h5"/></svg>';toggle.onclick=()=>void toggleChatTerminal();$('#skills-inbox').before(toggle);}
 toggle.hidden=source!=='agents';toggle.setAttribute('aria-pressed',String(openChatShells.has(record.id)));
 conversation.classList.toggle('has-chat-shell',openChatShells.has(record.id));
 let panel=$('.chat-shell-panel',conversation);
 if(!openChatShells.has(record.id)){if(panel){const shell=chatShells.get(record.id),view=shell&&terminalViews.get(shell.id);if(view&&panel.contains(view.node))terminalPool.append(view.node);panel.remove();}return;}
 const shell=chatShells.get(record.id);if(!shell)return;
 if(panel&&panel.dataset.shellId!==shell.id){panel.remove();panel=null;}
 if(!panel){panel=el('section','chat-shell-panel');panel.dataset.shellId=shell.id;const head=el('div','chat-shell-heading'),title=el('span','','Terminal'),cwd=el('code','',shell.cwd),hide=featureButton('×',()=>toggleChatTerminal(),'icon-button'),restart=featureButton('Restart shell',()=>showChatTerminal(record,{restart:true}),'text-button');hide.setAttribute('aria-label','Hide chat terminal');head.append(title,cwd,restart,hide);panel.append(head);conversation.append(panel);}
 const view=ensureTerminal(shell);if(!panel.contains(view.node))panel.append(view.node);requestAnimationFrame(()=>{if(view.node.isConnected&&!view.node.closest('[hidden]'))view.resize();});
}
new MutationObserver(()=>{const button=$('#chat-terminal-toggle');if(button)button.hidden=source!=='agents';}).observe(document.body,{attributes:true,attributeFilter:['class']});
let actionOutputId='',actionOutputTimer=null;
async function openActionOutput(id){
 actionOutputId=id||'';let panel=$('#action-output-panel');if(!panel){panel=el('section','action-output-panel');panel.id='action-output-panel';panel.setAttribute('aria-label','Quick action output');const head=el('header'),title=el('strong','','Quick action output'),close=featureButton('×',()=>{panel.hidden=true;clearTimeout(actionOutputTimer);},'icon-button');close.setAttribute('aria-label','Close action output');head.append(title,close);const select=el('select');select.id='action-output-run';select.setAttribute('aria-label','Action run');select.onchange=()=>{actionOutputId=select.value;void refreshActionOutput();};const meta=el('div','form-hint');meta.id='action-output-meta';const output=el('pre');output.id='action-output-text';output.setAttribute('role','log');panel.append(head,select,meta,output);document.body.append(panel);}panel.hidden=false;await refreshActionOutput();
}
async function refreshActionOutput(){clearTimeout(actionOutputTimer);const panel=$('#action-output-panel');if(!panel||panel.hidden)return;try{const {jobs}=await api('/api/quick-actions'),rows=jobs.sort((a,b)=>b.startedAt-a.startedAt);const select=$('#action-output-run');select.replaceChildren();for(const row of rows)select.add(new Option(row.title+' · '+row.state,row.id));if(!rows.some(row=>row.id===actionOutputId))actionOutputId=rows[0]?.id||'';select.value=actionOutputId;const job=rows.find(row=>row.id===actionOutputId);$('#action-output-meta').textContent=job?[job.command,job.cwd,job.state,job.exitCode==null?null:'Exit '+job.exitCode,job.message].filter(Boolean).join(' · '):'No saved runs yet.';$('#action-output-text').textContent=job?.output||(job?.state==='running'?'Running…':'This command produced no output.');if(rows.some(row=>row.state==='running'))actionOutputTimer=setTimeout(refreshActionOutput,750);}catch(error){$('#action-output-text').textContent=error.message;}}
async function openThemeChooser(){
 if(!requestDialogNavigation())return false;
 const opening={},navigation=captureDialogNavigation();openThemeChooser.current=opening;let baseline;
 try{baseline=await loadAppSettings({canApply:()=>openThemeChooser.current===opening&&navigation.isCurrent()});}finally{navigation.dispose();}
 if(!baseline||openThemeChooser.current!==opening||!navigation.isCurrent())return false;
 const dialog=$('#quick-action-picker'),form=$('#quick-action-picker-form'),page=source;
 form.replaceChildren(el('h2','','Change theme'));
 const query=el('input'),list=el('div','theme-choice-list'),message=el('p','form-hint'),actions=el('div','ui-actiongroup');
 query.type='search';query.placeholder='Find a theme…';query.setAttribute('aria-label','Find a theme');message.tabIndex=-1;message.setAttribute('role','status');
 actions.setAttribute('role','group');actions.setAttribute('aria-label','Theme chooser actions');
 const current={dialog,host:form,pending:false,refreshing:false,result:null,controls:new Map(),ui:{message}};openThemeChooser.current=current;
 const owns=()=>openThemeChooser.current===current&&$('#quick-action-picker')===dialog&&$('#quick-action-picker-form')===form&&form.isConnected&&query.parentElement===form;
 current.isCurrent=()=>owns()&&dialog.open&&source===page;
 const unobscured=()=>![...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog&&!open.contains(dialog));
 const dispose=()=>{dialog.removeEventListener('cancel',cancel);dialog.removeEventListener('close',closed);observer.disconnect();};
 const cancel=event=>{if(owns()&&(current.pending||current.refreshing))event.preventDefault();};
 const closed=()=>{if(!dialog.open)dispose();};
 const observer=new MutationObserver(()=>{if(!owns())dispose();});observer.observe(form,{childList:true});
 dialog.addEventListener('cancel',cancel);dialog.addEventListener('close',closed);
 const close=settingsButton('Close',()=>{if(current.isCurrent())requestDialogClose(dialog);},'button');close.dataset.featureRecovery='true';
 const refresh=settingsButton('Refresh view',()=>recover(false),'button'),reload=settingsButton('Reload themes',()=>recover(true),'button');
 for(const button of [refresh,reload]){button.dataset.featureRecovery='true';button.hidden=true;}
 actions.append(close,refresh,reload);form.append(query,list,message,actions);
 const paintOutcome=()=>{
  if(!current.isCurrent())return;
  const locked=current.result?.kind==='accepted'||current.result?.kind==='unknown'||current.result?.conflict;
  freezeFeatureForm(current,!!locked);close.disabled=current.pending||current.refreshing||!!current.closeDisabled;
  refresh.hidden=!['accepted','unknown'].includes(current.result?.kind);reload.hidden=!current.result?.conflict;
  refresh.disabled=reload.disabled=current.pending||current.refreshing;
  if(current.result?.kind==='accepted')message.textContent='Theme saved. The view could not update. Refresh the view without saving again.';
  else if(current.result?.kind==='unknown')message.textContent='The theme save outcome is unknown. Refresh and inspect the saved theme before making another change. This choice will not be resubmitted.';
  else if(current.result?.conflict)message.textContent='Settings changed elsewhere. No theme was saved. Reload themes before choosing again.';
  else if(current.result)message.textContent=current.result.error;
  restoreFormFocus(current,current.result?.conflict?reload:current.result?.kind==='unknown'?close:current.result?.kind==='accepted'?refresh:current.focusReturn);
 };
 const readCurrent=async()=>{
  for(let attempt=0;attempt<2;attempt++){
   if(!current.isCurrent())return null;
   const identity=appSettings,next=await loadAppSettings({canApply:()=>current.isCurrent()&&appSettings===identity});
   if(!current.isCurrent())return null;
   if(next&&!next.configError)return next;
  }
  throw new Error('The current settings could not be displayed.');
 };
 async function recover(conflict){
  if(!current.isCurrent()||current.pending||current.refreshing)return;
  current.refreshing=true;holdFormFocus(form,current);freezeFeatureForm(current,true);close.disabled=refresh.disabled=reload.disabled=true;
  try{
   const next=await readCurrent();if(!next||!current.isCurrent())return;
   if(conflict){baseline=next;current.result=null;draw();message.textContent='Current settings loaded. Choose a theme.';}
   else if(current.result.kind==='accepted'){if(unobscured())dialog.close();return;}
   else{for(const button of list.querySelectorAll('button'))button.setAttribute('aria-pressed',String(setting('theme','')===button.dataset.themeId));message.textContent='Saved settings refreshed. The earlier save outcome remains unknown. Inspect the current theme; this choice was not resubmitted.';}
  }catch(error){if(current.isCurrent())message.textContent=(current.result?.kind==='accepted'?'Theme saved. ':current.result?.kind==='unknown'?'The theme save outcome remains unknown. ':'No theme was saved. ')+error.message;}
  finally{current.refreshing=false;if(current.isCurrent()){const text=message.textContent;paintOutcome();message.textContent=text;restoreFormFocus(current,conflict&&!current.result?query:current.result?.kind==='unknown'?close:refresh);}}
 }
 async function choose(theme){
  if(!current.isCurrent()||current.pending||current.refreshing||current.result?.kind==='accepted'||current.result?.kind==='unknown'||current.result?.conflict)return;
  current.closeDisabled=close.disabled;current.pending=true;holdFormFocus(form,current);freezeFeatureForm(current,true);close.disabled=refresh.disabled=reload.disabled=true;message.textContent='Saving theme…';
  try{await api('/api/settings',{revision:baseline.revision,contentHash:baseline.contentHash,patch:{theme:theme.id}});current.result={kind:'accepted'};}
  catch(error){current.result={kind:[400,401,403,404,409].includes(error.status)?'rejected':'unknown',conflict:error.status===409,error:error.message};}
  if(current.result.kind!=='rejected')appSettings={...appSettings};
  try{
   if(current.result.kind==='accepted'&&current.isCurrent()){const next=await readCurrent();if(next&&current.isCurrent()&&unobscured()){current.result.presentation='ready';dialog.close();return;}}
  }catch(error){current.result.error=error.message;}
  finally{current.pending=false;if(current.isCurrent())paintOutcome();else{dispose();if(current.result.kind!=='rejected'&&current.result.presentation!=='ready')toast(current.result.kind==='accepted'?'Theme saved. Refresh settings to inspect it.':'The theme save outcome is unknown. Inspect settings before choosing again.');}}
 }
 const draw=()=>{for(const control of list.querySelectorAll('button'))current.controls.delete(control);list.replaceChildren();for(const theme of Object.values(window.mydashThemeCatalog).filter(theme=>(theme.label+' '+theme.type).toLowerCase().includes(query.value.toLowerCase()))){const button=featureButton(theme.label+' · '+theme.type,()=>choose(theme),'theme-choice');button.dataset.themeId=theme.id;button.setAttribute('aria-pressed',String(setting('theme','')===theme.id));list.append(button);}};
 query.oninput=()=>{if(current.isCurrent()&&!current.pending&&!current.refreshing&&!current.result?.conflict&&!['accepted','unknown'].includes(current.result?.kind))draw();};
 form.onsubmit=event=>{event.preventDefault();if(current.isCurrent())list.querySelector('button')?.click();};
 draw();dialog.showModal();query.focus();return true;
}
// Sharing has its own CAS document; keep its reviewed draft outside settings.json.
const sharingSettingsDraft={baseline:null,draft:null,loading:null,saving:false,change:0,operation:0,status:''};
let sharingSettingsRenderer=0,sharingSettingsRefresh=null;
function sharingSettingsConfig(value){
 const config=value?.config;
 if(config?.error)throw new Error(typeof config.error==='string'?config.error:'Could not load sharing configuration.');
 if(!config||!Number.isSafeInteger(config.revision)||config.revision<0||typeof config.contentHash!=='string'||!/^[a-f0-9]{64}$/.test(config.contentHash))throw new Error('Could not load sharing configuration. Reload it before saving.');
 return structuredClone(config);
}
function sharingSettingsFields(config){const transport=sharingTransport(config);return {kind:transport.kind,root:transport.root,handle:config.handle||''};}
async function renderSharingSettings(panel){
 const draft=sharingSettingsDraft,hadBaseline=!!draft.baseline,ticket=++sharingSettingsRenderer,container=el('section','sharing-preferences');panel.append(container);container.append(el('p','form-hint','Loading sharing configuration…'));
 const live=()=>ticket===sharingSettingsRenderer&&container.isConnected&&(!container.closest('dialog')||container.closest('dialog').open);
 try{
  if(!draft.baseline){
   if(!draft.loading)draft.loading=sharingCommand('sharing.state',{}).then(value=>{draft.baseline=sharingSettingsConfig(value);draft.draft=sharingSettingsFields(draft.baseline);}).finally(()=>{draft.loading=null;});
   await draft.loading;if(!live())return;
  }
  container.replaceChildren();container.append(el('h3','','Sharing destination'),el('p','form-hint','Share selected Context Bundles through an existing folder or Google Cloud Storage.'));
  const kind=el('select','ui-field');kind.setAttribute('aria-label','Sharing storage type');kind.add(new Option('Local / company folder (local extension)','folder'));kind.add(new Option('Google Cloud Storage','gcs'));
  const root=el('input','ui-field'),handle=el('input','ui-field'),inbox=el('code','sharing-inbox-uri'),status=el('p','form-hint'),hint=el('p','form-hint');status.setAttribute('role','status');root.maxLength=2000;root.setAttribute('aria-label','Sharing storage root');handle.maxLength=65;handle.placeholder='your-name';handle.setAttribute('aria-label','Receiving inbox name');
  const dirty=()=>JSON.stringify(draft.draft)!==JSON.stringify(sharingSettingsFields(draft.baseline));
  const update=()=>{root.placeholder=kind.value==='folder'?'Absolute path to a mounted shared folder':'gs://your-bucket/context-bundles/v1';hint.textContent=sharingTransportHint(kind.value);inbox.textContent=root.value.trim()&&handle.value.trim()?root.value.trim().replace(/\/$/,'')+'/inboxes/'+handle.value.replace(/^\//,'').toLowerCase()+'/deliveries/':'Configure storage and inbox to see receiving path';};
  const changed=()=>{if(!live()||draft.saving)return;draft.draft={kind:kind.value,root:root.value,handle:handle.value};draft.change++;draft.status='';update();sync();};
  for(const[label,input]of[['Storage type',kind],['Storage root',root],['Receiving inbox',handle]]){const row=el('label','sharing-preference-field');row.append(el('strong','',label),input);container.append(row);input.oninput=changed;input.onchange=changed;}
  container.append(hint,el('p','','Your inbox location'),inbox,el('p','form-hint','Inbox names are routing labels, not verified identities. Saving or importing a profile does not upload context or grant access. Unsaved drafts are kept while this app stays open.'));
  const save=settingsButton('Save sharing configuration',async()=>{
   if(!live()||draft.saving)return;
   let payload;try{payload=sharingConfigurationPayload(draft.baseline,draft.draft.kind,draft.draft.root,draft.draft.handle);}catch(error){draft.status=error.message;sync();return;}
   const submitted=JSON.stringify(draft.draft);draft.saving=true;draft.operation++;draft.status='Saving sharing configuration…';sync();
   try{const result=await sharingCommand('sharing.configure',payload);draft.baseline=sharingSettingsConfig(result);if(JSON.stringify(draft.draft)===submitted){draft.draft=sharingSettingsFields(draft.baseline);draft.change++;draft.status='Sharing configuration saved. No content was uploaded.';}else draft.status='Earlier sharing configuration saved. Your newer draft is still unsaved.';}
   catch(error){draft.status=error.message;}
   finally{draft.saving=false;sharingSettingsRefresh?.();}
  },'button primary');
  const check=settingsButton('Check inbox access',async()=>{if(!live()||draft.saving)return;check.disabled=true;try{await sharingCommand('sharing.refresh',{});if(live()){draft.status='Inbox refresh requested for the saved destination. Read status in the sharing menu; no test share was uploaded.';sync();}}catch(error){if(live()){draft.status=error.message;sync();}}finally{if(live())check.disabled=draft.saving;}},'button');
  const reload=settingsButton('Discard draft & reload',async()=>{
   if(!live()||draft.saving)return;const operation=++draft.operation,change=draft.change;
   try{const value=await sharingCommand('sharing.state',{});if(!live()||operation!==draft.operation)return;if(change!==draft.change)throw new Error('Sharing draft changed while reloading. Your edits were kept; discard and reload again to replace them.');draft.baseline=sharingSettingsConfig(value);draft.draft=sharingSettingsFields(draft.baseline);draft.change++;draft.status='Sharing draft discarded. Saved configuration reloaded.';sync();}
   catch(error){if(live()&&operation===draft.operation){draft.status=error.message;sync();}}
  },'text-button');
  const exportProfile=settingsButton('Export sharing profile',()=>{if(!live()||draft.saving)return;try{const profile=sharingProfile(draft.draft.kind,draft.draft.root,draft.draft.handle),link=el('a');link.href=URL.createObjectURL(new Blob([JSON.stringify(profile,null,2)],{type:'application/json'}));link.download='truffle-sharing.json';link.click();setTimeout(()=>URL.revokeObjectURL(link.href),1000);}catch(error){draft.status=error.message;sync();}},'text-button');
  const file=el('input');file.type='file';file.accept='.json,application/json';file.hidden=true;file.onchange=async()=>{
   if(!live()||draft.saving){file.value='';return;}const selected=file.files[0];if(!selected)return;const operation=++draft.operation,change=draft.change;
   try{if(selected.size>65536)throw new Error('Sharing profile exceeds 64 KiB.');const profile=parseSharingProfile(JSON.parse(await selected.text()));if(!live()||operation!==draft.operation)return;if(change!==draft.change)throw new Error('Sharing draft changed while reading the profile. Your edits were kept; import it again.');draft.draft={kind:profile.transport.kind,root:profile.transport.root,handle:profile.identity.handle};draft.change++;draft.status='Profile loaded for review. Save to apply this destination.';sync();}
   catch(error){if(live()&&operation===draft.operation){draft.status=error.message;sync();}}
   finally{file.value='';}
  };
  const importProfile=settingsButton('Import sharing profile',()=>{if(live()&&!draft.saving)file.click();},'text-button');
  const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label','Sharing configuration actions');actions.append(save,check,reload,exportProfile,importProfile);container.append(actions,file,status);
  function sync(){kind.value=draft.draft.kind;root.value=draft.draft.root;handle.value=draft.draft.handle;update();for(const control of [kind,root,handle,save,check,reload,exportProfile,importProfile,file])control.disabled=draft.saving;status.textContent=draft.status||(dirty()?'Unsaved sharing draft. Save sharing configuration to apply it.':'Sharing configuration is saved.');}
  sharingSettingsRefresh=()=>{if(live())sync();};sync();
  if(hadBaseline&&!draft.saving&&!dirty()){
   const operation=++draft.operation,change=draft.change;
   void sharingCommand('sharing.state',{}).then(value=>{if(!live()||operation!==draft.operation||change!==draft.change||draft.saving)return;draft.baseline=sharingSettingsConfig(value);draft.draft=sharingSettingsFields(draft.baseline);draft.status='';sync();}).catch(error=>{if(live()&&operation===draft.operation){draft.status=error.message;sync();}});
  }
 }catch(error){if(live())container.replaceChildren(el('p','inline-error',error.message));}
}

let onboardingOffered=false;
async function maybeOfferOnboarding(){if(onboardingOffered||!state)return;onboardingOffered=true;if(!(state.items||[]).length&&!(state.sessions||[]).length){try{const setup=await api('/api/onboarding');if(!setup.progress.completed&&!Object.keys(setup.progress.steps||{}).length)await window.openAppChatSetup();}catch(error){console.warn('Setup unavailable:',error.message);}}}
