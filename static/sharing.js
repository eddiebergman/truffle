/* Portable handover UI. Cloud and session work remain background service jobs. */
'use strict';
function sharingCommand(name,args){return api('/api/commands/invoke',{name,args});}
function normalizeShareHandle(value){const name=String(value||'').trim().replace(/^\//,'').toLowerCase();return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(name)?name:'';}
function matchingSharePeople(people,query){const q=String(query||'').replace(/^\//,'').toLowerCase().trim();return people.filter(p=>p.active!==false&&normalizeShareHandle(p.id)&&[p.name,p.id].some(v=>String(v||'').toLowerCase().includes(q))).slice(0,8);}
function sharingTransport(config){
 if(['gcs','folder'].includes(config?.transport?.kind))return {kind:config.transport.kind,root:config.transport.root||''};
 return config?.root?{kind:'gcs',root:config.root}:{kind:'folder',root:''};
}
function sharingProfile(kind,root,handle){
 const name=normalizeShareHandle(handle);root=String(root).trim().replace(/\/+$/,'');
 if(!name)throw new Error('Choose a receiving handle using letters, numbers, underscore or hyphen.');
 if(!['gcs','folder'].includes(kind)||!root||root.includes('\0')||root.length>2000)throw new Error('Choose a supported storage type and root.');
 if(kind==='gcs'&&!/^gs:\/\/[a-zA-Z0-9._-]+(?:\/[a-zA-Z0-9._-]+)*$/.test(root))throw new Error('Use a plain gs://bucket/prefix.');
 if(kind==='folder'&&(!root.startsWith('/')||root==='/'||root.split('/').some(part=>part==='.'||part==='..')))throw new Error('Choose an absolute mounted folder path.');
 return {format:'truffle-sharing',version:1,transport:{kind,root},identity:{handle:name}};
}
function parseSharingProfile(profile){
 if(!profile||profile.format!=='truffle-sharing'||profile.version!==1||typeof profile.transport?.root!=='string'||typeof profile.identity?.handle!=='string')throw new Error('Unsupported sharing profile.');
 return sharingProfile(profile.transport.kind,profile.transport.root,profile.identity.handle);
}
function sharingConfigurationPayload(config,kind,root,handle){
 const profile=sharingProfile(kind,root,handle);
 if(!Number.isSafeInteger(config?.revision)||typeof config.contentHash!=='string'||!/^[a-f0-9]{64}$/.test(config.contentHash))throw new Error('Reload sharing settings before saving.');
 return {revision:config.revision,contentHash:config.contentHash,handle:profile.identity.handle,transport:profile.transport};
}
function sharingTransportHint(kind){return kind==='folder'?'Local folder extension: file:// delivery links require recipients to mount this folder at the same absolute path. Use a filesystem that supports atomic hard links. Existing filesystem or company-share permissions control access; this app does not grant access.':'Google Cloud Storage uses your existing Google identity and bucket permissions. Inbox handles route messages and do not verify sender identity.';}
if(typeof module!=='undefined')module.exports={normalizeShareHandle,matchingSharePeople,sharingTransport,sharingProfile,parseSharingProfile,sharingConfigurationPayload,sharingTransportHint};
if(typeof document!=='undefined')(()=>{
  const menu=$('#sharing-menu'),panel=$('#sharing-popover'),badge=$('#sharing-unread');
  let data=null,tab='share',loading=false,lastFetch=0,previewId='',previewJob='',archive=false,lastRendered='',previewStarting=false,renderedRecordId='';
  const locations=new Map(),shareDrafts=new Map(),receivingAgents=new Map(),imports=new Map();
  let openIntent=null;
  const jobDetails=new Map(),detailsLoading=new Set();
  function receiving(id=previewId){
    if(!imports.has(id))imports.set(id,{jobId:'',starting:false,opening:false,error:''});
    while(imports.size>20){const old=[...imports].find(([key,value])=>key!==id&&!value.starting&&!value.opening&&currentJob(value.jobId)?.status!=='working');if(!old)break;imports.delete(old[0]);}
    return imports.get(id);
  }
  function importedSession(entry){const job=currentJob(imports.get(entry?.id)?.jobId);return entry?.sessionId||(job?.status==='ready'?job.sessionId:'')||'';}
  function currentImportIntent(intent){return openIntent===intent&&menu.open&&tab==='inbox'&&previewId===intent.deliveryId&&selectedAgentKey===intent.agentKey&&source===intent.source;}
  function openImported(deliveryId,id){
    if(receiving(deliveryId).opening)return;
    previewId=deliveryId;lastRendered='';const intent={deliveryId,agentKey:selectedAgentKey,source,started:true};openIntent=intent;
    return showImported(id,intent);
  }
  function selected(){const live=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);return live?recordForCandidate(live):state?.sessions?.find(s=>'record:'+s.id===selectedAgentKey);}
  function chatDraft(record=selected()){
    if(!record)return null;
    if(!shareDrafts.has(record.id)){
      const prior=data?.jobs?.find(job=>job.kind==='prepare'&&job.recordId===record.id&&['ready','working'].includes(job.status));
      const prepare=typeof prior?.recentConversation==='boolean'?prior:null;
      const send=prior?data?.jobs?.find(job=>job.kind==='send'&&job.draftId===prior.id):null;
      shareDrafts.set(record.id,{recipient:send?.recipients?.[0]?('/'+send.recipients[0]).slice(0,200):'',message:String(send?.message||'').slice(0,4000),recent:prepare?.recentConversation??true,draftId:prepare?.id||'',sendId:prepare?send?.id||'':'',revision:0,preparing:false,sending:false});
      while(shareDrafts.size>20)shareDrafts.delete(shareDrafts.keys().next().value);
    }
    const draft=shareDrafts.get(record.id);shareDrafts.delete(record.id);shareDrafts.set(record.id,draft);return draft;
  }
  function draftSending(draft){return Boolean(draft?.sending||currentJob(draft?.sendId)?.status==='working');}
  function editDraft(draft,field,value){if(draftSending(draft))return;draft[field]=value;draft.revision++;draft.sendId='';}
  function button(label,action,classes='button'){const b=el('button',classes+' ui-control',label);b.dataset.variant=classes.includes('primary')?'primary':'secondary';b.type='button';b.onclick=action;return b;}
  function iconButton(label,glyph,action){const b=button(glyph,action,'sharing-icon');b.dataset.size='icon';b.title=label;b.setAttribute('aria-label',label);return b;}
  function actionGroup(label){const group=el('div','sharing-controls ui-actiongroup');group.setAttribute('role','group');group.setAttribute('aria-label',label);return group;}
  function message(node,text,error=false){if(!node)return;node.replaceChildren();if(text)node.append(el('p',error?'sharing-error':'sharing-note',text));}
  async function call(name,args){data=await sharingCommand(name,args);lastFetch=Date.now();paint();return data;}
  async function load(){if(loading||!state)return;loading=true;const initial=!data;try{data=await sharingCommand('sharing.state',{});lastFetch=Date.now();if($('#sharing-error'))message($('#sharing-error'),'');if(initial&&menu.open)build();else paint();}catch(e){if(menu.open)message($('#sharing-error'),e.message+(e.message.includes('Not found')?' Restart the updated local service to enable sharing.':''),true);}finally{loading=false;}}
  function field(label,input){const row=el('label','sharing-field');row.append(el('span','',label),input);return row;}
  async function openSharingSettings(){
    if(typeof settingsDirty!=='undefined'&&settingsDirty){message($('#sharing-error'),'Save or discard your Settings changes before opening Sharing settings.',true);return;}
    try{
      if(await openPreferences({category:'sharing'})!==true)throw new Error('Sharing settings could not open. Close the current dialog or finish saving, then try again.');
      menu.open=false;
    }catch(error){message($('#sharing-error'),error.message,true);}
  }
  function switchView(next){if(tab===next)return;openIntent=null;tab=next;build();$('#sharing-view-'+next)?.focus();}
  function build(){
    panel.replaceChildren();lastRendered='';
    const header=el('div','sharing-header'),tabs=el('div','sharing-tabs');tabs.setAttribute('role','group');tabs.setAttribute('aria-label','Sharing view');
    for(const [id,label]of [['share','Share'],['inbox','Inbox']]){const b=button(label,()=>switchView(id),'sharing-view'+(id===tab?' selected':''));b.id='sharing-view-'+id;b.setAttribute('aria-pressed',String(id===tab));tabs.append(b);}
    const actions=actionGroup('Sharing panel actions');actions.append(iconButton('Sharing settings','⚙',openSharingSettings),iconButton('Close sharing','×',()=>{menu.open=false;menu.querySelector('summary')?.focus();}));header.append(tabs,actions);panel.append(header);
    const errors=el('div','sharing-error');errors.id='sharing-error';errors.setAttribute('role','status');panel.append(errors);
    if(!data?.config?.root||!data?.config?.handle)panel.append(el('p','sharing-note','Configure a storage destination and receiving inbox in Sharing settings.'));
    if(tab==='share'){
      const record=selected(),draftState=chatDraft(record);renderedRecordId=record?.id||'';panel.append(el('strong','sharing-chat-title',record?.title||'Select an agent chat to share'));
      const prepare=button('Prepare handover',async()=>{
        if(!record||draftState.preparing||draftSending(draftState))return;const recentConversation=draftState.recent;draftState.preparing=true;draftState.error='';paint();
        try{const result=await call('sharing.prepare',{recordId:record.id,recentConversation});if(draftState.recent===recentConversation){draftState.draftId=result.jobId;draftState.sendId='';}lastRendered='';}
        catch(error){draftState.error=error.message;}finally{draftState.preparing=false;paint();}
      },'button');prepare.id='sharing-prepare';prepare.disabled=!record;
      const recent=el('label','sharing-check'),check=el('input');check.type='checkbox';check.checked=draftState?.recent??true;check.id='sharing-recent';check.onchange=()=>{if(!draftState)return;if(draftState.preparing||currentJob(draftState.draftId)?.status==='working'||draftSending(draftState)){check.checked=draftState.recent;return;}if(draftState.recent!==check.checked){draftState.recent=check.checked;draftState.draftId='';draftState.sendId='';draftState.revision++;lastRendered='';paint();}};recent.append(check,document.createTextNode('Recent conversation'));const controls=actionGroup('Prepare shared context');controls.append(prepare,recent);panel.append(controls);
      const recipient=el('input','ui-field');recipient.id='sharing-recipient';recipient.maxLength=200;recipient.value=draftState?.recipient||'';recipient.placeholder='Name or /handle';recipient.autocomplete='off';recipient.setAttribute('role','combobox');recipient.setAttribute('aria-autocomplete','list');recipient.setAttribute('aria-controls','sharing-people');recipient.setAttribute('aria-expanded','false');
      const people=el('div','sharing-people');people.id='sharing-people';people.setAttribute('role','listbox');people.setAttribute('aria-label','Recipient suggestions');
      const dismissPeople=e=>{if(e.key!=='Escape'||!people.childElementCount)return false;e.preventDefault();e.stopPropagation();recipient.focus();people.replaceChildren();recipient.setAttribute('aria-expanded','false');return true;};
      recipient.onfocus=paintPeople;recipient.oninput=()=>{if(draftState){editDraft(draftState,'recipient',recipient.value.slice(0,200));recipient.value=draftState.recipient;}paintPeople();paintSend();};recipient.onkeydown=e=>{if(dismissPeople(e))return;if(e.key==='ArrowDown'){e.preventDefault();people.querySelector('button')?.focus();}};
      people.onkeydown=e=>{if(dismissPeople(e))return;const buttons=[...people.querySelectorAll('button')],at=buttons.indexOf(document.activeElement);if(['ArrowDown','ArrowUp'].includes(e.key)){e.preventDefault();buttons[(at+(e.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();}};
      const note=el('textarea','ui-field');note.id='sharing-message';note.rows=2;note.maxLength=4000;note.value=draftState?.message||'';note.placeholder='Add a short message';note.oninput=()=>{if(draftState){editDraft(draftState,'message',note.value.slice(0,4000));note.value=draftState.message;}paintSend();};
      panel.append(field('To',recipient),people,field('Message (optional)',note));
      const draft=el('div','sharing-draft');draft.id='sharing-draft';panel.append(draft);
      const send=button('Send',sendDraft,'button primary');send.id='sharing-send';send.disabled=true;const sendActions=actionGroup('Send shared context');sendActions.append(send);panel.append(sendActions);
      if(!record){recipient.disabled=note.disabled=check.disabled=true;}
    }else{
      const controls=actionGroup('Inbox actions'),archiveToggle=button(archive?'Show inbox':'Show archived',()=>{openIntent=null;archive=!archive;archiveToggle.textContent=archive?'Show inbox':'Show archived';lastRendered='';paint();},'text-button');controls.append(button('Refresh',refreshInbox,'text-button'),archiveToggle);panel.append(controls);
      const list=el('div','sharing-entries');list.id='sharing-entries';panel.append(list);
      const preview=el('div','sharing-preview');preview.id='sharing-preview';panel.append(preview);
      const choices=actionGroup('Import shared context');choices.className+=' sharing-import-options';choices.id='sharing-import-options';choices.hidden=true;
      const agent=el('select','ui-field');agent.id='sharing-agent';agent.setAttribute('aria-label','Receiving agent');for(const [value,label]of [['claude','Claude'],['codex','Codex']])agent.add(new Option(label,value));agent.value=receivingAgents.get(previewId)||receivingAgents.get('')||$('#quick-agent').value;agent.onchange=()=>{receivingAgents.delete(previewId);receivingAgents.set(previewId,agent.value);while(receivingAgents.size>20)receivingAgents.delete(receivingAgents.keys().next().value);};
      const folder=el('select','ui-field');folder.id='sharing-folder';folder.setAttribute('aria-label','Receiving folder');folder.add(new Option('Choose folder…',''));const dirs=inventory.directories||[];for(const path of dirs)folder.add(new Option(path.replace(/^\/Users\/[^/]+/,'~'),path));if(!dirs.length)folder.add(new Option('Select a folder in the + menu',''));
      folder.onchange=()=>{locations.set(previewId,folder.value);lastRendered='';paint();};
      const go=button('Import into new chat',importEntry,'button primary');go.id='sharing-import';choices.append(agent,folder,go);panel.append(choices);
    }
    const footer=el('small','sharing-footnote');footer.id='sharing-status';footer.setAttribute('role','status');panel.append(footer);paint();
  }
  async function previewEntry(id){
    if(previewStarting)return;
    if(previewId!==id)openIntent=null;
    previewId=id;previewJob='';lastRendered='';paint();
    if(data.entries.find(entry=>entry.id===id)?.error)return;
    previewStarting=true;
    try{const result=await call('sharing.preview',{deliveryId:id});if(previewId===id){previewJob=result.jobId;lastRendered='';paint();}}
    catch(error){message($('#sharing-error'),error.message,true);}
    finally{previewStarting=false;}
  }
  async function refreshInbox(){
    try{await call('sharing.refresh',{});if(previewId&&currentJob(previewJob)?.status==='failed')await previewEntry(previewId);}
    catch(error){message($('#sharing-error'),error.message,true);}
  }
  function paintPeople(){const list=$('#sharing-people'),input=$('#sharing-recipient');if(!list||!input)return;list.replaceChildren();const q=input.value;
    for(const person of matchingSharePeople(data?.people||[],q)){const b=button(person.name+' · /'+person.id,()=>{input.value='/'+person.id;input.focus();list.replaceChildren();input.setAttribute('aria-expanded','false');const draft=chatDraft();if(draft)editDraft(draft,'recipient',input.value);paintSend();},'sharing-person');b.setAttribute('role','option');list.append(b);}
    const direct=normalizeShareHandle(q);if(direct&&!matchingSharePeople(data?.people||[],q).some(p=>p.id===direct)){const b=button('Send to /'+direct,()=>{input.value='/'+direct;input.focus();list.replaceChildren();input.setAttribute('aria-expanded','false');const draft=chatDraft();if(draft)editDraft(draft,'recipient',input.value);paintSend();},'sharing-person');b.setAttribute('role','option');list.append(b);}input.setAttribute('aria-expanded',String(list.childElementCount>0));
  }
  function currentJob(id){const basic=data?.jobs?.find(j=>j.id===id);if(!basic)return null;
    if(basic.status==='ready'&&['prepare','preview'].includes(basic.kind)&&!jobDetails.has(id)&&!detailsLoading.has(id)){
      detailsLoading.add(id);void sharingCommand('sharing.job',{id}).then(detail=>{jobDetails.set(id,detail);lastRendered='';paint();}).catch(error=>{jobDetails.set(id,{error:error.message});message($('#sharing-error'),error.message,true);}).finally(()=>detailsLoading.delete(id));
    }
    return {...jobDetails.get(id),...basic};
  }
  function paintSend(){const b=$('#sharing-send'),local=chatDraft();if(!b||!local)return;const draft=currentJob(local.draftId),send=currentJob(local.sendId);const sending=draftSending(local);for(const id of ['sharing-recipient','sharing-message']){const input=$('#'+id);if(input)input.disabled=sending;}const recent=$('#sharing-recent');if(recent)recent.disabled=sending||local.preparing||draft?.status==='working';b.disabled=sending||local.preparing||!(draft?.status==='ready'&&normalizeShareHandle(local.recipient)&&data?.config?.root)||send?.status==='working';b.textContent=local.sending||send?.status==='working'?'Sending…':send?.status==='failed'?'Retry send':send?.status==='ready'?'Sent':'Send';if(send?.status==='ready')b.disabled=true;}
  function summary(node,preview){node.replaceChildren();if(!preview){node.append(el('p','sharing-note','Loading handover preview…'));return;}node.append(el('strong','sharing-preview-title',preview.title||'Shared context'));if(preview.summary)node.append(el('p','sharing-summary',preview.summary));const count=preview.resourceCount??preview.resources?.length??0;if(count)node.append(el('small','sharing-note',count+' files · '+Math.round((preview.totalBytes??preview.resources.reduce((n,r)=>n+(r.size||0),0))/1024)+' KB'));
    const details=document.createElement('details'),heading=document.createElement('summary');heading.textContent='Context & files';details.append(heading);const list=el('ul','sharing-artifacts');for(const item of preview.items||[])list.append(el('li','',item.title||item.kind||'Context'));for(const resource of preview.resources||[])list.append(el('li','',(resource.name||resource.media_type||'File')+' · '+Math.round((resource.size||0)/1024)+' KB'));for(const [category,refs]of Object.entries(preview.uris||{}))for(const ref of refs)list.append(el('li','',category+' · '+(ref.title||ref.uri||'Reference')));details.append(list);node.append(details);if(preview.itemsTruncated||preview.resourcesTruncated)node.append(el('small','sharing-note','Preview shortened; all content is retained in the handover.'));
  }
  function paint(){if(openIntent&&(selectedAgentKey!==openIntent.agentKey||source!==openIntent.source))openIntent=null;if(!data)return;badge.textContent=data.unread||0;badge.hidden=!data.unread;if(!menu.open)return;
    if(!panel.childElementCount||(tab==='share'&&renderedRecordId!==(selected()?.id||''))){build();return;}const status=$('#sharing-status');if(status)status.textContent=data.refreshing?'Checking inbox…':data.lastSuccess?'Checked '+new Date(data.lastSuccess*1000).toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'}):'Cached inbox · refresh every 5 minutes';
    if(tab==='share'&&status){if(data.peopleError)status.textContent+=' · Recipient suggestions: '+data.peopleError;else if(data.peopleHasMore)status.textContent+=' · More recipient names loading…';}if(data.error)message($('#sharing-error'),data.error,true);
    if(tab==='share'){
      const local=chatDraft(),draft=currentJob(local?.draftId),send=currentJob(local?.sendId);const signature=JSON.stringify([draft?.id,draft?.status,draft?.summary,draft?.error,send?.status,send?.error]);
      if(signature!==lastRendered){lastRendered=signature;const target=$('#sharing-draft');if(draft?.status==='ready'){summary(target,draft.preview);if(draft.diagnostics?.length)target.append(el('small','sharing-note',draft.diagnostics.length+' compatibility notices retained.'));if(draft.notices?.length){const notes=document.createElement('details'),heading=document.createElement('summary');heading.textContent='Preparation notes ('+draft.notices.length+')';notes.append(heading);for(const note of draft.notices)notes.append(el('p','sharing-note',typeof note==='string'?note:JSON.stringify(note)));target.append(notes);}}else message(target,draft?.status==='working'?'Preparing with the source agent…':draft?.error||'Prepare a handover to review its content before sending.',draft?.status==='failed');if(send?.error)target.append(el('p','sharing-error',send.error));if(send?.uri&&send.status==='ready')target.append(button('Copy context URI',async()=>{try{await navigator.clipboard.writeText(send.uri);toast('Context URI copied.');}catch(_){toast(send.uri);}},'text-button'));}
      if(document.activeElement===$('#sharing-recipient'))paintPeople();const prepare=$('#sharing-prepare');if(prepare)prepare.disabled=!selected()||local?.preparing||draftSending(local)||draft?.status==='working';if(local?.error)message($('#sharing-error'),local.error,true);paintSend();
    }else{
      const entries=(data.entries||[]).filter(e=>!!e.dismissed===archive),selectedEntry=(data.entries||[]).find(e=>e.id===previewId),job=currentJob(previewJob),local=receiving(),importing=currentJob(local.jobId);
      const signature=JSON.stringify([entries,entries.map(entry=>importedSession(entry)),previewId,job?.status,job?.error,local.starting,local.opening,local.error,importing?.status,importing?.error]);
      if(signature!==lastRendered){lastRendered=signature;const list=$('#sharing-entries');list.replaceChildren();if(!entries.length)list.append(el('p','sharing-note',archive?'No archived shares.':'Your inbox is empty.'));
        for(const entry of entries){const row=el('div','sharing-entry'+(!entry.read?' unread':''));const open=button('',()=>previewEntry(entry.id),'sharing-entry-open');open.append(el('strong','',entry.title),el('small','',(entry.sender?.name||entry.sender?.id||'Unknown sender')+' · '+(entry.created_at?new Date(entry.created_at).toLocaleDateString():'Unknown date')+(entry.imported?' · Imported':'')));row.append(open,iconButton((archive?'Restore ':'Archive ')+entry.title,archive?'↶':'×',async()=>{if(entry.id===previewId)openIntent=null;try{await call('sharing.dismiss',{id:entry.id,dismissed:!archive});}catch(e){message($('#sharing-error'),e.message,true);}}));const sessionId=importedSession(entry);if(sessionId){const go=button('Open imported chat',()=>openImported(entry.id,sessionId),'text-button');go.setAttribute('aria-label','Open imported chat: '+entry.title);go.disabled=!!imports.get(entry.id)?.opening;row.append(go);}list.append(row);}
        if(data.hasMore)list.append(el('small','sharing-note','More entries are loading; this scan is incomplete.'));
        const target=$('#sharing-preview');if(job?.status==='ready')summary(target,job.preview);else message(target,selectedEntry?.error||job?.error||(job?.status==='working'?'Loading handover…':selectedEntry?.message||''),!!selectedEntry?.error||job?.status==='failed');
        if(job?.status==='failed'&&selectedEntry&&!selectedEntry.error)target.append(button('Retry preview',()=>previewEntry(previewId),'button small'));
        $('#sharing-import-options').hidden=!(job?.status==='ready');$('#sharing-agent').value=receivingAgents.get(previewId)||receivingAgents.get('')||$('#quick-agent').value;const go=$('#sharing-import');const folder=$('#sharing-folder');const suggested=locations.has(previewId)?locations.get(previewId):job?.recommendedCwd;if(suggested&&! [...folder.options].some(o=>o.value===suggested))folder.add(new Option(suggested.replace(/^\/Users\/[^/]+/,'~'),suggested));folder.value=suggested||'';go.disabled=local.starting||local.opening||importing?.status==='working'||(!importedSession(selectedEntry)&&!folder.value);go.textContent=local.opening?'Opening…':local.starting||importing?.status==='working'?'Importing…':importedSession(selectedEntry)?'Open imported chat':'Import into new chat';if(local.error||importing?.error)target.append(el('p','sharing-error',local.error||importing.error));
      }
      if(importing?.status==='ready'&&importing.sessionId&&openIntent&&!openIntent.started&&currentImportIntent(openIntent)){openIntent.started=true;void showImported(importing.sessionId,openIntent);}
    }
  }
  async function sendDraft(){
    const local=chatDraft();if(!local||local.sending||currentJob(local.sendId)?.status==='working'||local.preparing)return;
    const payload={draftId:local.draftId,recipients:[normalizeShareHandle(local.recipient)],message:local.message};if(currentJob(local.sendId)?.status==='failed')payload.retryId=local.sendId;
    local.sending=true;local.error='';paintSend();try{const result=await call('sharing.send',payload);local.sendId=result.jobId;lastRendered='';}
    catch(error){local.error=error.message;}finally{local.sending=false;paint();}
  }
  async function importEntry(){
    const deliveryId=previewId,local=receiving(deliveryId);
    if(local.starting||local.opening||currentJob(local.jobId)?.status==='working')return;
    const existing=importedSession(data.entries.find(entry=>entry.id===deliveryId));
    if(existing){await openImported(deliveryId,existing);return;}
    const args={deliveryId,agent:$('#sharing-agent').value,cwd:$('#sharing-folder').value};
    openIntent={deliveryId,agentKey:selectedAgentKey,source,started:false};local.starting=true;local.error='';lastRendered='';paint();
    try{const result=await call('sharing.import',args);local.jobId=result.jobId;}
    catch(error){local.error=error.message;}
    finally{local.starting=false;lastRendered='';paint();}
  }
  async function showImported(id,intent){
    const local=receiving(intent.deliveryId);if(local.opening||!currentImportIntent(intent))return;
    local.opening=true;local.error='';lastRendered='';paint();
    try{
      const entry=data.entries.find(entry=>entry.id===intent.deliveryId),job=currentJob(local.jobId),receipt=entry?.openReceipt||(job?.status==='ready'&&job.sessionId===id?job.openReceipt:null);
      if(receipt&&!adoptSavedSessionOpen(receipt,id))throw new Error('The imported chat open receipt is unavailable. Inspect the chat; no additional open request was sent.');
      const [snapshot,agents]=await Promise.all([loadStateSnapshot({attempts:2}),api('/api/agents')]);
      if(!currentImportIntent(intent))return;
      if(!snapshot.accepted)throw new Error('Dashboard changed while opening this chat. Your newer changes were kept. Use Open imported chat to try again; do not import again.');
      inventory=agents;let record=state.sessions.find(record=>record.id===id);
      if(!record)throw new Error('The imported chat is unavailable in the refreshed dashboard. Use Open imported chat to try again; do not import again.');
      let candidate=candidateForRecord(record);
      if(!candidate&&!data.demo){
        const result=await openSavedSession(record,null,{isCurrent:()=>currentImportIntent(intent),navigate:false,notify:false});
        if(!currentImportIntent(intent))return;
        if(result.presentation!=='ready'||!result.candidate){local.error=savedSessionOpenMessage(savedSessionOpenState(id)||{});return;}
        record=result.record;candidate=result.candidate;
      }
      if(!currentImportIntent(intent))return;
      selectAgent(record,candidate);menu.open=false;const opening=savedSessionOpenState(id);toast(data.demo?'Preview imported · no agent launched':opening?.receipt?.status==='unknown'||opening?.receipt?.localSaved===false?'Shared context imported. '+savedSessionOpenMessage(opening):'Shared context imported into a new chat');
    }catch(error){local.error=error.message;}
    finally{local.opening=false;lastRendered='';paint();}
  }
  menu.addEventListener('toggle',()=>{if(!menu.open)openIntent=null;if(menu.open){if(!data){tab='share';panel.replaceChildren(el('p','sharing-note','Loading sharing…'),el('div'));panel.lastChild.id='sharing-error';}else build();void load();}});
  document.addEventListener('pointerdown',e=>{if(menu.open&&!menu.contains(e.target))menu.open=false;});
  menu.addEventListener('keydown',e=>{if(e.key==='Escape'){e.preventDefault();menu.open=false;menu.querySelector('summary')?.focus();e.stopPropagation();}});
  setInterval(()=>{const working=data?.jobs?.some(j=>j.status==='working');const interval=menu.open||working?1500:60000;if(!document.hidden&&Date.now()-lastFetch>interval)void load();},1000);
  registerRenderHook('sharing:initial-load',()=>{if(!data)void load();else if(menu.open)paint();},{order:50});
})();
