'use strict';
function onboardingNextStep(step, delta=1){const steps=['assistant','theme','editor','keymap','integrations','skills','review','company','finish'];return steps[Math.max(0,Math.min(steps.length-1,steps.indexOf(step)+delta))];}
function onboardingSupportedEditors(editors){return editors.filter(editor=>editor.installed&&editor.diffSupported);}
function onboardingSkillAgent(skill,preferred){return skill.agents.includes(preferred)?preferred:skill.agents[0];}
function onboardingReviewPatch(skill,agent){
 if(!skill?.available||!['claude','codex'].includes(agent)||!skill.commands?.[agent])throw new Error('The portable review skill is unavailable for this assistant.');
 return {[agent==='codex'?'codexReviewCommand':'claudeReviewCommand']:skill.commands[agent]};
}
function onboardingKeymapPreset(current){
 const basics={commands:'Primary+Shift+P','find-work':'Primary+P','toggle-left':'Primary+B'},bindings={...current,...basics},displaced=[];
 for(const [id,value]of Object.entries(current))if(!Object.hasOwn(basics,id)&&value&&Object.values(basics).some(chord=>chord.toUpperCase()===value.toUpperCase())){bindings[id]='';displaced.push(id);}
 return {bindings,displaced};
}
function onboardingCompanyMessage(instructions){
 const guide=instructions.trim();if(!guide)throw new Error('Paste your team instructions first.');
 return 'Help me adapt these team setup instructions for Truffle. Explain the steps and propose changes for review. Review commands and sharing access in Settings; credentials belong in Connections. Do not apply settings or run commands automatically.\n\nTeam instructions:\n'+guide;
}
(function(){
 let documentState,dialog,content,error,skillIntent='on-demand',savedPreset=null,acceptedPresetId=null,pendingImport=false,continueAction=null,actionPending=false,paintGeneration=0,host=null,shell=null,active=false,openGeneration=0,returnGeneration=0,resume=null,actionSerial=0;
 const labels={assistant:'Your assistant',theme:'Make yourself at home',editor:'Diff editor',keymap:'Familiar shortcuts',integrations:'Connect your work',skills:'Add your installed skills',review:'Your review workflow',company:'Company setup',finish:'Ready when you are'};
 const node=(tag,text,cls)=>{const n=document.createElement(tag);if(text)n.textContent=text;if(cls)n.className=cls;return n;};
 function onboardingRoot(){return host||dialog;}
 function onboardingIsOpen(){return Boolean(active&&shell?.parentElement===onboardingRoot()&&onboardingRoot()?.isConnected&&(host?!host.hidden:dialog.open));}
 function disposeStep(){
  for(const setup of content?.querySelectorAll('.onboarding-company-instructions')||[])setup.disposeCompanySetup?.();
  for(const importer of content?.querySelectorAll('.settings-profile-import')||[])importer.disposeProfileImport?.();
  for(const setup of content?.querySelectorAll('.connection-setup')||[])void setup.disposeConnectionSetup?.();
 }
 function closeOnboarding({host:requestedHost,forChild=false}={}){
  if(requestedHost!==undefined&&requestedHost!==host&&requestedHost!==resume?.host)return false;
  if(forChild)resume={host,ticket:++returnGeneration};else{resume=null;returnGeneration++;}
  active=false;openGeneration++;paintGeneration++;if(!forChild){actionSerial++;actionPending=false;}disposeStep();
  if(host)host.hidden=true;else if(dialog?.open)dialog.close();return true;
 }
 function showSurface(){if(host){host.hidden=false;host.append(shell);}else{dialog.append(shell);if(!dialog.open)dialog.showModal();}active=true;setBusy(actionPending);}
 function ensureRenderer(){
  if(shell)return;dialog=node('dialog',null,'onboarding-dialog');dialog.id='onboarding-dialog';
  shell=node('section',null,'onboarding-shell');const heading=node('header',null,'onboarding-heading');heading.append(node('h2','Set up truffle'));const count=node('small');count.id='onboarding-step-count';
  const close=button('Close',()=>closeOnboarding());close.dataset.onboardingClose='true';heading.append(count,close);content=node('main',null,'onboarding-content');error=node('p',null,'onboarding-error');error.setAttribute('role','alert');
  const footer=node('footer',null,'onboarding-footer'),back=button('Back',async()=>{const session=openGeneration;await save(onboardingNextStep(documentState.progress.step,-1));if(session===openGeneration&&onboardingIsOpen())await paint();});back.id='onboarding-back';const spacer=node('span',null,'onboarding-spacer'),skip=button('Skip',()=>next('skipped')),cont=button('Continue',()=>continueAction(),true);skip.id='onboarding-skip';cont.id='onboarding-continue';footer.append(back,spacer,skip,cont);shell.append(heading,content,error,footer);dialog.append(shell);document.body.append(dialog);
  dialog.addEventListener('close',()=>{if(!host&&!dialog.open&&active)closeOnboarding();});
 }
 async function runAction(action){
   if(actionPending)return;const serial=++actionSerial,focus=captureActionFocus(serial);actionPending=true;setBusy(true);
   try{await action();}catch(e){if(serial===actionSerial)showError(e);}finally{if(serial===actionSerial){actionPending=false;setBusy(false);settleActionFocus(focus);}}
 }
 function captureActionFocus(serial){
   const trigger=document.activeElement;if(!trigger||!onboardingIsOpen()||!shell?.contains(trigger))return null;
   const holder=shell.querySelector('.onboarding-heading h2');if(!holder)return null;
   const focus={trigger,holder,root:onboardingRoot(),shell,session:openGeneration,serial,step:documentState.progress.step};holder.tabIndex=-1;holder.focus({preventScroll:true});return focus;
 }
 function settleActionFocus(focus){
   if(!focus||focus.serial!==actionSerial||focus.session!==openGeneration||focus.root!==onboardingRoot()||focus.shell!==shell||!onboardingIsOpen()||document.activeElement!==focus.holder||!focus.holder.isConnected)return;
   const parent=focus.root.closest('dialog');if([...document.querySelectorAll('dialog[open]')].some(open=>open!==parent&&!open.contains(parent)))return;
   const sameStep=focus.step===documentState.progress.step,original=focus.trigger;
   const target=sameStep&&original.isConnected&&shell.contains(original)&&!original.disabled&&!original.hidden?original:content.querySelector('h3')||focus.holder;
   if(target.tagName==='H3')target.tabIndex=-1;target.focus({preventScroll:sameStep});
 }
 function setBusy(busy){const root=onboardingRoot();if(!root)return;root.setAttribute('aria-busy',String(busy));for(const control of root.querySelectorAll('.onboarding-shell button,.onboarding-shell select')){if(control.closest('.connection-setup,.settings-profile-import'))continue;control.disabled=busy||Boolean(documentState.progressReadOnly);}const back=$('#onboarding-back');if(back)back.disabled=busy||Boolean(documentState.progressReadOnly)||documentState.progress.step==='assistant';const close=root.querySelector('[data-onboarding-close]');if(close)close.disabled=busy;}
 const button=(text,action,primary=false)=>{const b=node('button',text,'button ui-control');b.dataset.variant=primary?'primary':'secondary';b.type='button';b.disabled=actionPending;b.onclick=()=>void runAction(action);return b;};
 const showError=e=>{error.textContent=e.message||String(e);};
 async function save(step,status='current'){const current=documentState,session=openGeneration;const progress=await api('/api/onboarding',{revision:current.progress.revision,step,status});if(session===openGeneration&&documentState===current){current.progress=progress;current.progressError=null;}return progress;}
 async function next(status='complete'){
  if(!onboardingIsOpen())return;const session=openGeneration;await save(documentState.progress.step,status);if(session!==openGeneration||!onboardingIsOpen())return;
  if(documentState.progress.step==='finish'){closeOnboarding();void window.truffleGuidance?.refresh();return;}
  await save(onboardingNextStep(documentState.progress.step));if(session===openGeneration&&onboardingIsOpen())await paint();
 }
 async function preference(patch){const session=openGeneration,current=await api('/api/settings');if(session!==openGeneration||!onboardingIsOpen())return;await api('/api/settings',{revision:current.revision,contentHash:current.contentHash,patch});if(session===openGeneration&&onboardingIsOpen())await loadAppSettings();}
 function paragraph(text){content.append(node('p',text));}
 function confirmChoice(items,preferred,key,question){
   const session=openGeneration;let selected=items.find(item=>item.id===preferred)||items[0];paragraph(question);
   const controls=node('div',null,'onboarding-choices ui-segmented');controls.setAttribute('role','group');controls.setAttribute('aria-label',question);const options=[];
   const update=()=>{for(const [item,control]of options)control.setAttribute('aria-pressed',String(item.id===selected.id));$('#onboarding-continue').textContent='Use '+selected.label;};
   if(items.length>1){for(const item of items){const control=node('button',item.label,'button ui-control');control.type='button';control.dataset.variant='secondary';control.disabled=actionPending||Boolean(documentState.progressReadOnly);control.onclick=()=>{if(actionPending||documentState.progressReadOnly||session!==openGeneration||!onboardingIsOpen())return;selected=item;update();};options.push([item,control]);controls.append(control);}content.append(controls);}else paragraph('We found '+selected.label+'.');
   update();continueAction=async()=>{await preference({[key]:selected.id});if(session===openGeneration&&onboardingIsOpen())await next();};
 }
 function choices(rows,change,current,label){const select=node('select',null,'ui-field');select.setAttribute('aria-label',label);for(const [id,title]of rows){const option=node('option',title);option.value=id;select.append(option);}select.value=current||rows[0]?.[0]||'';select.disabled=actionPending;select.onchange=()=>void runAction(()=>change(select.value));content.append(select);return select;}
 async function leaveSettings(category){
  const page=source,ticket=paintGeneration,step=documentState.progress.step;await save(step,'pending');if(!onboardingIsOpen()||ticket!==paintGeneration||documentState.progress.step!==step)return;
  closeOnboarding({forChild:true});const suspended=resume;
  try{const accepted=await openPreferences({category});const child=$('#settings-dialog');if(accepted!==true||!child?.open)throw new Error('Settings could not open. Your setup progress is saved; try again.');if(resume!==suspended){child.close();return;}returnAfter(child);}
  catch(e){if(resume===suspended){resume=null;if(source===page&&!document.querySelectorAll('dialog[open]').length){showSurface();await paint();}}throw e;}
 }
 function returnAfter(child){const suspended=resume;child.addEventListener('close',()=>{if(!suspended||resume!==suspended||suspended.ticket!==returnGeneration)return;resume=null;void window.openOnboarding({host:suspended.host||undefined}).catch(e=>{if(onboardingIsOpen())showError(e);});},{once:true});}
 async function addSkill(skill){
   const session=openGeneration;pendingImport=true;acceptedPresetId=null;savedPreset=null;await save('skills','pending');if(!onboardingIsOpen()||session!==openGeneration)return;closeOnboarding({forChild:true});
   const suspended=resume,child=$('#skill-editor');openSkillEditor({skill,source:skill?'installed':'file',returnLabel:'Close and return to setup',cwd:typeof currentAgentRecord==='function'?(currentAgentRecord()?.cwd||''):'',onSaved:id=>{if(resume===suspended&&suspended?.ticket===returnGeneration)acceptedPresetId=id;}});returnAfter(child);
 }
 async function discoverSkills(review=false,isCurrent=onboardingIsOpen){
   const box=node('div',null,'onboarding-card'),query=node('input',null,'ui-field'),results=node('div');query.type='search';if(review)query.value='review';query.placeholder='Find an installed skill by name or path';query.setAttribute('aria-label',query.placeholder);box.append(query,results);content.append(box);
   const expectedStep=documentState.progress.step;const catalog=await api('/api/skills/catalog',{cwd:typeof currentAgentRecord==='function'?(currentAgentRecord()?.cwd||''):''});
   if(!isCurrent()||documentState.progress.step!==expectedStep)return;const draw=()=>{results.replaceChildren();const matches=skillPickerMatches(catalog.skills||[],query.value,setting('defaultAgent','claude'));for(const skill of matches){const b=button(skill.name,async()=>{if(!review){await addSkill(skill);return;}const agent=onboardingSkillAgent(skill,setting('defaultAgent','claude'));await preference({[agent==='codex'?'codexReviewCommand':'claudeReviewCommand']:(agent==='codex'?'$':'/')+(skill.commandName||skill.name)});if(isCurrent())await next();});b.className='onboarding-result';b.append(node('small',skill.description),node('small',skill.path));results.append(b);}if(!matches.length)results.append(node('p','No matching installed skills.'));};query.oninput=draw;draw();
 }
 function renderCompanySetup(){
   const card=node('section',null,'onboarding-card onboarding-company-instructions'),label=node('label','Have instructions from your team?'),input=node('textarea',null,'ui-field');input.rows=5;input.placeholder='Paste the team setup guide you want help adapting';input.setAttribute('aria-label','Team setup instructions');input.setAttribute('style','width:100%;max-width:100%;resize:vertical');input.autocomplete='off';input.spellcheck=false;
   for(const target of [card,input])for(const marker of ['data-private','data-agent-private','data-agent-context-exclude'])target.setAttribute(marker,'true');
   let disposed=false;card.disposeCompanySetup=()=>{disposed=true;input.value='';};
   const review=button('Review in App chat',()=>{if(disposed)return;const message=onboardingCompanyMessage(input.value);if(typeof window.prepareAppChatMessage!=='function')throw new Error('App chat is unavailable. Your instructions are kept here.');window.prepareAppChatMessage(message);input.value='';closeOnboarding();},true);
   label.append(input);card.append(label,node('p','Preparing this draft sends nothing and changes no settings. Review it, then choose Send in App chat. After Send, App chat can apply routine preferences; commands and sharing access use their normal approval flows. Keep API keys and other credentials in Connections.'),review);content.append(card);
   const profile=node('details'),summary=node('summary','Or import a settings profile');profile.append(summary);renderSettingsProfilePreview(profile);content.append(profile,button('Open sharing setup',()=>leaveSettings('sharing')));paragraph('Sharing can use a shared folder or configured Google Cloud Storage. Review the destination and inbox in Sharing settings. No company destination or credentials are assumed.');
 }
 async function paint(){
   if(!onboardingIsOpen())return;const ticket=++paintGeneration;
   for(const setup of content.querySelectorAll('.onboarding-company-instructions'))setup.disposeCompanySetup?.();
   for(const importer of content.querySelectorAll('.settings-profile-import'))importer.disposeProfileImport?.();
   for(const setup of content.querySelectorAll('.connection-setup'))setup.disposeConnectionSetup?.();
   if(documentState.progressReadOnly){content.replaceChildren(node('h3','Setup unavailable'),node('p',documentState.progressError));$('#onboarding-step-count').textContent='';error.textContent='';setBusy(false);return;}
   continueAction=()=>next();const step=documentState.progress.step;const isCurrent=()=>ticket===paintGeneration&&onboardingIsOpen()&&documentState.progress.step===step;content.replaceChildren(node('h3',labels[step]));error.textContent='';$('#onboarding-step-count').textContent=(Object.keys(labels).indexOf(step)+1)+' / '+Object.keys(labels).length;$('#onboarding-back').disabled=step==='assistant';$('#onboarding-continue').textContent=step==='finish'?'Finish':'Continue';$('#onboarding-skip').hidden=step==='finish';if(documentState.progressError)paragraph(documentState.progressError);
   try{if(step==='assistant'){
     paragraph('Welcome. Let’s make this space yours, one choice at a time. You can skip any step and return from Settings.');const installed=documentState.assistants.filter(a=>a.installed);
     if(installed.length){confirmChoice(installed,setting('defaultAgent','claude'),'defaultAgent','Which assistant would you like for new chats?');paragraph('App chat uses your preferred installed assistant, or the other installed assistant if your preference is unavailable. Each reply keeps the assistant it started with.');}else{paragraph('No supported assistant found. You can finish setup and install Claude or Codex later.');$('#onboarding-continue').textContent='Continue without an assistant';continueAction=()=>next('skipped');}
   }else if(step==='theme'){
     paragraph('Pick a theme. Changes apply now and remain editable in Settings.');choices([['system','Follow system'],...Object.values(window.mydashThemeCatalog||{}).map(t=>[t.id,t.label+' · '+t.type])],v=>preference({theme:v}),setting('theme','system'),'Theme');
   }else if(step==='editor'){
     const editors=onboardingSupportedEditors(documentState.editors);if(editors.length){confirmChoice(editors,setting('editor','zed'),'editor','Which editor would you like to use for diffs?');}else{paragraph('No supported editor found. You can continue and change Diff editor later in Settings.');$('#onboarding-continue').textContent='Continue without an editor';continueAction=()=>next('skipped');}
     if(documentState.editors.some(e=>e.installed&&!e.diffSupported))paragraph('VS Code or Cursor was detected, but its current adapter opens a folder rather than a diff. It is not offered as a diff editor yet.');
   }else if(step==='keymap'){
     paragraph('Keep your current shortcuts, or use VS Code / Cursor basics for three common actions. This changes only shortcuts in truffle.');
     const current=await api('/api/settings');if(!isCurrent())return;const preview=onboardingKeymapPreset(current.values.keybindings),label=id=>appSettings.commands.find(command=>command.id===id)?.label||id;
     const list=node('ul',null,'onboarding-keymap-preview');for(const id of ['commands','find-work','toggle-left'])list.append(node('li',label(id)+': '+(current.values.keybindings[id]||'Unassigned')+' → '+preview.bindings[id]));
     for(const id of preview.displaced)list.append(node('li',label(id)+': '+current.values.keybindings[id]+' → Unassigned'));content.append(list);
     if(preview.displaced.length)paragraph('The conflicting shortcuts listed above become unassigned. You can set new shortcuts in Settings.');
     content.append(button('Keep current shortcuts',()=>next()),button('Apply these shortcuts',async()=>{const latest=await api('/api/settings');if(!isCurrent())return;if(latest.revision!==current.revision||latest.contentHash!==current.contentHash)throw new Error('Shortcuts changed in another window. Go Back and return to review the updated preview.');await api('/api/settings',{revision:current.revision,contentHash:current.contentHash,patch:{keybindings:preview.bindings}});await loadAppSettings();if(isCurrent())await next();},true));paragraph('This preset uses single key combinations. Multi-stroke and double-Shift shortcuts are not supported.');
   }else if(step==='integrations'){
     paragraph('Connect your work accounts, or skip and return later. Token inputs stay outside chat. Validate a token to preview its account and accessible resources before you save it.');
     await renderConnectionSetup(content);if(!isCurrent())return; 
   }else if(step==='skills'){
     paragraph('Would you like to add any of your installed Claude or Codex skills? Choose one below, or enter a direct path. Adding a skill does not run it.');choices([['on-demand','On demand'],['schedule','Set up a schedule']],async v=>{skillIntent=v;await paint();},skillIntent,'Skill availability');
     content.append(button('Enter a skill path',()=>addSkill(null)));await discoverSkills(false,isCurrent);if(!isCurrent())return;
     if(pendingImport&&acceptedPresetId){const library=await api('/api/library');if(!isCurrent())return;savedPreset=library.presets.find(preset=>preset.id===acceptedPresetId)||savedPreset;}
     if(!savedPreset&&documentState.progress.steps?.skills==='pending'){const library=await api('/api/library');if(!isCurrent())return;if(library.presets.length){paragraph('If you already saved a skill, choose it to configure its schedule.');const saved=choices(library.presets.map(p=>[p.id,p.title]),()=>{},library.presets[0].id,'Saved skill to schedule');content.append(button('Configure selected skill schedule',async()=>{const preset=library.presets.find(p=>p.id===saved.value);await save('skills','pending');if(!isCurrent())return;closeOnboarding({forChild:true});editSchedule(preset);returnAfter($('#schedule-editor'));}));}}
     if(savedPreset){const card=node('div',null,'onboarding-card'),schedule=typeof state==='undefined'?null:state?.skillSchedules?.[savedPreset.id];card.append(node('strong','Added: '+savedPreset.title));if(skillIntent==='schedule'||schedule?.enabled){const timing=schedule?.enabled?(schedule.mode==='daily'?'Daily at '+schedule.time:'Every '+schedule.everyMinutes+' minutes'):null;card.append(node('p',timing?'Schedule enabled: '+timing+'. You can also run this skill on demand.':'Choose the time and explicitly enable the schedule if you want it to run.'),button('Configure schedule',async()=>{await save('skills','pending');if(!isCurrent())return;closeOnboarding({forChild:true});editSchedule(savedPreset);returnAfter($('#schedule-editor'));}));}else card.append(node('p','Available on demand. No schedule is enabled.'));content.append(card);}
   }else if(step==='review'){
     const builtin=appSettings.reviewSkill;
     if(builtin?.available){
       paragraph('The portable review uses four independent reviewers for requirements, simplification, correctness and performance across related repositories. Choosing it does not start a review or enable a schedule.');
       paragraph('Edit its instructions at '+builtin.path);
       for(const agent of documentState.assistants.filter(agent=>agent.installed))content.append(button('Use portable review with '+agent.label,async()=>{await preference(onboardingReviewPatch(builtin,agent.id));if(isCurrent())await next();},agent.id===setting('defaultAgent','claude')));
     }
     paragraph('You can choose another available review skill, or skip this step.');await discoverSkills(true,isCurrent);if(!isCurrent())return;content.append(button('Enter review command in Settings',()=>leaveSettings('agents')));
   }else if(step==='company'){
     renderCompanySetup();
   }else{
     paragraph('Your preferences are saved. You can reopen setup at any time. Skipped steps remain available in Settings.');for(const [id,status]of Object.entries(documentState.progress.steps||{}))content.append(node('p',(labels[id]||id)+' · '+status));content.append(button('Start setup again',async()=>{const session=openGeneration;const progress=await api('/api/onboarding',{revision:documentState.progress.revision,reset:true});if(session!==openGeneration||!onboardingIsOpen())return;documentState.progress=progress;await paint();}));
   }}catch(e){if(isCurrent())showError(e);}
 }
 window.openOnboarding=async function({host:target}={}){
   if(typeof settingsDirty!=='undefined'&&settingsDirty)throw new Error('Save or discard settings before opening setup.');
   if(target!==undefined&&(!target?.isConnected||target.tagName!=='SECTION'))throw new Error('Setup needs a connected section host.');
   if(!requestDialogNavigation())return false;
   closeOnboarding();host=target||null;ensureRenderer();const ticket=++openGeneration;
   const navigation=captureDialogNavigation(),current=()=>ticket===openGeneration&&(!target||target.isConnected)&&navigation.isCurrent();
   let nextDocument;try{nextDocument=await api('/api/onboarding');if(!current())return false;await loadAppSettings({canApply:current});if(!current())return false;}finally{navigation.dispose();}
   documentState=nextDocument;showSurface();await paint();
 };
 window.closeOnboarding=closeOnboarding;window.onboardingIsOpen=onboardingIsOpen;window.onboardingRoot=onboardingRoot;
})();
