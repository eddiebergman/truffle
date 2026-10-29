'use strict';
let artifactLibrary=null,libraryLoading=false,libraryFetched=0,libraryError='',newsletterOpening=false;
let libraryFlight=null,libraryGeneration=0,libraryRetryAfter=0;
function loadArtifactLibrary(force=false){
  if(force)libraryGeneration++;
  if(libraryFlight)return libraryFlight;
  if(!force&&artifactLibrary&&!libraryError&&Date.now()-libraryFetched<15000)return Promise.resolve({accepted:true});
  if(!force&&Date.now()<libraryRetryAfter)return Promise.resolve({accepted:false,error:libraryError});
  libraryLoading=true;
  libraryFlight=(async()=>{
    let failure='The library changed while refreshing. Refresh the library again.';
    try{
      // One shared followup handles invalidation during an older in-flight read.
      for(let attempt=0;attempt<2;attempt++){
        const generation=libraryGeneration;
        try{
          const next=await api('/api/library');
          if(generation!==libraryGeneration)continue;
          artifactLibrary=next;libraryFetched=Date.now();libraryError='';libraryRetryAfter=0;
          return {accepted:true};
        }catch(error){
          if(generation!==libraryGeneration)continue;
          failure=error.message;break;
        }
      }
      libraryError=failure;libraryRetryAfter=Date.now()+15000;
      return {accepted:false,error:failure};
    }finally{
      libraryLoading=false;libraryFlight=null;
      if(['agents','skills','newsletters','artifacts'].includes(source))render();
      if(selectedSession&&$('#session-dialog').open)renderSessionArtifacts(selectedSession);
    }
  })();
  return libraryFlight;
}
function isSkillOutput(artifact){return artifact?.role==='skill-output'&&typeof artifact.presetId==='string'&&artifact.presetId.length>0;}
function skillInboxCounts(current){
 const read=current?.skillsReadAt||0;
 const outputs=(current?.artifacts||[]).filter(artifact=>isSkillOutput(artifact)&&artifact.at>read).length;
 const notices=(current?.skillRuns||[]).filter(run=>run.at>read&&['failed','skipped'].includes(run.outcome)).length;
 return {outputs,notices,total:outputs+notices};
}
function artifactLink(artifact,label){
  const link=el('a','text-button',label||'Open ↗');link.href=artifact.url||serviceClient.artifactUrl(artifact.id);link.target='_blank';link.rel='noopener noreferrer';return link;
}
async function startNewsletter(preset,button){
 if(newsletterOpening)return;newsletterOpening=true;
 try{await prepareSavedChat('skill:'+preset.id,'skill',{presetId:preset.id},{button});}catch(error){toast(error.message);}finally{newsletterOpening=false;}
}
function libraryRow(artifact){
  const row=el('article','artifact-row');
  const date=el('time','artifact-time',new Date(artifact.at*1000).toLocaleDateString(undefined,{month:'short',day:'numeric'}));date.title=dated(artifact.at);
  const type=el('span','artifact-format',artifact.format);
  const title=el('button','artifact-title',artifact.title);title.title=artifact.path||artifact.url;title.disabled=!artifact.available;title.onclick=()=>viewArtifact(artifact);
  const associations=el('div','artifact-sessions');
  for(const id of artifact.sessionIds||[]){const session=state.sessions.find(x=>x.id===id);if(session)associations.append(featureButton(session.title,()=>showSession(session),'artifact-session-link'));}
  if(!associations.children.length)associations.append(el('span','form-hint',artifact.available?'No session linked':'File unavailable'));
  const attach=featureButton('⇄',()=>editArtifact(artifact),'icon-button');attach.title='Link artifact to sessions';attach.setAttribute('aria-label','Link sessions: '+artifact.title);
  row.append(date,type,title,associations,attach);return row;
}
function renderNewsletterView(list,controls,query){
  controls.append(featureButton('↻ Refresh library',()=>loadArtifactLibrary(true),'text-button'),featureButton('＋ Add artifact',()=>editArtifact(),'text-button'));
  if(!artifactLibrary){list.append(el('p','feature-empty',libraryError||'Loading local reports…'));void loadArtifactLibrary();return;}
  if(libraryError)list.append(el('p','form-hint',libraryError));
  for(const warning of artifactLibrary.warnings||[])list.append(el('p','form-hint',warning));
  if(source==='newsletters'){
    controls.append(el('p','form-hint','One click starts an agent in the saved folder. Reports stay local unless you request delivery.'));
    for(const preset of artifactLibrary.presets.filter(x=>[x.title,x.description,x.command].join(' ').toLowerCase().includes(query))){
      const card=el('article','newsletter-card'),head=el('div','newsletter-heading');
      head.append(el('h2','',preset.title));const run=featureButton(newsletterOpening?'Opening…':'▶ Run '+preset.command,()=>startNewsletter(preset,run),'button primary');run.disabled=!preset.available||newsletterOpening;head.append(run);card.append(head,el('p','form-hint',preset.description));
      card.append(el('p','newsletter-context',`${agentName(preset.agent)} · ${preset.cwd.replace(/^\/Users\/[^/]+/,'~')}`));
      if(!preset.available)card.append(el('p','form-hint','Skill or folder unavailable. Update data/newsletters.json.'));
      const sessions=state.sessions.filter(x=>x.newsletterId===preset.id).sort((a,b)=>b.createdAt-a.createdAt).slice(0,5);
      for(const session of sessions){const entry=el('div','newsletter-run');entry.append(el('span','form-hint',dated(session.createdAt)),featureButton(session.title,()=>showSession(session),'text-button'),featureButton(session.launchNew?'Start ↗':'Continue ↗',()=>openSavedSession(session),'text-button'));card.append(entry);}
      const outputs=artifactLibrary.artifacts.filter(artifact=>isSkillOutput(artifact)&&artifact.presetId===preset.id).sort((a,b)=>b.at-a.at).slice(0,5);
      if(outputs.length){card.append(el('h3','group-header','Recent results'));outputs.forEach(x=>card.append(libraryRow(x)));}else card.append(el('p','form-hint','No reports yet. The agent will register its completed outputs.'));
      list.append(card);
    }
    list.append(el('p','form-hint','Add predefined workflows in data/newsletters.json. Each preset specifies a skill, folder, agent, and reports directory.'));
  }else{
    const filter=el('select');filter.setAttribute('aria-label','Filter artifacts by session');filter.append(new Option('All artifacts',''),new Option('Unlinked artifacts','__unlinked__'),...state.sessions.map(x=>new Option(x.title,x.id)));filter.value=artifactSessionFilter;
    filter.onchange=()=>{artifactSessionFilter=filter.value;render();};controls.append(filter);
    const rows=artifactLibrary.artifacts.filter(x=>[x.title,x.path,x.url,...(x.sessionIds||[]).map(id=>state.sessions.find(s=>s.id===id)?.title||'')].join(' ').toLowerCase().includes(query)).filter(x=>artifactSessionFilter==='__unlinked__'?!(x.sessionIds||[]).length:!artifactSessionFilter||(x.sessionIds||[]).includes(artifactSessionFilter));
    if(rows.length)rows.forEach(x=>list.append(libraryRow(x)));else list.append(el('p','feature-empty','No matching artifacts. Add a local file or an existing artifact URL.'));
    list.append(el('p','form-hint','Local reports are discovered from configured newsletter folders. Other files and existing Claude Artifact links can be added above.'));
  }
  void loadArtifactLibrary();
}
let artifactSessionFilter='';
function artifactEditorMutable(current){return featureFormMutable(current)&&!current.loading&&!current.conflict&&!current.loadFailed;}
function paintArtifactEditor(current,metadata=null){
  const form=current.host;form.elements.title.value=metadata?.artifact.title||'';form.elements.target.value=current.target?current.displayTarget:'';
  const picker=$('#artifact-session-picker');picker.replaceChildren();current.visibleIds=state.sessions.map(session=>session.id);
  for(const session of state.sessions){const label=el('label','artifact-session-choice'),box=el('input');box.type='checkbox';box.name='sessionIds';box.value=session.id;box.checked=(metadata?.artifact.sessionIds||[]).includes(session.id)||(!current.target&&session.id===current.initialSessionId);label.append(box,document.createTextNode(`${session.title}${session.archivedAt?' · archived':''}`));picker.append(label);}
}
async function readArtifactEditor(current){
  if(!current.isCurrent()||current.loading||current.pending)return;current.loading=true;current.loadFailed=false;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.message.textContent='Loading artifact metadata…';current.ui.refresh.disabled=true;
  try{const metadata=await api('/api/commands/invoke',{name:'artifacts.metadata.read',args:{id:current.target}});if(!current.isCurrent())return;
    if(metadata.artifact?.id!==current.target||!Array.isArray(metadata.artifact.sessionIds)||!metadata.expected||!['registered','discovered'].includes(metadata.origin?.type))throw new Error('Artifact metadata is unavailable.');
    current.baseline=metadata;current.conflict=false;current.result=null;paintArtifactEditor(current,metadata);current.ui.message.textContent=metadata.origin.type==='discovered'?'Discovered artifact. Save explicitly registers its metadata locally.':'';current.ui.refresh.hidden=true;
  }catch(error){if(current.isCurrent()){current.loadFailed=true;current.ui.message.textContent=error.message;current.ui.refresh.textContent='Reload artifact';current.ui.refresh.hidden=false;current.ui.refresh.onclick=()=>readArtifactEditor(current);}}
  finally{current.loading=false;if(current.isCurrent()){freezeFeatureForm(current,current.loadFailed);current.ui.refresh.disabled=false;restoreFormFocus(current,current.loadFailed?current.ui.refresh:current.host.elements.title);}}
}
function editArtifact(artifact=null,session=null){
  const dialog=$('#artifact-editor'),form=$('#artifact-form');if(!requestDialogNavigation())return false;disposeFeatureForm(dialog);form.reset();form.elements.target.disabled=!!artifact;
  $('#artifact-editor-title').textContent=artifact?'Artifact & sessions':'Add artifact';$('#artifact-error').hidden=true;
  const current=createFeatureForm(dialog,form,artifact?.id||null);current.displayTarget=artifact?.path||artifact?.url||'';current.initialSessionId=session?.id||null;current.baseline=null;current.loading=false;current.loadFailed=false;current.conflict=false;
  current.ui.refresh.onclick=()=>refreshArtifactEditor(current);paintArtifactEditor(current);dialog.showModal();if(current.target)return readArtifactEditor(current);
}
async function refreshArtifactEditor(current){
  if(!current.isCurrent()||current.pending||current.refreshing||!['accepted','unknown'].includes(current.result?.kind))return;current.refreshing=true;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.refresh.disabled=true;
  try{const [snapshot,library]=await Promise.all([loadStateSnapshot({attempts:2}),loadArtifactLibrary(true)]);if(!snapshot.accepted||!library.accepted)throw new Error('The artifact view could not refresh.');if(!current.isCurrent())return;render();
    if(current.result.kind==='accepted'){current.dialog.close();toast('Artifact saved.');}else current.ui.message.textContent='The save outcome is unknown. The view and library were refreshed. Inspect them before saving again; this draft was not resubmitted.';
  }catch(_){if(current.isCurrent())current.ui.message.textContent=localWriteMessage(current.result,'Artifact saved.');}
  finally{current.refreshing=false;if(current.isCurrent()){freezeFeatureForm(current,true);current.ui.refresh.disabled=false;restoreFormFocus(current,current.result.kind==='unknown'?current.ui.inspect:current.ui.refresh);}}
}
$('#artifact-form').onsubmit=async event=>{
  event.preventDefault();const current=featureForms.get($('#artifact-editor'));if(!artifactEditorMutable(current))return;const form=current.host,title=form.elements.title.value,checked=[...$('#artifact-session-picker').querySelectorAll('input')].filter(box=>box.checked).map(box=>box.value);
  const ids=[...new Set([...(current.baseline?.artifact.sessionIds||[]).filter(id=>!current.visibleIds.includes(id)),...checked])].sort();let name,args;
  if(current.target){const baseline=current.baseline;if(!baseline)return;const patch={},expected={};if(title.trim()&&title!==baseline.artifact.title)patch.title=title;if(JSON.stringify(ids)!==JSON.stringify([...baseline.artifact.sessionIds].sort()))patch.sessionIds=ids;
    const discovered=baseline.origin.type==='discovered';if(!Object.keys(patch).length){if(!discovered){current.dialog.close();return;}patch.sessionIds=ids;}
    for(const field of Object.keys(patch))expected[field]=baseline.expected[field];name=discovered?'artifacts.register-discovered':'artifacts.metadata.update';args={id:current.target,patch,expected};
  }else{const target=form.elements.target.value.trim();args={action:'save-artifact',title,sessionIds:ids,replaceSessions:true,[target.startsWith('https://')?'url':'path']:target};}
  current.pending=true;holdFormFocus(form,current);freezeFeatureForm(current,true);current.ui.message.textContent='Saving artifact…';current.acceptedText='Artifact saved.';
  const result=await presentLocalWrite(async()=>{try{await api(name?'/api/commands/invoke':'/api/edit',name?{name,args}:args);libraryGeneration++;libraryFetched=0;}catch(error){current.rejectedStatus=error.status;if(![400,401,403,404,409].includes(error.status)){libraryGeneration++;libraryFetched=0;}throw error;}});current.result=result;
  if(result.kind==='accepted'){try{const library=await loadArtifactLibrary(true);if(!library.accepted)throw new Error('The artifact library could not refresh.');}catch(error){current.result={kind:'accepted',presentation:'pending',error:error.message};}}
  current.pending=false;if(!current.isCurrent()){if(featureForms.get(current.dialog)===current&&current.dialog.open)freezeFeatureForm(current,true);toast(localWriteMessage(current.result,'Artifact saved.'));return;}
  if(current.result.kind==='accepted'&&current.result.presentation==='ready'){current.dialog.close();toast('Artifact saved.');return;}
  current.ui.refresh.textContent='Refresh view';current.ui.refresh.onclick=()=>refreshArtifactEditor(current);
  if(current.result.kind==='rejected'&&current.rejectedStatus===409&&current.target){
    // A rejected checked write retains the draft and its captured baseline. A
    // deliberate reload replaces that draft rather than silently rebasing it.
    current.conflict=true;freezeFeatureForm(current,false);current.ui.message.textContent=current.result.error+' Your draft remains. Reload artifact replaces this draft with current metadata before saving.';current.ui.refresh.textContent='Reload artifact';current.ui.refresh.hidden=false;current.ui.refresh.onclick=()=>readArtifactEditor(current);form.querySelector('button[type="submit"]').disabled=true;restoreFormFocus(current,current.ui.refresh);
  }else presentFeatureFormResult(current);
};
function artifactCopyButton(label,action){
  const button=featureButton(label,async()=>{button.disabled=true;try{await action();toast('Copied to clipboard.');}catch(error){toast(String(error.message||error));}finally{button.disabled=false;}},'text-button');return button;
}
let artifactPreviewGeneration=0;
async function viewArtifact(artifact){
  if(!requestDialogNavigation())return false;const generation=++artifactPreviewGeneration;
  $('#artifact-viewer-title').textContent=artifact.title;
  const content=$('#artifact-preview');content.replaceChildren();content.scrollTop=0;
  const actions=$('#artifact-viewer-actions');actions.replaceChildren(artifactLink(artifact,artifact.url?'Open original ↗':serviceClient.isDesktop?'Open in browser ↗':'Open in tab ↗'),featureButton('Link sessions',()=>editArtifact(artifact),'text-button'));
  const copies=el('div','artifact-copy-controls');actions.append(copies);
  const locations=el('div','artifact-locations');actions.append(locations);
  const path=(label,value)=>{const row=el('div','artifact-path'),input=el('input');input.value=value;input.readOnly=true;input.setAttribute('aria-label',label);input.title=value;input.onclick=()=>input.select();const copy=artifactCopyButton('Copy '+(label==='URL'?'URL':'path'),()=>serviceClient.copyText(value));copy.setAttribute('aria-label',label==='URL'?'Copy artifact URL':'Copy '+label.toLowerCase()+' path');row.append(el('small','',label),input,copy);locations.append(row);};
  if(artifact.url)path('URL',artifact.url);
  else{if(artifact.sourcePath&&artifact.sourcePath!==artifact.path)path('Source',artifact.sourcePath);if(artifact.path)path(artifact.snapshot?'Saved copy':'Path',artifact.path);}
  $('#artifact-viewer').showModal();
  if(artifact.url){content.append(el('p','form-hint','This artifact is hosted externally. Open the original to view it with your existing account.'));return;}
  const isImage=['png','jpg','jpeg','webp'].includes(artifact.format);
  const isText=['md','txt','csv','json','py','html'].includes(artifact.format);
  let textPromise=null;
  const raw=()=>textPromise||(textPromise=serviceClient.readArtifact(artifact.id));
  if(isText){
    const table=['csv','json'].includes(artifact.format);
    if(table){
      const rows=async()=>{const text=await raw();if(artifact.format==='csv')return parseCSV(text);const data=JSON.parse(text);if(data.schema!=='mydash.table/v1'||!Array.isArray(data.columns)||!Array.isArray(data.rows))throw new Error('Unsupported table schema.');return [data.columns,...data.rows];};
      copies.append(artifactCopyButton('Copy CSV',()=>serviceClient.copyText(artifact.format==='csv'?raw():rows().then(artifactCSV))),artifactCopyButton('Copy Markdown',()=>serviceClient.copyText(rows().then(artifactMarkdown))));
    }else copies.append(artifactCopyButton(artifact.format==='md'?'Copy Markdown':artifact.format==='html'?'Copy HTML':'Copy text',()=>serviceClient.copyText(raw())));
  }
  if(isImage)copies.append(artifactCopyButton('Copy image',()=>serviceClient.copyArtifactImage(artifact.id)));
  if(['md','txt','csv','json','py'].includes(artifact.format)){
    try{const body=await raw();if(generation!==artifactPreviewGeneration||!$('#artifact-viewer').open||$('#artifact-preview')!==content||!content.isConnected||$('#artifact-viewer-title').textContent!==artifact.title)return;const node=el(artifact.format==='md'?'div':'pre',artifact.format==='py'?'briefing-body artifact-code':'briefing-body');if(artifact.format==='md')appendReportText(node,body);else if(['csv','json'].includes(artifact.format))appendResultTable(node,body,artifact.format);else node.textContent=body;content.append(node);}catch(error){if(generation===artifactPreviewGeneration&&$('#artifact-viewer').open)content.append(el('p','form-hint',error.message));}
  }else if(isImage){const image=el('img','artifact-image');image.alt=artifact.title;image.src=serviceClient.artifactUrl(artifact.id);content.append(image);}
  else if(artifact.format==='pdf'){content.append(el('p','form-hint','Open this PDF in a new tab to use the browser PDF viewer.'));}
  else{const frame=el('iframe','artifact-frame');frame.title=artifact.title;frame.setAttribute('sandbox','allow-popups allow-popups-to-escape-sandbox');frame.referrerPolicy='no-referrer';frame.src=serviceClient.artifactUrl(artifact.id);content.append(frame);}
}
function renderSessionArtifacts(session){
  const section=$('#session-artifacts');section.replaceChildren();const heading=el('div','section-heading');heading.append(el('h3','','Artifacts'),featureButton('＋ Attach',()=>editArtifact(null,session),'text-button'));section.append(heading);
  const rows=(artifactLibrary?.artifacts||[]).filter(x=>(x.sessionIds||[]).includes(session.id));rows.forEach(x=>section.append(libraryRow(x)));
  if(!rows.length)section.append(el('p','form-hint',libraryLoading?'Loading artifacts…':'No artifacts linked. Attach an existing file or artifact URL.'));
}
