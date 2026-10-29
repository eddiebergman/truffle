'use strict';
let pageSettingsDocument=null;
const pageSettingsCatalog={
 linear:{label:'Linear',scope:'projects',category:'connections',description:'Choose the projects shown on the Linear page. This changes the view; it does not change imports or source permissions.'},
 github:{label:'GitHub',scope:'repositories',category:'connections',description:'Choose the repositories shown on the GitHub page. This changes the view; it does not change imports or source permissions.'},
 sentry:{label:'Sentry',scope:'projects',category:'connections',description:'Choose the projects shown on the Sentry page. This changes the view; it does not change imports or source permissions.'},
 agents:{label:'Agents',category:'agents',description:'Configure the default agent, editors and review workflow. Terminal appearance is in Appearance.'},
 skills:{label:'Skills',category:'agents',description:'Choose installed skills with Add skill. Each skill has its own Schedule control; schedules start disabled.'},
 sessions:{label:'History',category:'agents',description:'Use Show archived on this page to include archived conversations. Agent and workflow defaults apply to resumed chats.'},
 'chat-results':{label:'Search results',category:'agents',description:'Results keep their own query and selected item. Agent defaults apply when opening a saved chat.'},
 notes:{label:'Projects',category:'appearance',description:'Project status is edited on each project. Use this page’s status and grouping controls to arrange the list.'},
 all:{label:'All Items',category:'connections',description:'Connected sources supply this list. Use the status and grouping controls on the page to arrange it.'},
 pinned:{label:'Pinned',category:'appearance',description:'Pin or unpin individual items. Use this page’s status and grouping controls to arrange the list.'},
 logs:{label:'Logs',category:'connections',description:'Log source and search controls are on this page. Configure connection credentials and synchronization in Connections.'},
 changes:{label:'Changes',category:'connections',description:'Changes come from connected sources. Mark changes read on this page.'},
 move:{label:'Your move',category:'connections',description:'This view uses source state to identify work needing attention. Configure its sources in Connections.'},
 archive:{label:'Archive',category:'data',description:'Restore individual items from this page. Export and backup controls are in Data & profiles.'},
 artifacts:{label:'Artifacts',category:'data',description:'Files belong to their chat or skill run. Manage exports and profile storage in Data & profiles.'},
 newsletters:{label:'Skills',category:'agents',description:'Use each skill’s Schedule control to set its cadence.'},
 briefing:{label:'Briefings',category:'agents',description:'Briefings belong to their originating chat and workflow.'}
};
function pageSettingsAllows(item,page=source){if(item.source!==page)return true;const scopes=pageSettingsDocument?.pages?.[item.source]?.scopes;return !Array.isArray(scopes)||scopes.includes(scopeKey(item));}
async function loadPageSettings({canApply=()=>true}={}){try{const next=await api('/api/page-settings');if(!canApply())return null;pageSettingsDocument=next;render();return next;}catch(error){console.warn('Page settings could not load:',error.message);}}
function pageSettingsGear(page){const button=el('button','text-button page-settings-gear');button.type='button';button.title=pageSettingsCatalog[page]?.scope?'Configure '+pageSettingsCatalog[page].label+' visibility':(pageSettingsCatalog[page]?.label||'Page')+' settings';button.setAttribute('aria-label',button.title);button.innerHTML='<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="m9 3-.8 2.5-2 .9L3.8 6l-2 3.5 1.7 1.9v2l-1.7 1.9 2 3.5 2.4-.4 2 .9L9 22h4l.8-2.7 2-.9 2.4.4 2-3.5-1.7-1.9v-2l1.7-1.9-2-3.5-2.4.4-2-.9L13 3Z"/><circle cx="11" cy="12.5" r="3"/></svg>';button.onclick=()=>void openPageSettings(page);return button;}
function renderPageSettingsGear(){let button=document.querySelector('[data-page-settings-gear]');if(!pageSettingsCatalog[source]){button?.remove();return;}const host=source==='agents'?$('#agent-topbar'):$('.page-heading');if(!host)return;if(!button){button=pageSettingsGear(source);button.dataset.pageSettingsGear=source;}if(button.dataset.pageSettingsGear!==source){button.dataset.pageSettingsGear=source;button.title=pageSettingsCatalog[source].scope?'Configure '+pageSettingsCatalog[source].label+' visibility':pageSettingsCatalog[source].label+' settings';button.setAttribute('aria-label',button.title);}button.onclick=()=>void openPageSettings(source);if(button.parentElement!==host)host.append(button);}
async function openPageSettingsCategory(category){return openPreferences({category});}
function pageSettingsButton(label,handler,variant='secondary'){
 const button=el('button','ui-control',label);button.type='button';button.dataset.variant=variant;button.onclick=handler;return button;
}
function pageSettingsActions(label){
 const group=el('div','ui-actiongroup page-settings-actions');group.setAttribute('role','group');group.setAttribute('aria-label',label);return group;
}
async function openPageSettings(page){
 const definition=pageSettingsCatalog[page];if(!definition)return;
 if(typeof currentSavedViewState==='function'&&currentSavedViewState(page)){
  const dialog=el('dialog','page-settings-dialog');dialog.setAttribute('aria-label',definition.label+' settings');const heading=el('header','page-settings-header');heading.append(el('h2','',definition.label+' settings'));dialog.append(heading);
  const actions=pageSettingsActions(definition.label+' settings actions');
  actions.append(pageSettingsButton('Edit this view’s filters',()=>{dialog.close();openSavedViewEditor();},'primary'));
  if(definition.scope)actions.append(pageSettingsButton('Choose visible '+definition.scope,()=>{dialog.close();void openPageVisibilitySettings(page);}));
  actions.append(pageSettingsButton('Close',()=>dialog.close()));dialog.append(actions);dialog.addEventListener('close',()=>dialog.remove(),{once:true});document.body.append(dialog);dialog.showModal();return;
 }
 await openPageVisibilitySettings(page);
}
async function openPageVisibilitySettings(page){
 const definition=pageSettingsCatalog[page];if(!definition)return;
 if(!definition.scope){await openPageSettingsCategory(definition.category);return;}
 const dialog=el('dialog','page-settings-dialog');dialog.setAttribute('aria-label',definition.label+' settings');
 let saving=false,accepted=false,conflicted=false,reload=null,retry=null;
 const dismiss=()=>requestDialogClose(dialog);
 dialog.addEventListener('cancel',event=>{if(saving)event.preventDefault();});
 const header=el('header','page-settings-header');header.append(el('h2','',definition.label+' settings'));const close=pageSettingsButton('×',dismiss);close.dataset.size='icon';close.setAttribute('aria-label','Close page settings');header.append(close);dialog.append(header,el('p','form-hint',definition.description));
 const error=el('p','form-hint page-settings-error');error.setAttribute('role','status');error.dataset.tone='neutral';
 const status=el('p','form-hint');status.setAttribute('role','status');status.tabIndex=-1;status.hidden=true;
 const current={dialog,host:dialog,ui:{message:status},pending:false,refreshing:false,controls:new Map(),isCurrent:()=>dialog.open&&dialog.isConnected};
 const releaseFocus=target=>{restoreFormFocus(current,target);if(document.activeElement===status)status.textContent=error.textContent||'Page visibility is ready for review.';else status.hidden=true;};
 const actions=pageSettingsActions(definition.label+' visibility actions');
 try{
  pageSettingsDocument=await api('/api/page-settings');const revision=pageSettingsDocument.revision,contentHash=pageSettingsDocument.contentHash,selected=pageSettingsDocument.pages[page].scopes;
  const choices=[...new Set([...allItems().filter(x=>x.source===page).map(scopeKey),...(selected||[])])].sort((a,b)=>a.localeCompare(b));
  const label=el('label','page-settings-choice'),all=el('input');all.type='checkbox';all.checked=selected===null;label.append(all,document.createTextNode('All '+definition.scope+', including new ones'));dialog.append(label);
  const search=el('input','page-settings-search');search.type='search';search.placeholder='Find '+definition.scope+'…';search.setAttribute('aria-label',search.placeholder);dialog.append(search);
  const list=el('div','page-settings-scopes');list.setAttribute('role','group');list.setAttribute('aria-label','Visible '+definition.scope);const inputs=[];
  for(const scope of choices){const row=el('label','page-settings-choice'),input=el('input');input.type='checkbox';input.value=scope;input.checked=selected===null||selected.includes(scope);input.disabled=all.checked;row.append(input,el('span','',scope||(page==='github'?'No repository':'No project')));inputs.push(input);list.append(row);}
  if(!choices.length)list.append(el('p','form-hint','No '+definition.scope+' are cached yet. Connect and sync this source first.'));
  all.onchange=()=>{if(saving||accepted)return;for(const input of inputs)input.disabled=all.checked;};search.oninput=()=>{for(const row of list.children)row.hidden=!row.textContent.toLowerCase().includes(search.value.toLowerCase());};dialog.append(list);
  const clear=pageSettingsButton('Clear selection',()=>{if(saving||accepted)return;all.checked=false;for(const input of inputs){input.disabled=false;input.checked=false;}});actions.append(clear);
  const present=()=>{if(scopePrefs[page]&&Array.isArray(pageSettingsDocument.pages[page].scopes)&&!pageSettingsDocument.pages[page].scopes.includes(scopePrefs[page]))scopePrefs[page]='';persist();render();dialog.close();};
  const presentationFailed=()=>{
   error.dataset.tone='neutral';error.textContent='Visibility saved. The dashboard could not refresh. Use Refresh view; do not save again.';
   if(!retry){retry=pageSettingsButton('Refresh view',()=>{if(saving||!accepted)return;status.hidden=false;status.textContent='Refreshing view…';holdFormFocus(dialog,current);try{present();}catch(_){presentationFailed();}finally{releaseFocus(retry);}});actions.append(retry);}
  };
  const save=pageSettingsButton('Save visibility',null,'primary');save.onclick=async()=>{
   if(saving||accepted||conflicted)return;
   saving=true;current.pending=true;error.textContent='';status.hidden=false;status.textContent='Saving visibility…';holdFormFocus(dialog,current);current.controls.clear();freezeFeatureForm(current,true);
   try{
    pageSettingsDocument=await api('/api/page-settings',{revision,contentHash,page,scopes:all.checked?null:inputs.filter(i=>i.checked).map(i=>i.value)});accepted=true;
    if(dialog.open){try{present();}catch(_){presentationFailed();}}
    else toast('Visibility saved. Refresh the dashboard to see the changes; do not save again.');
   }catch(e){
    error.dataset.tone='error';if(e.status===409){conflicted=true;error.textContent='Page visibility changed elsewhere. Your selection is kept. Discard it and reload before saving.';
     if(!reload){reload=pageSettingsButton('Discard selection and reload',async()=>{if(saving)return;dismiss();if(!dialog.open)await openPageVisibilitySettings(page);});actions.insertBefore(reload,save);}
    }else error.textContent=e.message;
   }finally{
    saving=false;current.pending=false;freezeFeatureForm(current,false);
    if(accepted){all.disabled=true;for(const input of inputs)input.disabled=true;clear.disabled=true;}save.disabled=conflicted||accepted;
    releaseFocus(accepted?retry:conflicted?reload:current.focusReturn||save);
   }
  };actions.append(save);
 }catch(e){error.dataset.tone='error';error.textContent=e.message;}
 const category=pageSettingsButton('Open '+({connections:'Connections',agents:'Agents & workflows',appearance:'Appearance',data:'Data & profiles'}[definition.category]||'Settings')+' settings',()=>{if(saving)return;dismiss();if(!dialog.open)void openPageSettingsCategory(definition.category);});const done=pageSettingsButton('Close',dismiss);const save=actions.lastElementChild;if(save?.dataset.variant==='primary'){actions.insertBefore(category,save);actions.insertBefore(done,save);}else actions.append(category,done);dialog.append(error,status,actions);dialog.addEventListener('close',()=>dialog.remove(),{once:true});document.body.append(dialog);dialog.showModal();
}
void loadPageSettings();
