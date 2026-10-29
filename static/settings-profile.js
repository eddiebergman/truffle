'use strict';
function settingsProfileChanges(current,proposed,approval){
 const rows=[];
 for(const [key,after]of Object.entries(proposed)){
  const before=current[key];if(JSON.stringify(before)===JSON.stringify(after))continue;
  const policy=approval?.[key]==='routine'?'routine':'user';
  if(key==='keybindings')for(const [id,chord]of Object.entries(after)){if(before[id]!==chord)rows.push({key,id,before:before[id],after:chord,policy});}
  else rows.push({key,before,after,policy});
 }
 return rows;
}
function settingsProfilePatch(baseline,rows){
 const patch={};for(const row of rows){if(row.id){if(!patch.keybindings)patch.keybindings={...baseline.keybindings};patch.keybindings[row.id]=row.after;}else patch[row.key]=structuredClone(row.after);}return patch;
}
function renderSettingsProfilePreview(container,{initialProfile,expectedRevision,expectedContentHash}={}){
 const make=(tag,text,cls)=>{const n=document.createElement(tag);if(text)n.textContent=text;if(cls)n.className=cls;return n;};
 const root=make('section',null,'settings-profile-import');root.setAttribute('aria-label','Settings profile review');for(const marker of ['data-private','data-agent-private','data-agent-context-exclude'])root.setAttribute(marker,'true');container.append(root);
 const paste=make('textarea');paste.rows=4;paste.placeholder='Paste an exported truffle-settings v1 JSON profile';paste.setAttribute('aria-label','Settings profile JSON');paste.autocomplete='off';paste.spellcheck=false;for(const marker of ['data-private','data-agent-private','data-agent-context-exclude'])paste.setAttribute(marker,'true');
 const file=make('input');file.type='file';file.accept='.json,application/json';file.hidden=true;file.setAttribute('aria-label','Choose settings profile JSON');
 const controls=make('div',null,'settings-profile-actions ui-actiongroup'),preview=make('div',null,'settings-profile-preview'),status=make('p',null,'settings-profile-status');status.setAttribute('role','status');
 controls.setAttribute('role','group');controls.setAttribute('aria-label','Settings profile actions');
 let disposed=false,busy=false,generation=0,profile=initialProfile===undefined?null:structuredClone(initialProfile),review=null;
 const actionButton=(text,run)=>{const b=make('button',text,'button ui-control');b.dataset.variant='secondary';b.type='button';b.onclick=()=>{if(!disposed&&!busy)return run();};return b;};
 const previewButton=actionButton('Preview pasted profile',async()=>{let raw=paste.value;paste.value='';await read(raw);raw='';});
 const choose=actionButton('Choose profile file',()=>file.click()),refresh=actionButton('Refresh preview',()=>validate(profile)),apply=actionButton('Apply selected changes',applySelected);apply.dataset.variant='primary';
 const update=()=>{root.setAttribute('aria-busy',String(busy));paste.disabled=file.disabled=choose.disabled=previewButton.disabled=busy||disposed;refresh.disabled=busy||disposed||!profile;refresh.hidden=!profile;apply.disabled=busy||disposed||!review||review.stale||!review.rows.some(row=>row.checkbox.checked);for(const row of review?.rows||[])row.checkbox.disabled=busy||disposed||review.stale;};
 const clear=()=>{review=null;preview.replaceChildren();};
 const fail=message=>{status.textContent=message;};
 async function read(raw){
  clear();profile=null;update();
  try{if(new TextEncoder().encode(raw).length>65536)throw new Error('Profile exceeds 64 KiB.');try{profile=JSON.parse(raw);}catch{throw new Error('Enter valid JSON from an exported settings profile.');}raw='';await validate(profile);}catch(error){if(!disposed){profile=null;fail(error.message||'Profile could not be previewed.');update();}}
 }
 async function validate(candidate,{expectedRevision,expectedContentHash}={}){
  if(disposed)return;const ticket=++generation;busy=true;clear();status.textContent='Validating settings profile…';update();
  try{
   if(new TextEncoder().encode(JSON.stringify(candidate)).length>65536)throw new Error('Profile exceeds 64 KiB.');
   const current=await api('/api/settings');if(disposed||ticket!==generation)return;
   if(expectedRevision!==undefined&&(current.revision!==expectedRevision||current.contentHash!==expectedContentHash))throw new Error('Settings changed since this proposal. Refresh the preview and choose changes again.');
   const merged=mergeSettingsProfile(current.values,candidate),result=await api('/api/settings/preview',{revision:current.revision,contentHash:current.contentHash,patch:merged});if(disposed||ticket!==generation)return;
   if(result.revision!==current.revision||result.contentHash!==current.contentHash)throw new Error('Settings changed. Refresh the preview before applying.');
   const rows=settingsProfileChanges(current.values,result.values,current.approval);review={revision:current.revision,contentHash:current.contentHash,baseline:current.values,rows,stale:false};
   if(!rows.length){status.textContent='This profile matches your current settings. Nothing to apply.';return;}
   preview.append(make('p','Choose which changes to apply. Command, editor and review changes require your approval.','form-hint'));
   for(const row of rows){
    const label=row.id?((current.commands||[]).find(command=>command.id===row.id)?.label||row.id)+' shortcut':(current.fields||[]).find(field=>field.key===row.key)?.label||row.key;
    const card=make('div',null,'settings-profile-change'),heading=make('label',null,'settings-profile-choice'),checkbox=make('input');checkbox.type='checkbox';checkbox.checked=row.policy==='routine';checkbox.setAttribute('aria-label',(row.policy==='user'?'Approve changing ':'Apply ')+label);checkbox.onchange=update;row.checkbox=checkbox;
    heading.append(checkbox,make('strong',label),make('small',row.policy==='user'?'Approval required':'Preference'));
    const diff=make('div',null,'settings-profile-diff'),format=value=>typeof value==='string'?(value||'Unassigned'):JSON.stringify(value,null,2);const before=make('div'),after=make('div');before.append(make('small','Current'),make('pre',format(row.before)));after.append(make('small','Proposed'),make('pre',format(row.after)));diff.append(before,after);card.append(heading,diff);preview.append(card);
   }
   status.textContent=rows.length+' change'+(rows.length===1?'':'s')+' previewed. Nothing has been applied.';
  }catch(error){if(!disposed&&ticket===generation)fail(error.message||'Profile could not be previewed.');}
  finally{if(!disposed&&ticket===generation){busy=false;update();}}
 }
 async function applySelected(){
  if(!review||review.stale||disposed)return;const selected=review.rows.filter(row=>row.checkbox.checked);if(!selected.length)return;
  const expected=review,ticket=++generation;busy=true;status.textContent='Checking current settings…';update();
  try{
   const current=await api('/api/settings');if(disposed||ticket!==generation)return;
   if(current.revision!==expected.revision||current.contentHash!==expected.contentHash){expected.stale=true;throw new Error('Settings changed elsewhere. Refresh the preview and choose changes again.');}
   const patch=settingsProfilePatch(expected.baseline,selected);await api('/api/settings',{revision:expected.revision,contentHash:expected.contentHash,patch});if(disposed||ticket!==generation)return;
   clear();profile=null;status.textContent='Selected settings applied.';await loadAppSettings();
  }catch(error){if(!disposed&&ticket===generation){if(review)review.stale=true;fail(error.message||'Settings could not be applied. Refresh the preview to retry.');}}
  finally{if(!disposed&&ticket===generation){busy=false;update();}}
 }
 file.onchange=async()=>{const selected=file.files?.[0];file.value='';paste.value='';if(!selected||disposed)return;if(selected.size>65536){profile=null;clear();status.textContent='Profile exceeds 64 KiB.';update();return;}const ticket=++generation;busy=true;update();try{const raw=await selected.text();if(!disposed&&ticket===generation){busy=false;await read(raw);}}catch{if(!disposed&&ticket===generation){busy=false;status.textContent='The profile file could not be read.';update();}}};
 const dialog=container.closest('dialog'),onClose=()=>root.disposeProfileImport();
 root.disposeProfileImport=()=>{if(disposed)return;disposed=true;generation++;paste.value='';file.value='';profile=null;clear();dialog?.removeEventListener('close',onClose);update();};dialog?.addEventListener('close',onClose,{once:true});
 controls.append(previewButton,choose,refresh,apply);root.append(make('p',initialProfile===undefined?'Import a settings profile exported by truffle. Freeform setup instructions, credentials and sharing access are configured separately. Do not paste secrets.':'Review the proposed settings. Quick actions replace the existing action list. Nothing applies until you choose Apply selected changes.','form-hint'),paste,file,controls,preview,status);paste.hidden=previewButton.hidden=choose.hidden=initialProfile!==undefined;update();root.ready=initialProfile===undefined?Promise.resolve(root):validate(profile,{expectedRevision,expectedContentHash}).then(()=>root);return root;
}
