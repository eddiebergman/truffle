'use strict';
// These two forms edit preset creation and schedule metadata, never run a skill.
function skillEditorMutable(current=featureForms.get($('#skill-editor'))){return featureFormMutable(current)&&!current.loading&&!current.conflict;}
function skillScheduleMutable(current){return featureFormMutable(current)&&!current.loading&&!current.loadFailed&&!current.conflict;}
function openSkillEditor({skill=null,source='installed',cwd='',onSaved=null,returnLabel='Close and inspect Skills'}={}){
 const dialog=$('#skill-editor'),form=$('#skill-editor-form');if(!requestDialogNavigation())return false;disposeFeatureForm(dialog);form.reset();
 const agent=skill?(skill.agents.includes(setting('defaultAgent','claude'))?setting('defaultAgent','claude'):skill.agents[0]):setting('defaultAgent','claude');
 form.elements.agent.value=agent;form.elements.cwd.value=cwd;
 if(skill){form.elements.title.value=skill.name;form.elements.skill.value=skill.path;form.elements.command.value=(agent==='codex'?'$':'/')+(skill.commandName||skill.name);if(form.elements.description)form.elements.description.value=skill.description||'';}
 $('#skill-editor-error').hidden=true;dialog.dataset.skillSource=skill?'installed':source;
 const current=createFeatureForm(dialog,form,null);current.kind='preset';current.onSaved=onSaved;current.acceptedText='Skill saved.';current.ui.inspect.textContent=returnLabel;current.ui.refresh.onclick=()=>refreshSkillEditor(current);dialog.showModal();return current;
}
function updateScheduleFields(){const form=$('#schedule-form');$('#schedule-time').hidden=form.elements.mode.value!=='daily';$('#schedule-interval').hidden=form.elements.mode.value!=='interval';}
function paintSkillSchedule(current,metadata){
 const form=current.host,schedule=metadata.schedule;current.baseline=metadata;
 form.elements.presetId.value=current.target;for(const field of ['mode','time','everyMinutes','missed'])form.elements[field].value=schedule[field];form.elements.enabled.checked=schedule.enabled;updateScheduleFields();
}
async function readSkillSchedule(current){
 if(!current.isCurrent()||current.loading||current.pending)return;current.loading=true;current.loadFailed=false;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.message.textContent='Loading current schedule…';current.ui.refresh.disabled=true;
 try{const metadata=await api('/api/commands/invoke',{name:'skills.preset.read',args:{presetId:current.target}});if(!current.isCurrent())return;if(metadata.presetId!==current.target||!metadata.schedule||!metadata.scheduleExpected)throw new Error('Current schedule is unavailable.');paintSkillSchedule(current,metadata);current.result=null;current.conflict=false;current.ui.message.textContent='';current.ui.refresh.hidden=true;}
 catch(error){if(current.isCurrent()){current.loadFailed=true;current.ui.message.textContent=error.message;current.ui.refresh.textContent='Reload schedule';current.ui.refresh.hidden=false;current.ui.refresh.onclick=()=>readSkillSchedule(current);}}
 finally{current.loading=false;if(current.isCurrent()){freezeFeatureForm(current,current.loadFailed);current.ui.refresh.disabled=false;restoreFormFocus(current,current.loadFailed?current.ui.refresh:current.host.elements.mode);}}
}
function openSkillSchedule(preset){
 const dialog=$('#schedule-editor'),form=$('#schedule-form');if(!requestDialogNavigation())return false;disposeFeatureForm(dialog);form.reset();$('#schedule-title').textContent=preset.title+' schedule';$('#schedule-error').hidden=true;
 const current=createFeatureForm(dialog,form,preset.id);current.kind='schedule';current.acceptedText='Schedule saved.';current.ui.refresh.onclick=()=>refreshSkillEditor(current);dialog.showModal();return readSkillSchedule(current);
}
async function refreshSkillEditor(current){
 if(!current.isCurrent()||current.pending||current.refreshing||!['accepted','unknown'].includes(current.result?.kind))return;current.refreshing=true;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.refresh.disabled=true;
 try{const reads=[loadStateSnapshot({attempts:2})];if(current.kind==='preset')reads.push(loadArtifactLibrary(true));const results=await Promise.all(reads);if(results.some(result=>!result.accepted))throw new Error('The Skills view could not refresh.');if(!current.isCurrent())return;render();
  if(current.result.kind==='accepted'){current.dialog.close();toast(current.acceptedText);}else current.ui.message.textContent='The save outcome is unknown. Skills were refreshed. Inspect them before saving again; this draft was not resubmitted.';
 }catch(_){if(current.isCurrent())current.ui.message.textContent=localWriteMessage(current.result,current.acceptedText);}
 finally{current.refreshing=false;if(current.isCurrent()){freezeFeatureForm(current,true);current.ui.refresh.disabled=false;restoreFormFocus(current,current.result.kind==='unknown'?current.ui.inspect:current.ui.refresh);}}
}
async function saveSkillEditor(current,name,args){
 current.pending=true;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.message.textContent='Saving…';let receipt,invalidated=false;const invalidateLibrary=()=>{if(current.kind==='preset'&&!invalidated){libraryGeneration++;libraryFetched=0;invalidated=true;}};
 const result=await presentLocalWrite(async()=>{try{receipt=await api('/api/commands/invoke',{name,args});invalidateLibrary();if(receipt.saved!==true||typeof receipt.presetId!=='string'||!receipt.presetId)throw new Error('The save identity could not be confirmed.');}
  catch(error){current.rejectedStatus=error.status;if(![400,401,403,404,409].includes(error.status))invalidateLibrary();throw error;}});
 current.result=result;if(result.kind==='accepted'&&current.kind==='preset'){if(current.isCurrent()){current.acceptedPresetId=receipt.presetId;try{current.onSaved?.(receipt.presetId);}catch(error){current.result={kind:'accepted',presentation:'pending',error:error.message};}}try{const library=await loadArtifactLibrary(true);if(!library.accepted)throw new Error('The Skills library could not refresh.');}catch(error){current.result={kind:'accepted',presentation:'pending',error:error.message};}}
 current.pending=false;if(!current.isCurrent()){if(featureForms.get(current.dialog)===current&&current.dialog.open)freezeFeatureForm(current,true);toast(localWriteMessage(current.result,current.acceptedText));return;}
 if(current.result.kind==='accepted'&&current.result.presentation==='ready'){current.dialog.close();toast(current.acceptedText);return;}
 current.ui.refresh.textContent='Refresh view';current.ui.refresh.onclick=()=>refreshSkillEditor(current);
 if(current.result.kind==='rejected'&&current.rejectedStatus===409&&current.kind==='schedule'){current.conflict=true;freezeFeatureForm(current,false);current.ui.message.textContent=current.result.error+' Your draft remains. Reload schedule replaces it with the current schedule before saving.';current.ui.refresh.textContent='Reload schedule';current.ui.refresh.hidden=false;current.ui.refresh.onclick=()=>readSkillSchedule(current);current.host.querySelector('button[type="submit"]').disabled=true;restoreFormFocus(current,current.ui.refresh);}
 else presentFeatureFormResult(current);
}
$('#skill-editor-form').onsubmit=async event=>{
 event.preventDefault();const current=featureForms.get($('#skill-editor'));if(!skillEditorMutable(current))return;const args={};for(const field of ['title','cwd','skill','command','agent','description'])if(current.host.elements[field])args[field]=current.host.elements[field].value;
 await saveSkillEditor(current,'skills.save',args);
};
$('#schedule-form').elements.mode.onchange=updateScheduleFields;
$('#schedule-form').onsubmit=async event=>{
 event.preventDefault();const current=featureForms.get($('#schedule-editor'));if(!skillScheduleMutable(current))return;const form=current.host,values={mode:form.elements.mode.value,time:form.elements.time.value,everyMinutes:Number(form.elements.everyMinutes.value),missed:form.elements.missed.value,enabled:form.elements.enabled.checked},patch={},expected={};
 for(const field of Object.keys(values))if(values[field]!==current.baseline.schedule[field]){patch[field]=values[field];expected[field]=current.baseline.scheduleExpected[field];}
 if(!Object.keys(patch).length){current.dialog.close();return;}await saveSkillEditor(current,'skills.schedule',{presetId:current.target,patch,expected});
};
