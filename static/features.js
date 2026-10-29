'use strict';
// All optional views build on the same local records and existing agent actions.
let bundleIndex=new Map(), bundleCatalog=new Map(), environmentMembers=new Set();
let linkedSessionIndex=new Map();
let sentryFilters=prefs.sentryFilters&&typeof prefs.sentryFilters==='object'?prefs.sentryFilters:{};
let sentryView=typeof prefs.sentryView==='string'?prefs.sentryView:'';
function buildBundleIndex(items=allItems()){
  bundleIndex=new Map();bundleCatalog=new Map();linkedSessionIndex=new Map();
  for(const item of state.relatedItems||[])bundleCatalog.set(item.id,item);
  for(const session of state.sessions||[])for(const item of session.items)bundleCatalog.set(item.id,item);
  for(const item of items)bundleCatalog.set(item.id,item);
  for(const bundle of state.bundles||[])for(const key of bundle.itemIds)bundleIndex.set(key,bundle.itemIds);
  for(const session of state.sessions||[]){
    if(session.archivedAt)continue;
    for(const ref of session.items){const id=ref.id;
      if(!linkedSessionIndex.has(id))linkedSessionIndex.set(id,new Set());
      linkedSessionIndex.get(id).add(session);
    }
  }
}
function relatedItemIds(id){return new Set(bundleIndex.get(id)||[id]);}
function relatedRecords(id){return [...relatedItemIds(id)].map(key=>bundleCatalog.get(key)).filter(Boolean);}
function featureButton(label,handler,cls='button'){const button=el('button',cls,label);button.type='button';button.onclick=handler;return button;}
function featureIconButton(text,label,handler,cls='icon-button'){const button=featureButton(text,handler,cls);button.title=label;button.setAttribute('aria-label',label);return button;}
// Fixed feature forms share pending/recovery ownership; payloads stay in each feature.
const featureForms=new Map(),featureFormDialogs=new WeakSet();
function disposeFeatureForm(dialog){const current=featureForms.get(dialog);featureForms.delete(dialog);if(current)for(const [control,disabled]of current.controls)control.disabled=disabled;dialog.querySelector('[data-feature-write]')?.remove();dialog.setAttribute('aria-busy','false');}
function createFeatureForm(dialog,form,target){
  disposeFeatureForm(dialog);const page=source,current={dialog,host:form,target,pending:false,refreshing:false,result:null,controls:new Map(),afterReady:null,acceptedText:''};
  current.isCurrent=()=>featureForms.get(dialog)===current&&dialog.open&&source===page&&(!current.targetCurrent||current.targetCurrent());featureForms.set(dialog,current);
  if(!featureFormDialogs.has(dialog)){featureFormDialogs.add(dialog);dialog.addEventListener('cancel',event=>{if(featureForms.get(dialog)?.pending)event.preventDefault();});dialog.addEventListener('close',()=>{if(!dialog.open)disposeFeatureForm(dialog);});}
  const region=el('div');region.dataset.featureWrite='true';const message=el('p','form-hint');message.tabIndex=-1;message.setAttribute('role','status');const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label','Save recovery actions');
  const refresh=featureButton('Refresh view',()=>refreshFeatureForm(current),'ui-control');refresh.dataset.variant='secondary';refresh.dataset.featureRecovery='true';refresh.hidden=true;
  const inspect=featureButton('Close and inspect this view',()=>{if(current.isCurrent()&&!current.pending)dialog.close();},'ui-control');inspect.dataset.variant='secondary';inspect.dataset.featureRecovery='true';inspect.hidden=true;
  actions.append(refresh,inspect);region.append(message,actions);form.append(region);current.ui={region,message,refresh,inspect};return current;
}
function featureFormMutable(current){return current?.isCurrent()&&!current.pending&&!current.refreshing&&!['accepted','unknown'].includes(current.result?.kind);}
function freezeFeatureForm(current,locked){
  for(const control of current.host.querySelectorAll('input,textarea,select,button')){if(control.dataset.featureRecovery||control.closest('#app-chat-panel,.xterm'))continue;if(!current.controls.has(control))current.controls.set(control,control.disabled);control.disabled=control.classList.contains('close-dialog')?current.pending:locked||current.controls.get(control);}
  current.dialog.setAttribute('aria-busy',String(current.pending||current.refreshing));
}
function presentFeatureFormResult(current){
  if(!current.isCurrent())return;const locked=['accepted','unknown'].includes(current.result.kind);freezeFeatureForm(current,locked);current.ui.message.textContent=localWriteMessage(current.result,current.acceptedText);current.ui.refresh.hidden=!locked;current.ui.inspect.hidden=!locked;
  const correction=current.focusReturn?.matches('input,textarea,select')&&!current.focusReturn.disabled?current.focusReturn:current.host.querySelector('input:not([type="hidden"]):not(:disabled),textarea:not(:disabled),select:not(:disabled),button[type="submit"]:not(:disabled)');
  restoreFormFocus(current,locked?(current.result.kind==='unknown'?current.ui.inspect:current.ui.refresh):correction);
}
async function refreshFeatureForm(current){
  if(!current.isCurrent()||current.pending||current.refreshing||!['accepted','unknown'].includes(current.result?.kind))return;current.refreshing=true;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.refresh.disabled=true;
  try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('The dashboard changed while refreshing.');if(!current.isCurrent())return;render();if(!current.isCurrent())return;
    if(current.result.kind==='accepted'){await current.afterReady();toast(localWriteMessage({kind:'accepted',presentation:'ready'},current.acceptedText));}
    else current.ui.message.textContent='The save outcome is unknown. The view was refreshed. Inspect it before saving again; this draft was not resubmitted.';
  }catch(_){if(current.isCurrent())current.ui.message.textContent=localWriteMessage(current.result,current.acceptedText);else if(current.result.kind==='accepted')toast(localWriteMessage({kind:'accepted',presentation:'pending'},current.acceptedText));}
  finally{current.refreshing=false;if(current.isCurrent()){freezeFeatureForm(current,true);current.ui.refresh.disabled=false;restoreFormFocus(current,current.result.kind==='unknown'?current.ui.inspect:current.ui.refresh);}else if(featureForms.get(current.dialog)===current&&current.dialog.open){freezeFeatureForm(current,true);current.ui.refresh.disabled=false;}}
}
async function submitFeatureForm(current,payload,acceptedText,afterReady){
  if(!featureFormMutable(current))return;current.pending=true;current.afterReady=afterReady;current.acceptedText=acceptedText;holdFormFocus(current.host,current);freezeFeatureForm(current,true);current.ui.message.textContent='Saving…';
  let result=await mutate(payload);current.pending=false;current.result=result;
  if(!current.isCurrent()){if(featureForms.get(current.dialog)===current&&current.dialog.open)freezeFeatureForm(current,true);toast(localWriteMessage(result,acceptedText));return;}
  if(result.kind==='accepted'&&result.presentation==='ready'){
    try{await afterReady();toast(localWriteMessage(result,acceptedText));return;}catch(error){current.result=result={kind:'accepted',presentation:'pending',error:error.message};}
  }
  if(current.isCurrent())presentFeatureFormResult(current);else toast(localWriteMessage(result,acceptedText));
}
async function featureWrite(payload,acceptedText,{button=null,isCurrent=()=>true,afterReady=null,write=()=>mutate(payload)}={}){
  if(button?.disabled)return;const page=source;if(button)button.disabled=true;let result=await write();
  if(result.kind==='accepted'&&result.presentation==='ready'&&source===page&&isCurrent())try{afterReady?.();}catch(error){result={kind:'accepted',presentation:'pending',error:error.message};}
  if(result.kind==='rejected'&&button?.isConnected)button.disabled=false;toast(localWriteMessage(result,acceptedText));return result;
}
let featureNavigationIntent=0;
function markObservedChange(change,button,open=false){
  const id=change.id,itemId=change.itemId,page=source,query=$('#search').value,intent=++featureNavigationIntent;
  return featureWrite(null,'Change marked read.',{button,write:()=>markChangesRead([id]),isCurrent:()=>source===page&&$('#search').value===query&&featureNavigationIntent===intent&&!document.querySelector('dialog[open]'),afterReady:()=>{if(open){const item=allItems().find(row=>row.id===itemId);if(item)openItem(item);}}});
}
function dated(timestamp){return new Date(timestamp*1000).toLocaleString(undefined,{dateStyle:'medium',timeStyle:'short'});}
function sourceLink(url,label){const link=el('a','linear-ticket-link',label);link.href=url;link.target='_blank';link.rel='noopener noreferrer';return link;}
function renderRelatedWork(item){
  const section=$('#related-work');section.replaceChildren(el('h3','','Related work'));
  const records=relatedRecords(item.id).filter(x=>x.id!==item.id);
  section.append(el('p','form-hint',records.length?'Connected through ticket references, Sentry links, shared sessions, or manual links.':'No related work found yet. Link it below, or attach several items to the same session.'));
  for(const record of records){
    const row=el('div','related-row');
    const open=featureButton(`${sourceNames[record.source]} · ${workReference(record)||record.title} · ${record.title}`,()=>openItem(record),'text-button related-title');
    row.append(open);
    const edge=[item.id,record.id].sort();
    if((state.workLinks||[]).some(x=>x[0]===edge[0]&&x[1]===edge[1])){const remove=featureButton('Remove manual link',()=>featureWrite({action:'unlink-work',itemIds:edge},'Manual link removed.',{button:remove,isCurrent:()=>$('#item-dialog').open&&currentItem?.id===item.id&&remove.isConnected,afterReady:()=>{const latest=allItems().find(row=>row.id===item.id);if(latest)renderRelatedWork(latest);}}),'text-button');row.append(remove);}
    section.append(row);
  }
  $('#link-work-open').onclick=()=>openPalette(item);
  $('#snooze-open').textContent=item.snoozed?'Change snooze':'◷ Snooze';
  $('#snooze-open').onclick=()=>openSnooze(item);
}
let snoozeItem=null;
function openSnooze(item){
  if(!requestDialogNavigation())return false;disposeFeatureForm($('#snooze-dialog'));snoozeItem=item;
  $('#snooze-form').reset();$('#snooze-title').textContent=item.title;
  $('#snooze-custom').hidden=true;$('#snooze-form').elements.until.required=false;
  createFeatureForm($('#snooze-dialog'),$('#snooze-form'),item.id);$('#snooze-dialog').showModal();
}
$('#snooze-form').elements.duration.onchange=()=>{const custom=$('#snooze-form').elements.duration.value==='custom';$('#snooze-custom').hidden=!custom;$('#snooze-form').elements.until.required=custom;};
$('#snooze-form').onsubmit=async event=>{
  event.preventDefault();const form=event.currentTarget,current=featureForms.get($('#snooze-dialog'));if(!featureFormMutable(current))return;const choice=form.elements.duration.value;
  let payload={action:'snooze',id:current.target,mode:choice==='change'?'change':'until'};
  if(choice!=='change'){
    const date=choice==='custom'?new Date(form.elements.until.value):new Date();
    if(choice!=='custom'){date.setDate(date.getDate()+(choice==='week'?7:1));date.setHours(9,0,0,0);}
    payload.until=date.getTime()/1000;
    if(!Number.isFinite(payload.until)||payload.until<=Date.now()/1000){toast('Choose a future time.');return;}
  }
  await submitFeatureForm(current,payload,'Snoozed. Restore it from the bottom of the work view.',()=>$('#snooze-dialog').close());
};
function renderSnoozed(items){
  const section=$('#snoozed-items'),list=$('#snoozed-list');
  const query=$('#search').value.toLowerCase();
  const rows=items.filter(x=>x.snoozed&&!x.archivedAt&&(source==='move'?x.actionReasons?.length:matchesSource(x))&&matchesScope(x)&&[x.title,workReference(x),x.project].join(' ').toLowerCase().includes(query));
  section.hidden=['changes','briefing','archive','sessions','logs','newsletters','artifacts'].includes(source)||!rows.length;
  $('#snoozed-summary').textContent=`Snoozed · ${rows.length}`;list.replaceChildren();
  if(section.hidden||!section.open)return;
  for(const item of rows){
    const row=el('div','snoozed-row'),value=state.snoozes[item.id];
    row.append(featureButton(item.title,()=>openItem(item),'text-button related-title'),el('span','form-hint',value.until?dated(value.until):'Until source changes'),featureButton('Wake now',event=>featureWrite({action:'snooze',id:item.id,mode:'clear'},'Work item awakened.',{button:event.currentTarget}),'text-button'));
    list.append(row);
  }
}
$('#snoozed-items').ontoggle=()=>{if(state)renderSnoozed(allItems());};
function matchesSentry(item,page=source){
  if(page!=='sentry')return true;
  if(Number(item.userCount||0)<Number(sentryFilters.minUsers||0)||Number(item.eventCount||0)<Number(sentryFilters.minEvents||0))return false;
  if(sentryFilters.level&&item.level!==sentryFilters.level)return false;
  if(sentryFilters.category&&(item.category||'error')!==sentryFilters.category)return false;
  if(sentryFilters.days&&!(Date.parse(item.lastSeen)>Date.now()-sentryFilters.days*86400000))return false;
  if(sentryFilters.environment&&!environmentMembers.has(item.id))return false;
  return true;
}
let environmentRequest=null,environmentNext=0;
// Only unresolved request outcomes are retained, never provider data or snapshots.
const environmentOutcomes=new Map();
function currentEnvironment(name){return source==='sentry'&&sentryFilters.environment===name;}
async function refreshEnvironmentView(name,entry){
  if(!currentEnvironment(name)||environmentOutcomes.get(name)!==entry||entry.pending||entry.reading)return;entry.reading=true;
  try{const snapshot=await loadStateSnapshot({attempts:2});if(!snapshot.accepted)throw new Error('The dashboard changed while refreshing.');if(!currentEnvironment(name)||environmentOutcomes.get(name)!==entry)return;render();if(entry.kind==='accepted')environmentOutcomes.delete(name);}
  catch(_){/* The request outcome stays unresolved; recovery only reads cached state. */}
  finally{entry.reading=false;if(currentEnvironment(name)&&(!environmentOutcomes.has(name)||environmentOutcomes.get(name)===entry))renderSentryControls(false);}
}
async function ensureEnvironment(){
  const name=sentryFilters.environment;
  if(source!=='sentry'||!name||environmentRequest||environmentOutcomes.has(name)||environmentOutcomes.size>=20||Date.now()<environmentNext)return;
  const cached=state.sentryEnvironments?.[name],sourceInfo=state.sources.sentry;
  if(sourceInfo?.disabled===true||sourceInfo?.failures&&sourceInfo.nextAttempt>Date.now()/1000||state.sentryEnvNextAttempt>Date.now()/1000)return;
  if(cached?.running||cached?.nextAttempt>Date.now()/1000)return;
  const entry={kind:'pending',pending:true,reading:false};environmentOutcomes.set(name,entry);environmentRequest=entry;environmentNext=Date.now()+5000;
  try{const result=await presentLocalWrite(()=>api('/api/sentry/environment',{environment:name}));entry.pending=false;entry.kind=result.kind;if(environmentRequest===entry)environmentRequest=null;
    if(result.kind==='rejected'||result.kind==='accepted'&&result.presentation==='ready')environmentOutcomes.delete(name);
    if(currentEnvironment(name)){if(result.kind==='rejected')toast(result.error);renderSentryControls(false);}
  }finally{if(environmentRequest===entry)environmentRequest=null;}
}
function retryEnvironment(name,entry){
  if(!currentEnvironment(name)||environmentOutcomes.get(name)!==entry||entry.kind!=='unknown'||entry.pending||entry.reading||environmentRequest)return;
  // A separate explicit user action is required after an unknown request outcome.
  const info=state.sources.sentry;if(info?.disabled===true||info?.failures&&info.nextAttempt>Date.now()/1000)return;
  environmentOutcomes.delete(name);environmentNext=0;return ensureEnvironment();
}
function renderSentryControls(requestEnvironment=true){
  environmentMembers=new Set(state.sentryEnvironments?.[sentryFilters.environment]?.itemIds||[]);
  $('#sentry-controls').hidden=source!=='sentry';if(source!=='sentry')return;
  const select=$('#sentry-view');
  select.replaceChildren(new Option('All issues',''),...(state.sentryViews||[]).map(x=>new Option(x.name,x.id)));
  if(sentryView&&!state.sentryViews.some(x=>x.id===sentryView)){const current=featureForms.get($('#sentry-filter-dialog'));if(current?.isCurrent()&&current.selection===sentryView)current.selection='';sentryView='';}
  if(!sentryView&&Object.values(sentryFilters).some(Boolean))select.add(new Option('Custom filter','custom'));
  select.value=sentryView|| (Object.values(sentryFilters).some(Boolean)?'custom':'');
  const parts=[];
  if(sentryFilters.minUsers)parts.push(`≥ ${sentryFilters.minUsers} users`);
  if(sentryFilters.minEvents)parts.push(`≥ ${sentryFilters.minEvents} events`);
  if(sentryFilters.days)parts.push(`Seen within ${sentryFilters.days}d`);
  if(sentryFilters.level)parts.push(sentryFilters.level);
  if(sentryFilters.category)parts.push(sentryFilters.category);
  if(sentryFilters.environment){
    const cached=state.sentryEnvironments?.[sentryFilters.environment],paused=state.sources.sentry?.disabled===true;
    const status=cached?.running?'refreshing…'+(paused?' · connection paused for new requests':''):paused?(cached?.lastSuccess?'connection paused · cached '+relative(cached.lastSuccess):'connection paused · no cached data'):cached?.error||(cached?.lastSuccess?'cached '+relative(cached.lastSuccess):state.sources.sentry?.failures?'waiting for connection retry':'loading…');
    parts.push(`${sentryFilters.environment} · ${status}`);
    if(requestEnvironment)void ensureEnvironment();
  }
  const summary=$('#sentry-filter-summary');summary.textContent=parts.join(' · ');summary.setAttribute('role','status');
  const name=sentryFilters.environment,entry=environmentOutcomes.get(name);
  if(entry){const text=entry.pending?'Environment refresh requested; waiting for confirmation.':entry.kind==='unknown'?'Environment refresh outcome unknown; showing last loaded data.':'Environment request accepted; showing last loaded data.';summary.append(el('span','', ' · '+text));
    if(!entry.pending){const refresh=featureButton('Refresh view',()=>refreshEnvironmentView(name,entry),'text-button');refresh.disabled=entry.reading;summary.append(refresh);if(entry.kind==='unknown'){const retry=featureButton('Retry refresh',()=>retryEnvironment(name,entry),'text-button');const info=state.sources.sentry;retry.disabled=entry.reading||!!environmentRequest||info?.disabled===true||!!(info?.failures&&info.nextAttempt>Date.now()/1000);summary.append(retry);}}
  }else if(name&&environmentOutcomes.size>=20)summary.append(el('span','',' · Environment requests paused: recover an earlier request first.'));
}
$('#sentry-view').onchange=event=>{sentryView=event.target.value;const view=state.sentryViews.find(x=>x.id===sentryView);sentryFilters=view?{...view.filters}:{};persist();render();};
$('#sentry-filter-open').onclick=()=>{
  if(!requestDialogNavigation())return false;const dialog=$('#sentry-filter-dialog');disposeFeatureForm(dialog);const form=$('#sentry-filter-form');form.reset();
  for(const key of ['minUsers','minEvents','days','level','category','environment'])form.elements[key].value=sentryFilters[key]??(['days','minUsers','minEvents'].includes(key)?0:'');
  const view=state.sentryViews.find(row=>row.id===sentryView),selection=sentryView;form.elements.viewName.value=view?.name||'';$('#sentry-delete').hidden=!view;
  const current=createFeatureForm(dialog,form,view?.id||crypto.randomUUID());current.existing=!!view;current.selection=selection;current.targetCurrent=()=>sentryView===current.selection;dialog.showModal();
};
$('#sentry-filter-form').onsubmit=async event=>{
  event.preventDefault();const form=event.currentTarget,current=featureForms.get($('#sentry-filter-dialog'));if(!featureFormMutable(current))return;const filters={};
  for(const key of ['minUsers','minEvents','days'])filters[key]=Number(form.elements[key].value);
  for(const key of ['level','category','environment'])filters[key]=form.elements[key].value.trim();const name=form.elements.viewName.value.trim(),id=current.target;
  const apply=()=>{if(name&&!state.sentryViews.some(view=>view.id===id))throw new Error('The saved filter is unavailable in the refreshed view.');sentryFilters=filters;current.selection=sentryView=name?id:'';persist();render();$('#sentry-filter-dialog').close();};
  if(name)await submitFeatureForm(current,{action:'save-sentry-view',id,name,filters},'Sentry filter saved.',apply);else apply();
};
$('#sentry-clear').onclick=()=>{const current=featureForms.get($('#sentry-filter-dialog'));if(!featureFormMutable(current))return;sentryFilters={};sentryView='';persist();render();$('#sentry-filter-dialog').close();};
$('#sentry-delete').onclick=async()=>{const current=featureForms.get($('#sentry-filter-dialog'));if(!featureFormMutable(current)||!current.existing)return;const id=current.target;await submitFeatureForm(current,{action:'delete-sentry-view',id},'Sentry filter deleted.',()=>{sentryFilters={};current.selection=sentryView='';persist();render();$('#sentry-filter-dialog').close();});};
function changePill(value){
  const lower=value.toLowerCase();
  if(lower.startsWith('ci '))return el('span','ci-pill '+(lower.includes('fail')?'failed':'passed'),value);
  let status='todo';
  if(Object.hasOwn(labels,lower))status=lower;
  else if(/merge conflict|blocked/.test(lower))status='blocked';
  else if(/merged|closed|resolved|completed|done|cancel|duplicate|ignored|archived/.test(lower))status='done';
  else if(/draft/.test(lower))status='draft';
  else if(/approved|ready to merge/.test(lower))status='ready-to-merge';
  else if(/changes requested|in progress|started/.test(lower))status='in-progress';
  else if(/review/.test(lower))status='needs-review';
  const caption=/merge conflict/.test(lower)?'Conflict':lower==='your review requested'?'Review requested':labels[lower]||value;
  return el('span','status-pill '+status,caption);
}
function renderFeatureView(items){
  const active=items.filter(x=>!x.hidden&&!x.snoozed&&!x.archivedAt&&x.status!=='done');
  $('#count-move').textContent=active.filter(x=>x.actionReasons?.length).length;
  $('#count-changes').textContent=(state.changes||[]).filter(x=>!x.read).length;
  $('#count-briefing').textContent=(state.briefings||[]).length;
  const controls=$('#feature-controls');controls.hidden=!['move','changes','briefing','logs','newsletters','artifacts'].includes(source);controls.replaceChildren();
  if(controls.hidden)return false;
  const query=$('#search').value.toLowerCase(),list=$('#work-list');list.replaceChildren();
  if(['newsletters','artifacts'].includes(source)){renderNewsletterView(list,controls,query);return true;}
  if(source==='logs'){renderLogsView(list,controls,query);return true;}
  if(source==='move'){
    controls.append(el('p','form-hint','Requested reviews, failing CI, conflicts, requested changes, and merge-ready work. Based on source state.'));
    const rows=active.filter(x=>x.actionReasons?.length&&[x.title,workReference(x),x.project,...x.actionReasons].join(' ').toLowerCase().includes(query));
    for(const reason of [...new Set(rows.flatMap(x=>x.actionReasons))]){
      const group=el('div','group'),matching=rows.filter(x=>x.actionReasons[0]===reason);if(!matching.length)continue;
      group.append(el('h2','group-header',`${reason} · ${matching.length}`),...matching.map(renderRow));list.append(group);
    }
    if(!rows.length)list.append(el('p','feature-empty','Nothing needs your move in this view.'));
  }else if(source==='changes'){
    const visible=(state.changes||[]).filter(x=>[x.title,x.message,x.reference].join(' ').toLowerCase().includes(query)),unreadIds=visible.filter(change=>!change.read).map(change=>change.id),ids=unreadIds.slice(0,500);
    const mark=featureButton(unreadIds.length>500?'Mark first 500 visible read':'Mark visible read',event=>featureWrite(null,'Visible changes marked read.',{button:event.currentTarget,write:()=>markChangesRead(ids)}),'text-button');mark.disabled=!ids.length;
    controls.append(el('p','form-hint','Source changes observed since tracking started. First sync establishes a baseline; counters and bot timestamps stay quiet.'),mark);
    for(const change of visible){
      const row=el('article','change-row'+(change.read?' read':''));
      const current=items.find(x=>x.id===change.itemId);
      const stamp=el('time','change-timestamp');stamp.dateTime=new Date(change.at*1000).toISOString();stamp.title=dated(change.at);stamp.setAttribute('aria-label',dated(change.at));
      stamp.append(el('span','',new Date(change.at*1000).toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit'})),el('small','',new Date(change.at*1000).toLocaleDateString(undefined,{month:'short',day:'numeric'})));
      const title=el(change.url?'a':'button','change-title');
      const reference=workReference(change)||sourceNames[change.source];
      title.append(el('span','change-ref',reference),el('span','change-title-text',change.title));title.title=reference+' · '+change.title;title.setAttribute('aria-label',reference+' · '+change.title);
      if(change.url){title.href=change.url;title.target='_blank';title.rel='noopener noreferrer';title.onclick=()=>{if(!change.read)void markObservedChange(change,title);};}
      else{title.type='button';title.onclick=()=>markObservedChange(change,title,true);}
      if(/^(?:chore|build|ci|style)(?:\([^)]*\))?!?:|^(?:update|bump)\s/i.test(change.title))title.classList.add('maintenance-title');
      if(current?.hidden||current?.snoozed)title.title+=current.hidden?' · Hidden work':' · Snoozed work';
      const transitions=el('div','change-transitions');
      for(const part of change.message.split(' · ')){
        const group=el('span','change-transition-group'),pair=part.split(' → ');
        pair.forEach((value,index)=>{if(index)group.append(el('span','change-arrow','→'));const pill=changePill(value);pill.title=value;group.append(pill);});
        transitions.append(group);
      }
      row.append(stamp,transitions,title);
      if(!change.read){const read=featureButton('✓',event=>markObservedChange(change,event.currentTarget),'icon-button change-read');read.title='Mark read';read.setAttribute('aria-label','Mark change read: '+change.title);row.append(read);}
      else row.append(el('span','change-read read-indicator','✓'));
      list.append(row);
    }
    if(!list.children.length)list.append(el('p','feature-empty','No changes observed yet. Future syncs will record status, CI, review, and conflict changes here.'));
  }else{
    controls.append(featureButton('Generate briefing ↗',()=>$('#summarize-work').click()),featureButton('Paste a report…',()=>openBriefingEditor(),'text-button'),el('p','form-hint','Reports are generated on demand. Nothing runs automatically. Keep up to 30 reports.'));
    for(const report of (state.briefings||[]).filter(x=>x.body.toLowerCase().includes(query))){
      const card=el('article','briefing-card');card.append(el('h2','',dated(report.at)));
      const session=state.sessions.find(x=>x.id===report.sessionId);
      if(session)card.append(featureButton('Continue review ↗',()=>openSavedSession(session),'text-button'));
      const syncs=Object.entries(report.sourceTimes||{}).map(([key,value])=>`${sourceNames[key]}: ${value?dated(value):'not synced'}`);
      card.append(el('p','form-hint','Cache at review start: '+syncs.join(' · ')));
      const body=el('div','briefing-body');appendReportText(body,report.body);card.append(body);list.append(card);
    }
    if(!list.children.length)list.append(el('p','feature-empty','Your next Summarize report will appear here when the agent saves it. Existing reports can be pasted above.'));
  }
  return true;
}
function appendReportText(node,text){
  // Only HTML produced by the restricted local Markdown renderer reaches this sink.
  const references=[...state.items,...(state.relatedItems||[]),...(state.sessions||[]).flatMap(session=>session.items)];
  node.innerHTML=renderBriefingMarkdown(text,references);
}

function openBriefingEditor(){
  const reviews=state.sessions.filter(x=>x.workflow==='work-review');
  if(!reviews.length){toast('Start Summarize first to create a Work review session.');return;}
  if(!requestDialogNavigation())return false;const form=$('#briefing-form');form.reset();form.elements.record.replaceChildren(...reviews.slice().reverse().map(x=>new Option(`${dated(x.createdAt)} · ${agentName(x.agent)}`,x.id)));
  disposeFeatureForm($('#briefing-dialog'));createFeatureForm($('#briefing-dialog'),form,null);$('#briefing-dialog').showModal();
}
$('#briefing-form').onsubmit=async event=>{event.preventDefault();const form=event.currentTarget,current=featureForms.get($('#briefing-dialog'));if(!featureFormMutable(current))return;const id=form.elements.record.value,body=form.elements.body.value;current.target=id;current.targetCurrent=()=>form.elements.record.value===id;await submitFeatureForm(current,{action:'save-briefing',id,body},'Briefing saved.',()=>$('#briefing-dialog').close());};
let checkpointSession=null;
function openCheckpoint(session){
  if(!requestDialogNavigation())return false;disposeFeatureForm($('#checkpoint-dialog'));checkpointSession=session;const form=$('#checkpoint-form');form.reset();
  for(const key of ['done','blocked','next'])form.elements[key].value=session.checkpoint?.[key]||'';
  $('#checkpoint-session').textContent=session.title;createFeatureForm($('#checkpoint-dialog'),form,session.id);$('#checkpoint-dialog').showModal();
}
function renderCheckpoint(session){
  let section=$('#session-checkpoint');
  if(!section){section=el('section','issue-family');section.id='session-checkpoint';$('#session-dialog').insertBefore(section,$('#session-context').closest('details'));}
  section.replaceChildren(el('h3','','Handoff checkpoint'));
  if(session.checkpoint){section.append(el('p','form-hint',dated(session.checkpoint.at)));for(const key of ['done','blocked','next'])if(session.checkpoint[key])section.append(el('strong','',key==='next'?'Next step':key==='done'?'Done':'Blocked on'),el('p','checkpoint-text',session.checkpoint[key]));}
  else section.append(el('p','form-hint','Save where you left off. New agents can update this checkpoint with the local helper.'));
  section.append(featureButton('Edit checkpoint',()=>openCheckpoint(session),'text-button'));
}
$('#checkpoint-form').onsubmit=async event=>{event.preventDefault();const form=event.currentTarget,current=featureForms.get($('#checkpoint-dialog'));if(!featureFormMutable(current))return;const id=current.target,payload={action:'checkpoint',id};for(const key of ['done','blocked','next'])payload[key]=form.elements[key].value;await submitFeatureForm(current,payload,'Checkpoint saved.',()=>{const session=state.sessions.find(row=>row.id===id);if(!session)throw new Error('The saved chat is unavailable in this view.');$('#checkpoint-dialog').close();return showSession(session);});};
let paletteAnchor=null,paletteEntries=[],paletteSelected=0,paletteGeneration=0,paletteMutationBlocked=false;
function openPalette(anchor=null){
  if(!requestDialogNavigation())return false;paletteGeneration++;paletteMutationBlocked=false;paletteAnchor=anchor;$('#command-query').value='';$('#command-title').textContent=anchor?'Link related work':'Find & act';$('#command-actions').replaceChildren();
  $('#command-dialog').showModal();renderPalette();$('#command-query').focus();
}
$('#command-open').onclick=()=>openPalette();
function renderPalette(){
  const query=$('#command-query').value.toLowerCase().trim();
  const entries=allItems().map(item=>({item,title:`${workReference(item)||sourceNames[item.source]} · ${item.title}`,extra:item.project||''}));
  if(!paletteAnchor)for(const session of state.sessions||[])entries.push({session,title:session.title,extra:`${agentName(session.agent)} · ${session.cwd}`});
  paletteEntries=entries.filter(x=>(!paletteAnchor||x.item?.id!==paletteAnchor.id)&&[x.title,x.extra].join(' ').toLowerCase().includes(query)).sort((a,b)=>Number(!!b.session)-Number(!!a.session)||Number(!!b.item?.actionReasons?.length)-Number(!!a.item?.actionReasons?.length)).slice(0,40);
  paletteSelected=0;$('#command-actions').replaceChildren();paintPalette();
}
function paletteMetadata(entry){
  const meta=el('div','palette-result-meta'),item=entry.item,session=entry.session;
  if(item){
    meta.append(el('span','status-pill '+item.status,workStatusLabel(item)),el('span','',item.project||sourceNames[item.source]));
    const role=githubRole(item);if(role)meta.append(el('span','',role.label));
    if(item.ci){const info=ciInfo(item.ci);meta.append(el('span','ci-mark '+item.ci.state,info.label));}
    if(item.eventCount)meta.append(el('span','',Number(item.eventCount).toLocaleString()+' events'));
    if(item.hidden||item.snoozed)meta.append(el('span','',item.hidden?'Hidden':'Snoozed'));
  }else{
    meta.append(el('span','palette-agent',agentName(session.agent)),el('span','',sessionRuntime(session).label),el('span','palette-directory',session.cwd||''));
    if(session.archivedAt)meta.append(el('span','','Archived'));
  }
  const updated=item?.updatedAt||session?.updatedAt;
  const time=typeof updated==='number'?updated:Date.parse(updated)/1000;
  if(Number.isFinite(time)&&time>0){const date=el('span','palette-updated','Updated '+relative(time));date.title=dated(time);meta.append(date);}
  return meta;
}
function paintPalette(){
  const list=$('#command-results');list.replaceChildren();
  paletteEntries.forEach((entry,index)=>{
    const button=featureButton('',()=>{paletteSelected=index;paintPalette();choosePalette();},'command-result'+(index===paletteSelected?' selected':''));
    button.setAttribute('role','option');button.setAttribute('aria-selected',String(index===paletteSelected));button.id='command-option-'+index;
    const title=el('div','palette-result-heading');
    if(entry.item)title.append(el('span','palette-reference',workReference(entry.item)||sourceNames[entry.item.source]));
    else title.append(el('span','palette-reference','Agent'));
    title.append(el('span','palette-result-title',entry.item?.title||entry.session.title));
    button.append(title,paletteMetadata(entry));list.append(button);
  });
  $('#command-query').setAttribute('aria-activedescendant',paletteEntries.length?'command-option-'+paletteSelected:'');
  $('#command-hint').textContent=paletteEntries.length?`↑↓ results · Enter actions · ←→ actions · Esc close · ${paletteEntries.length}${paletteEntries.length===40?'+':''} ${paletteEntries.length===1?'match':'matches'}`:'No matches. Try a title, repo, or ticket ID.';
  paintPaletteActions(false);
}
async function choosePalette(){
  const entry=paletteEntries[paletteSelected];if(!entry)return;
  if(paletteAnchor){if(paletteMutationBlocked)return;paletteMutationBlocked=true;const id=paletteAnchor.id,itemId=entry.item.id,generation=paletteGeneration,result=await featureWrite({action:'link-work',itemIds:[id,itemId]},'Work linked.',{isCurrent:()=>$('#command-dialog').open&&paletteGeneration===generation&&paletteAnchor?.id===id,afterReady:()=>{const anchor=allItems().find(row=>row.id===id);if(anchor){$('#command-dialog').close();openItem(anchor);}}});if(paletteGeneration===generation&&result.kind==='rejected')paletteMutationBlocked=false;return;}
  paintPaletteActions(true);
}
function paintPaletteActions(focus){
  const entry=paletteEntries[paletteSelected],actions=$('#command-actions'),preview=$('#command-preview');actions.replaceChildren();preview.replaceChildren();preview.hidden=true;
  if(!entry||paletteAnchor)return;
  if(entry.session){actions.append(featureButton('Continue agent ↗',()=>{if(requestDialogNavigation())return openSavedSession(entry.session);}),featureButton('Details',()=>showSession(entry.session)),featureButton('Edit checkpoint',()=>openCheckpoint(entry.session)));}
  else{
    const item=entry.item;
    actions.append(featureButton('Details',()=>openItem(item)));
    if(item.url){const link=el('a','button palette-source-action','Open source ↗');link.href=item.url;link.target='_blank';link.rel='noopener noreferrer';link.onclick=event=>{if(!requestDialogNavigation())event.preventDefault();};actions.append(link);}
    const session=linkedSessions(item)[0];actions.append(featureButton(session?'Continue agent ↗':'Start agent ↗',()=>{if(requestDialogNavigation())return session?openSavedSession(session):quickStart(item);}));
    const generation=paletteGeneration,hidden=!item.hidden,hide=featureButton(hidden?'Hide':'Show',()=>featureWrite({action:'override',id:item.id,value:{hidden}},hidden?'Hidden.':'Shown again.',{button:hide,isCurrent:()=>$('#command-dialog').open&&paletteGeneration===generation&&hide.isConnected,afterReady:()=>$('#command-dialog').close()}));actions.append(featureButton('Snooze',()=>openSnooze(item)),hide);
    const reviewers=(item.requestedReviewers||[]).map(person=>'@'+person.login);
    if(reviewers.length)preview.append(el('span','', 'Reviewers: '+reviewers.join(', ')));
    if(session)preview.append(el('span','',agentName(session.agent)+' · '+session.title));
    if(item.actionReasons?.length)preview.append(el('span','',item.actionReasons.join(' · ')));
    preview.hidden=!preview.childElementCount;
  }
  if(focus)actions.querySelector('button,a')?.focus();
}
function handlePaletteKey(event){
  if(event.isComposing||event.metaKey||event.ctrlKey||event.altKey)return;
  if(['ArrowDown','ArrowUp'].includes(event.key)){
    event.preventDefault();paletteSelected=(paletteSelected+(event.key==='ArrowDown'?1:-1)+paletteEntries.length)%Math.max(1,paletteEntries.length);paintPalette();$('#command-query').focus();$('#command-results').children[paletteSelected]?.scrollIntoView({block:'nearest'});
  }else if(event.key==='Enter'&&event.target===$('#command-query')){event.preventDefault();void choosePalette();}
  else if(['ArrowLeft','ArrowRight'].includes(event.key)&&event.target.closest('#command-actions')){
    const actions=[...$('#command-actions').querySelectorAll('button:not(:disabled),a[href]')];if(!actions.length)return;
    event.preventDefault();const index=actions.indexOf(document.activeElement);actions[(index+(event.key==='ArrowRight'?1:-1)+actions.length)%actions.length].focus();
  }
}
$('#command-query').oninput=renderPalette;
$('#command-dialog').addEventListener('keydown',handlePaletteKey);
document.addEventListener('keydown',event=>{if(!(typeof appSettings!=='undefined'&&appSettings)&&(/Mac|iPhone|iPad/.test(navigator.platform)?event.metaKey:event.ctrlKey)&&event.key.toLowerCase()==='p'&&!event.shiftKey){event.preventDefault();if($('#command-dialog').open)requestDialogClose($('#command-dialog'));else openPalette();}});

let logData=null,logLoading=false,logLastFetch=0,logLoadError='';
const logFilters={source:'',outcome:'failure'},openLogEntries=new Set();
async function loadLogs(force=false){
  if(logLoading||!force&&Date.now()-logLastFetch<8000)return;
  logLoading=true;logLastFetch=Date.now();
  try{logData=await api('/api/logs');logLoadError='';}
  catch(error){logLoadError=error.message;}
  finally{logLoading=false;if(source==='logs')render();}
}
function logAppChatRows(entry){return typeof appChatDiagnosticRows==='function'?appChatDiagnosticRows({id:entry.requestId,diagnostics:entry.diagnostics}):[];}
function logSummary(entry){
  if(entry.source==='app-chat'){
    if(entry.outcome==='success')return 'App chat completed';
    const reasons={spawn_failed:'Could not start the CLI',process_failed:'CLI process failed',process_wait_failed:'Could not read CLI process status',timed_out:'App chat timed out',cancelled:'App chat stopped',oversized_output:'CLI output exceeded the limit',invalid_utf8:'CLI output is not valid text',invalid_cli_json:'CLI output is not valid JSON',cli_reported_error:'CLI reported an error',missing_result:'CLI returned no reply',invalid_assistant_json:'Assistant reply is not valid JSON',invalid_assistant_shape:'Assistant reply has an unsupported format',tool_limit:'App chat reached its tool limit',failed:'App chat failed'},stages={launch:'launch',process:'CLI process','cli-envelope':'CLI result','assistant-response':'assistant reply',complete:'completion'};
    const stage=stages[entry.diagnostics?.stage];return (reasons[entry.diagnostics?.code]||'App chat failed')+(stage?' · '+stage:'');
  }
  if(entry.outcome==='success')return `${entry.operation==='environment-sync'?'Environment sync':'Sync'} completed${entry.itemCount!==undefined?' · '+Number(entry.itemCount).toLocaleString()+' items':''}`;
  const reasons={timeout:'Request timed out',dns:'DNS lookup failed','tls-certificate':'TLS certificate verification failed',tls:'TLS connection failed',connection:'Connection interrupted',protocol:'Response interrupted',network:'Network failure',http:'HTTP error',authentication:'Authentication rejected','rate-limit':'Rate limited','cli-exit':'GitHub CLI failed',graphql:'GraphQL query failed','invalid-json':'Invalid JSON response','not-installed':'GitHub CLI not installed','local-execution':'Could not start GitHub CLI'};
  return (reasons[entry.kind]||'Sync failed · reason not recorded')+(entry.httpStatus?' · HTTP '+entry.httpStatus:'')+(entry.page?' · page '+entry.page:'');
}
function renderLogsView(list,controls,query){
  const focused=document.activeElement?.dataset.focusKey;
  const sourceSelect=el('select');sourceSelect.setAttribute('aria-label','Log source');sourceSelect.dataset.focusKey='logs:source';
  const sources=[...new Set(['github','linear','sentry',...(logData?.entries||[]).map(entry=>entry.source),logFilters.source].filter(key=>typeof key==='string'&&/^[a-z][a-z0-9-]{0,63}$/.test(key)))];
  sourceSelect.replaceChildren(new Option('All sources',''),...sources.map(key=>new Option(key==='app-chat'?'App chat':sourceNames[key]||key,key)));sourceSelect.value=logFilters.source;
  sourceSelect.onchange=()=>{logFilters.source=sourceSelect.value;render();};
  const outcome=el('select');outcome.setAttribute('aria-label','Log outcome');outcome.dataset.focusKey='logs:outcome';outcome.replaceChildren(new Option('Failures','failure'),new Option('All outcomes',''),new Option('Successes','success'));outcome.value=logFilters.outcome;outcome.onchange=()=>{logFilters.outcome=outcome.value;render();};
  const refresh=featureButton(logLoading?'Loading…':'↻ Refresh logs',()=>loadLogs(true),'text-button');refresh.disabled=logLoading;
  controls.append(sourceSelect,outcome,refresh,el('span','form-hint','Local diagnostics · auto-refresh while open · latest 500 retained entries'));
  for(const [key,value] of Object.entries(state.sources))if(value.error&&!value.disabled)list.append(el('p','log-current-error',`${sourceNames[key]}: ${value.error} Next retry: ${dated(value.nextAttempt)}.`));
  list.append(el('p','form-hint log-location','data/logs/sync.jsonl · 512 KiB × 3 files · tokens, queries, and response bodies are excluded. Times below use your local timezone.'));
  if(typeof renderPerformancePanel==='function')list.append(renderPerformancePanel());
  if(logLoadError)list.append(el('p','log-current-error',logLoadError));
  const rows=(logData?.entries||[]).filter(entry=>(!logFilters.source||entry.source===logFilters.source)&&(!logFilters.outcome||entry.outcome===logFilters.outcome)&&[...Object.values(entry),...(entry.source==='app-chat'?logAppChatRows(entry).flat():[])].join(' ').toLowerCase().includes(query));
  for(const entry of rows){
    const key=[entry.time,entry.source,entry.operation,entry.outcome,entry.elapsedMs,entry.requestMs,entry.itemCount,entry.requestId].join('|');
    const details=el('details','log-entry');details.open=openLogEntries.has(key);details.ontoggle=()=>{if(!details.isConnected)return;if(details.open)openLogEntries.add(key);else openLogEntries.delete(key);};
    const summary=el('summary','log-summary'),date=new Date(entry.time),valid=Number.isFinite(date.getTime());
    const time=el('time','log-timestamp',valid?date.toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit',second:'2-digit'}):'Unknown time');
    if(valid){time.dateTime=entry.time;time.title=date.toLocaleString();time.append(el('small','',date.toLocaleDateString(undefined,{month:'short',day:'numeric'})));}
    const badge=el('span','log-outcome '+(entry.outcome==='success'?'success':'failure'),entry.outcome==='success'?'Success':'Failure');
    const diagnosticRows=entry.source==='app-chat'?logAppChatRows(entry):[],duration=diagnosticRows.find(([label])=>label==='Duration')?.[1]||(entry.elapsedMs!==undefined?(entry.elapsedMs/1000).toFixed(1)+'s':'');
    summary.append(time,el('span','log-source',entry.source==='app-chat'?'App chat':sourceNames[entry.source]||entry.source||'Unknown'),badge,el('span','log-message',logSummary(entry)),el('span','log-duration',duration));
    details.append(summary);
    const body=el('div','log-details');
    if(entry.source!=='app-chat'&&entry.outcome==='failure'&&!entry.kind)body.append(el('p','form-hint','This older entry retained sync timing but no request-level cause. New failures include transport, deadline, and source-specific details.'));
    const labels={transport:'Transport',phase:'Request phase',kind:'Failure category',scope:'Search scope',page:'Page',exceptionType:'Application exception',reasonType:'Underlying exception',httpStatus:'HTTP status',timeoutSeconds:'Request deadline (s)',requestMs:'Failed request duration (ms)',elapsedMs:'Total sync duration (ms)',retrySeconds:'Retry delay (s)',receivedItems:'Items received before failure',itemCount:'Items synced',host:'Host',exitCode:'CLI exit code',errno:'OS error code',tlsVerifyCode:'TLS verification code',operation:'Operation'};
    const values=el('dl','log-fields');
    if(entry.source==='app-chat'){for(const [label,value]of diagnosticRows)values.append(el('dt','',label),el('dd','',value));}
    else for(const [field,label] of Object.entries(labels))if(entry[field]!==undefined&&entry[field]!==null)values.append(el('dt','',label),el('dd','',String(entry[field])));
    const displayedEntry=entry.source==='app-chat'?{source:'app-chat',outcome:entry.outcome==='success'?'success':'failure',time:valid?date.toISOString():null,metadata:Object.fromEntries(diagnosticRows)}:entry;
    body.append(values);
    const raw=el('details','log-raw'),rawSummary=el('summary','',entry.source==='app-chat'?'Diagnostic JSON':'Raw JSON');raw.append(rawSummary,el('pre','',JSON.stringify(displayedEntry,null,2)));body.append(raw);
    const copy=featureButton('Copy JSON',async()=>{try{await navigator.clipboard.writeText(JSON.stringify(displayedEntry,null,2));toast('Log entry copied.');}catch(_){toast('Copy unavailable. Expand '+(entry.source==='app-chat'?'Diagnostic JSON':'Raw JSON')+' to select the entry.');}},'text-button');body.append(copy);details.append(body);list.append(details);
  }
  if(!rows.length)list.append(el('p','feature-empty',logLoading?'Loading local log entries…':logLoadError?'Logs could not be loaded.':logData?'No matching entries. Try All outcomes or another source.':'No logs loaded yet.'));
  if(focused)for(const control of controls.querySelectorAll('[data-focus-key]'))if(control.dataset.focusKey===focused)control.focus({preventScroll:true});
  loadLogs();
}
