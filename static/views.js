'use strict';
let savedViewsDocument=null,savedViewsError='',savedViewEditor=null;
const savedViewPages=['all','github','linear','sentry','notes'];
const savedViewScalarFields=['source','kind','status','sourceStatus','scope'];
const savedViewSetOps=['containsAny','containsAll','containsNone'];
const savedViewLifecycles=['active','completed','any'];
const savedViewGroups=['status','source','none','project','stack'];
const savedViewStorageKey='truffle-saved-view-state';
let savedViewPageStates={},savedViewRenderedPage=null,savedViewMigrationComplete=false;
try{savedViewMigrationComplete=localStorage.getItem('truffle-saved-view-migration')==='1';}catch(_){}
try{const stored=JSON.parse(localStorage.getItem(savedViewStorageKey)||'{}');if(stored&&typeof stored==='object'&&!Array.isArray(stored))savedViewPageStates=stored;}catch(_){}
function cloneSavedView(value){return JSON.parse(JSON.stringify(value));}
function savedViewText(value,max){return typeof value==='string'&&value.trim().length>0&&new TextEncoder().encode(value).length<=max&&!value.includes('\0');}
const savedViewDateFields=['updatedAt','createdAt'];
const savedViewDateOps=['withinLast','olderThan','beforeDate','onOrAfterDate','calendarPeriod'];
const savedViewCalendarPeriods={today:'Today',yesterday:'Yesterday',sinceYesterday:'Since yesterday',thisWeek:'This week',lastWeek:'Last week'};
const savedViewCalendarFormatters=new Map();
let savedViewTimeZoneInputId=0;
function savedViewCalendarFormatter(timezone){
 if(typeof timezone!=='string'||timezone.length<1||timezone.length>100||!/^[-A-Za-z0-9_+]+(?:\/[-A-Za-z0-9_.+]+)*$/.test(timezone))return null;
 if(savedViewCalendarFormatters.has(timezone))return savedViewCalendarFormatters.get(timezone);
 try{const formatter=new Intl.DateTimeFormat('en-US-u-ca-iso8601-nu-latn',{timeZone:timezone,year:'numeric',month:'2-digit',day:'2-digit',era:'short'});if(savedViewCalendarFormatters.size>=32)savedViewCalendarFormatters.clear();savedViewCalendarFormatters.set(timezone,formatter);return formatter;}catch{return null;}
}
function savedViewLocalTimeZone(){const zone=Intl.DateTimeFormat().resolvedOptions().timeZone;return savedViewCalendarFormatter(zone)?zone:'UTC';}
function savedViewCalendarDay(seconds,timezone){
 const formatter=savedViewCalendarFormatter(timezone);if(!formatter||!Number.isFinite(seconds))return NaN;
 try{const parts=Object.fromEntries(formatter.formatToParts(new Date(seconds*1000)).map(part=>[part.type,part.value]));const date=new Date(0),year=Number(parts.year);date.setUTCFullYear(parts.era==='BC'?1-year:year,Number(parts.month)-1,Number(parts.day));date.setUTCHours(0,0,0,0);return date.getTime()/86400000;}catch{return NaN;}
}
function matchesSavedViewCalendar(filter,value,nowSeconds){
 const today=savedViewCalendarDay(nowSeconds,filter.timezone),day=savedViewCalendarDay(value,filter.timezone);if(!Number.isFinite(today)||!Number.isFinite(day))return false;
 const monday=today-(new Date(today*86400000).getUTCDay()+6)%7;
 const windows={today:[today,today+1],yesterday:[today-1,today],sinceYesterday:[today-1,today+1],thisWeek:[monday,monday+7],lastWeek:[monday-7,monday]},window=windows[filter.period];return Boolean(window&&day>=window[0]&&day<window[1]);
}
function savedViewDateSeconds(date){
 if(typeof date!=='string'||!/^\d{4}-\d{2}-\d{2}$/.test(date))return NaN;
 const value=Date.parse(date+'T00:00:00Z');return Number.isFinite(value)&&new Date(value).toISOString().slice(0,10)===date?value/1000:NaN;
}
function savedViewTimestamp(value){
 if(typeof value==='number')return Number.isFinite(value)?value:NaN;
 if(typeof value!=='string'||!/^\d{4}-\d{2}-\d{2}[T ](?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)$/i.test(value)||!Number.isFinite(savedViewDateSeconds(value.slice(0,10))))return NaN;
 return Date.parse(value)/1000;
}
function validateSavedViewFilter(filter,depth=0,budget={nodes:0}){
 if(!filter||typeof filter!=='object'||Array.isArray(filter)||depth>4||++budget.nodes>128)throw new Error('Filters support at most four nested levels and 128 conditions.');
 const keys=Object.keys(filter),allowed=(names)=>keys.every(key=>names.includes(key));
 if(['all','any'].includes(filter.op)){
  if(!allowed(['op','conditions'])||!Array.isArray(filter.conditions)||filter.conditions.length>32)throw new Error('A filter group supports at most 32 conditions.');
  for(const condition of filter.conditions)validateSavedViewFilter(condition,depth+1,budget);
 }else if(['in','notIn',...savedViewSetOps].includes(filter.op)){
  if(!allowed(['op','field','values'])||!(['in','notIn'].includes(filter.op)?savedViewScalarFields.includes(filter.field):filter.field==='relations')||!Array.isArray(filter.values)||filter.values.length===0||filter.values.length>64||filter.values.some(value=>!savedViewText(value,500))||new Set(filter.values).size!==filter.values.length)throw new Error('Choose a supported field and one or more distinct values.');
 }else if(filter.op==='calendarPeriod'){
  if(!allowed(['op','field','period','timezone'])||!savedViewDateFields.includes(filter.field)||!Object.hasOwn(savedViewCalendarPeriods,filter.period)||!savedViewCalendarFormatter(filter.timezone))throw new Error('Choose a calendar period and a supported IANA time zone.');
 }else if(savedViewDateOps.includes(filter.op)){
  const relative=['withinLast','olderThan'].includes(filter.op);
  if(!allowed(relative?['op','field','days']:['op','field','date'])||!savedViewDateFields.includes(filter.field)||(relative?(!Number.isInteger(filter.days)||filter.days<1||filter.days>3650):!Number.isFinite(savedViewDateSeconds(filter.date))))throw new Error('Choose a valid UTC date or between 1 and 3650 days.');
 }else if(filter.op==='equals'){
  if(!allowed(['op','field','value'])||filter.field!=='pinned'||typeof filter.value!=='boolean')throw new Error('Pinned must be true or false.');
 }else throw new Error('Unsupported filter operator.');
 return filter;
}
function validateSavedViewsDocument(doc){
 if(!doc||doc.version!==1||!Number.isSafeInteger(doc.revision)||doc.revision<0||!Array.isArray(doc.views)||doc.views.length>64||typeof doc.contentHash!=='string'||!/^[a-f0-9]{64}$/.test(doc.contentHash)||Object.keys(doc).some(key=>!['version','revision','contentHash','views'].includes(key)))throw new Error('Unsupported saved views document.');
 if(new TextEncoder().encode(JSON.stringify(doc)).length>65536)throw new Error('Saved views exceed 64 KiB.');
 const ids=new Set();
 for(const view of doc.views){
  if(!view||Object.keys(view).some(key=>!['id','name','parent','lifecycle','filter','query','groupBy'].includes(key))||!savedViewText(view.id,80)||!/^[a-zA-Z0-9_-]+$/.test(view.id)||ids.has(view.id)||!savedViewText(view.name,120)||!savedViewPages.includes(view.parent)||!savedViewLifecycles.includes(view.lifecycle)||typeof view.query!=='string'||new TextEncoder().encode(view.query).length>1000||view.query.includes('\0')||!savedViewGroups.includes(view.groupBy)||view.groupBy==='stack'&&view.parent!=='github')throw new Error('Invalid saved view.');
  ids.add(view.id);validateSavedViewFilter(view.filter);
 }
 return doc;
}
function savedViewField(item,field){
 if(field==='scope'){
  if(item.source==='github'){const match=typeof item.url==='string'&&item.url.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\//);if(match)return match[1];}
  return typeof item.project==='string'&&item.project.length?item.project:undefined;
 }
 return typeof item[field]==='string'?item[field]:undefined;
}
function matchesSavedViewFilter(filter,item,nowSeconds=Date.now()/1000){
 switch(filter.op){
  case 'all':return filter.conditions.every(condition=>matchesSavedViewFilter(condition,item,nowSeconds));
  case 'any':return filter.conditions.some(condition=>matchesSavedViewFilter(condition,item,nowSeconds));
  case 'in':{const value=savedViewField(item,filter.field);return value!==undefined&&filter.values.includes(value);}
  case 'notIn':{const value=savedViewField(item,filter.field);return value!==undefined&&!filter.values.includes(value);}
  case 'containsAny':case 'containsAll':case 'containsNone':{
   const values=item[filter.field];if(!Array.isArray(values)||values.some(value=>typeof value!=='string'))return false;
   if(filter.op==='containsAny')return filter.values.some(value=>values.includes(value));
   return filter.op==='containsAll'?filter.values.every(value=>values.includes(value)):filter.values.every(value=>!values.includes(value));
  }
  case 'calendarPeriod':return matchesSavedViewCalendar(filter,savedViewTimestamp(item[filter.field]),nowSeconds);
  case 'withinLast':case 'olderThan':case 'beforeDate':case 'onOrAfterDate':{
   const value=savedViewTimestamp(item[filter.field]);if(!Number.isFinite(value))return false;
   const boundary=['withinLast','olderThan'].includes(filter.op)?nowSeconds-filter.days*86400:savedViewDateSeconds(filter.date);
   if(!Number.isFinite(boundary))return false;
   return filter.op==='withinLast'?value>=boundary&&value<=nowSeconds:filter.op==='olderThan'?value<boundary:filter.op==='beforeDate'?value<boundary:value>=boundary;
  }
  case 'equals':return typeof item[filter.field]==='boolean'&&item[filter.field]===filter.value;
  default:throw new Error('Unsupported filter operator.');
 }
}
function savedViewMatches(view,item,nowSeconds=Date.now()/1000){
 return (view.parent==='all'||item.source===view.parent)&&(view.lifecycle==='any'||(view.lifecycle==='completed'?item.status==='done':item.status!=='done'))&&matchesSavedViewFilter(view.filter,item,nowSeconds);
}
function defaultSavedViewState(page){
 const old=typeof prefs==='object'?prefs:{},legacy=old.status,group=typeof groupingPrefs==='object'?groupingPrefs[page]:'status';
 const draft={parent:page,lifecycle:legacy==='all'?'any':'active',filter:{op:'all',conditions:[]},query:'',groupBy:savedViewGroups.includes(group)&&(group!=='stack'||page==='github')?group:'status'};
 const scope=old.scopes?.[page];if(typeof scope==='string'&&scope)draft.filter.conditions.push({op:'in',field:'scope',values:[scope]});
 if(legacy&&legacy!=='active'&&legacy!=='all'){draft.lifecycle='any';draft.filter.conditions.push({op:'in',field:'status',values:[legacy]});}
 return {id:'',draft};
}
function currentSavedViewState(page=source){
 if(!savedViewPages.includes(page))return null;
 let state=savedViewPageStates[page];
 try{if(!state||typeof state.id!=='string'||!state.draft||state.draft.parent!==page||!savedViewLifecycles.includes(state.draft.lifecycle)||typeof state.draft.query!=='string'||!savedViewGroups.includes(state.draft.groupBy))throw new Error();validateSavedViewFilter(state.draft.filter);}catch(_){state=defaultSavedViewState(page);savedViewPageStates[page]=state;}
 return state;
}
function resetSavedViewState(page=source){
 const state=currentSavedViewState(page);if(!state)return;
 state.id='';delete state.base;delete state.stale;state.draft={parent:page,lifecycle:'active',filter:{op:'all',conditions:[]},query:'',groupBy:state.draft.groupBy};applySavedViewPresentation(state.draft);persistSavedViewState();
}
function prepareSavedViewPage(){
 if(savedViewRenderedPage===source)return;
 savedViewRenderedPage=source;savedViewUsePage(source);
}
function setSavedViewScope(scope,page=source){
 const state=currentSavedViewState(page);if(!state)return;
 const previous=state.draft.filter,conditions=previous.op==='all'?previous.conditions:[previous];
 state.draft.filter={op:'all',conditions:conditions.filter(condition=>condition.field!=='scope')};
 if(scope)state.draft.filter.conditions.push({op:'in',field:'scope',values:[scope]});persistSavedViewState();
}
function setSavedViewStatus(status,page=source){
 const state=currentSavedViewState(page);if(!state)return;savedViewCapturePresentation(page);
 const previous=state.draft.filter,conditions=previous.op==='all'?previous.conditions:[previous];
 state.draft.filter={op:'all',conditions:conditions.filter(condition=>condition.field!=='status')};
 state.draft.lifecycle=status==='active'?'active':'any';
 if(status!=='active')state.draft.filter.conditions.push({op:'in',field:'status',values:[status]});
 persistSavedViewState();
}
function persistSavedViewState(){try{localStorage.setItem(savedViewStorageKey,JSON.stringify(savedViewPageStates));}catch(_){} }
function savedViewAllows(item,page=source){
 const state=currentSavedViewState(page);if(!state)return true;
 if(savedViewsError)return false;
 return savedViewMatches(state.draft,item);
}
function savedViewSearchAllows(query,item){return Number.isFinite(workSearchScore(query,item));}
function savedViewCount(view,items,nowSeconds=Date.now()/1000){return items.filter(item=>!item.archivedAt&&!item.hidden&&!item.snoozed&&savedViewMatches(view,item,nowSeconds)&&savedViewSearchAllows(view.query,item)&&(typeof matchesSentry!=='function'||matchesSentry(item,view.parent))&&(typeof pageSettingsAllows!=='function'||pageSettingsAllows(item,view.parent))).length;}
function savedViewRefresh(){persistSavedViewState();if(typeof render==='function')render();}
function applySavedViewPresentation(draft){
 if(typeof scopePrefs==='object'&&Object.hasOwn(scopePrefs,draft.parent)){const filter=draft.filter,conditions=filter.op==='all'?filter.conditions:[filter],scope=conditions.find(condition=>condition.op==='in'&&condition.field==='scope'&&condition.values.length===1);scopePrefs[draft.parent]=scope?scope.values[0]:'';}
 const query=document.querySelector('#search');if(query)query.value=draft.query;
 if(typeof groupingPrefs==='object')groupingPrefs[draft.parent]=draft.groupBy;
 if(typeof persist==='function')persist();
}
function activateSavedView(id){
 const view=savedViewsDocument?.views.find(view=>view.id===id);if(!view)return;
 savedViewPageStates[view.parent]={id:view.id,draft:cloneSavedView(view),base:cloneSavedView(view)};source=view.parent;applySavedViewPresentation(view);savedViewRefresh();
}
function savedViewUsePage(page){
 const state=currentSavedViewState(page);if(state)applySavedViewPresentation(state.draft);
}
function savedViewCapturePresentation(page=source){
 const state=currentSavedViewState(page);if(!state)return;
 const query=document.querySelector('#search');if(query)state.draft.query=query.value;
 if(typeof groupingPrefs==='object'&&savedViewGroups.includes(groupingPrefs[page]))state.draft.groupBy=groupingPrefs[page];
 persistSavedViewState();
}
function sameSavedViewDefinition(left,right){
 const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
 const fields=view=>({parent:view.parent,lifecycle:view.lifecycle,filter:view.filter,query:view.query,groupBy:view.groupBy});
 return JSON.stringify(canonical(fields(left)))===JSON.stringify(canonical(fields(right)));
}
async function loadSavedViews({canApply=()=>true}={}){
 try{
  const previous=savedViewsDocument?.views||[],next=validateSavedViewsDocument(await api('/api/views'));if(!canApply())return null;savedViewsDocument=next;savedViewsError='';
  for(const page of savedViewPages){
   const state=currentSavedViewState(page);
   if(!savedViewMigrationComplete&&!state.id&&page==='github'&&typeof prefs==='object'&&['mine','review'].includes(prefs.githubView)){
    const view=next.views.find(view=>view.id==='github-'+prefs.githubView);if(view){state.id=view.id;state.draft=cloneSavedView(view);state.base=cloneSavedView(view);}
   }
   if(!state.id)continue;
   const latest=next.views.find(view=>view.id===state.id&&view.parent===page);
   if(!latest){state.id='';state.stale='removed';continue;}
   const knownBaseline=state.base||previous.find(view=>view.id===state.id&&view.parent===page),baseline=knownBaseline||latest;
   if(sameSavedViewDefinition(state.draft,baseline)){state.draft=cloneSavedView(latest);state.base=cloneSavedView(latest);delete state.stale;}
   else{state.base=cloneSavedView(baseline);if(!knownBaseline||!sameSavedViewDefinition(latest,baseline))state.stale='changed';}
  }
  savedViewMigrationComplete=true;try{localStorage.setItem('truffle-saved-view-migration','1');}catch(_){}
  if(savedViewPages.includes(source))applySavedViewPresentation(currentSavedViewState().draft);
  persistSavedViewState();
 }catch(error){savedViewsError=error.message||String(error);}
 if(typeof render==='function')render();return savedViewsDocument;
}
function savedViewButton(label,run,cls='text-button'){const button=el('button',cls,label);button.type='button';button.onclick=run;return button;}
function savedViewControl(label,run,variant='secondary'){const button=savedViewButton(label,run,'ui-control');button.dataset.variant=variant;return button;}
function savedViewSelect(values,selected,onchange,label){
 const select=el('select');select.setAttribute('aria-label',label);
 for(const [value,title]of values)select.add(new Option(title,value));select.value=selected;select.onchange=()=>onchange(select.value);return select;
}
function savedViewSummary(filter){
 if(['all','any'].includes(filter.op)){const text=filter.conditions.map(savedViewSummary).join(filter.op==='all'?' AND ':' OR ');return filter.conditions.length>1?'('+text+')':text||(filter.op==='all'?'All items':'No items');}
 const field={source:'Source',kind:'Kind',status:'Status',sourceStatus:'Source status',scope:'Project / repository',relations:'Relationship',pinned:'Pinned',updatedAt:'Updated',createdAt:'Created'}[filter.field];
 if(filter.op==='calendarPeriod')return field+' during '+savedViewCalendarPeriods[filter.period]+' ('+filter.timezone+'; weeks start Monday)';
 if(savedViewDateOps.includes(filter.op))return field+' '+({withinLast:'within the last',olderThan:'more than',beforeDate:'before',onOrAfterDate:'on or after'}[filter.op])+' '+(['withinLast','olderThan'].includes(filter.op)?filter.days+' days'+(filter.op==='olderThan'?' ago':'')+' (rolling 24 hours)':filter.date+' (UTC)');
 if(filter.op==='equals')return field+' is '+(filter.value?'yes':'no');
 return field+' '+({in:'is',notIn:'is not',containsAny:'includes any',containsAll:'includes all',containsNone:'includes none'}[filter.op])+' '+filter.values.join(', ');
}
function savedViewRuleCount(filter){
 if(!['all','any'].includes(filter.op))return 1;
 return filter.conditions.reduce((count,condition)=>count+savedViewRuleCount(condition),0)||Number(filter.op==='any');
}
function renderSavedViewControls(){
 const state=currentSavedViewState();let host=document.querySelector('#saved-view-controls');
 if(!state){if(host)host.hidden=true;return;}
 if(!host){const toolbar=document.querySelector('.toolbar');if(!toolbar)return;host=el('div','saved-view-controls');host.id='saved-view-controls';toolbar.append(host);}
 const focused=host.contains(document.activeElement)?document.activeElement.dataset.savedViewControl:null;
 const restoreFocus=()=>{if(!focused)return;const buttons=[...host.querySelectorAll('button')],target=buttons.find(button=>button.dataset.savedViewControl===focused&&!button.disabled)||buttons.find(button=>button.getAttribute('aria-pressed')==='true'&&!button.disabled)||buttons.find(button=>!button.disabled);target?.focus({preventScroll:true});};
 host.hidden=false;host.replaceChildren();
 if(savedViewsError){const error=el('span','saved-view-error',savedViewsError);error.setAttribute('role','alert');const reload=savedViewControl('Reload views',()=>void loadSavedViews());reload.dataset.savedViewControl='reload';host.append(error,reload);restoreFocus();return;}
 const lifecycle=el('div','saved-view-lifecycle ui-segmented');lifecycle.setAttribute('role','group');lifecycle.setAttribute('aria-label','Work lifecycle');
 for(const [value,label]of [['active','Active'],['completed','Completed'],['any','Any']]){const button=savedViewControl(label,()=>{savedViewCapturePresentation();state.draft.lifecycle=value;savedViewRefresh();});button.dataset.savedViewControl='lifecycle:'+value;button.setAttribute('aria-pressed',String(state.draft.lifecycle===value));lifecycle.append(button);}
 const view=savedViewsDocument?.views.find(view=>view.id===state.id),label=view?.name||'Current view',modified=Boolean(view&&state.base&&!sameSavedViewDefinition(state.draft,state.base));
 const rules=savedViewRuleCount(state.draft.filter),filter=savedViewControl('Filters'+(rules?' · '+rules:''),()=>openSavedViewEditor());filter.title=savedViewSummary(state.draft.filter);filter.dataset.savedViewControl='filters';
 const save=savedViewControl(view?'Edit view':'Save view',()=>openSavedViewEditor());save.dataset.savedViewControl='save';save.disabled=!savedViewsDocument;
 host.append(lifecycle,filter,save);if(state.stale){const notice=el('small','form-hint',state.stale==='removed'?'Saved view removed. Your filters are kept as an unsaved view.':'Saved view changed. Your filters are kept. Reset to use the saved version.');notice.setAttribute('role','status');host.append(notice);}else if(modified)host.append(el('small','form-hint','Modified · unsaved filters'));if(view)host.append(el('span','saved-view-name',label));if(view&&(modified||state.stale)){const reset=savedViewControl('Reset',()=>activateSavedView(view.id));reset.dataset.savedViewControl='reset';reset.title='Restore saved filters';host.append(reset);}
 restoreFocus();
}
function renderSavedViewSidebar(){
 if(!savedViewsDocument||savedViewsError)return;
 const items=typeof allItems==='function'?allItems():[],nowSeconds=Date.now()/1000;
 for(const page of savedViewPages){
  const parent=document.querySelector('[data-source="'+page+'"]');if(!parent)continue;
  let host=document.querySelector('[data-saved-view-parent="'+page+'"]')||(page==='github'?document.querySelector('#github-subnav'):null);
  if(!host){host=el('div','source-subnav saved-view-subnav');parent.insertAdjacentElement('afterend',host);}
  const active=document.activeElement,focused=source===page&&!host.hidden&&host.contains(active)?active.dataset.savedViewId:null;
  host.dataset.savedViewParent=page;host.setAttribute('role','group');host.setAttribute('aria-label',(typeof sourceNames==='object'?sourceNames[page]:page)+' saved views');host.hidden=source!==page;host.replaceChildren();
  for(const view of savedViewsDocument.views.filter(view=>view.parent===page)){
   const button=savedViewButton(view.name,()=>activateSavedView(view.id));button.dataset.savedViewId=view.id;button.setAttribute('aria-pressed',String(source===page&&currentSavedViewState(page).id===view.id));button.classList.toggle('active',source===page&&currentSavedViewState(page).id===view.id);button.append(el('b','',savedViewCount(view,items,nowSeconds)));host.append(button);
  }
  const next=document.activeElement;
  if(focused&&source===page&&host.isConnected&&!host.hidden&&host.getClientRects().length&&getComputedStyle(host).visibility!=='hidden'&&!document.querySelector('dialog[open]')&&(!next||next===document.body||next===active&&!active.isConnected)){const target=[...host.querySelectorAll('button')].find(button=>button.dataset.savedViewId===focused&&!button.disabled&&!button.hidden);target?.focus({preventScroll:true});}
 }
}
const savedViewFieldLabels={source:'Source',kind:'Kind',status:'Status',sourceStatus:'Source status',scope:'Project / repository',relations:'Relationship',pinned:'Pinned',updatedAt:'Updated',createdAt:'Created'};
function savedViewValueOptions(field){
 const defaults={source:['github','linear','sentry','notes'],kind:['Pull request','Issue','Error','Project'],status:['todo','in-progress','needs-review','ready-to-merge','draft','blocked','done'],relations:['authored','assigned','reviews']};
 const values=new Set(defaults[field]||[]);
 for(const item of typeof allItems==='function'?allItems():[]){const value=field==='relations'?item.relations:savedViewField(item,field);for(const entry of Array.isArray(value)?value:[value])if(typeof entry==='string'&&entry.trim())values.add(entry);}
 return [...values].sort((a,b)=>a.localeCompare(b)).slice(0,100);
}
function savedViewValueEditor(filter,onchange,canEdit){
 const node=el('div','saved-view-values'),chips=el('div','saved-view-value-chips'),entry=el('input','saved-view-value-entry');
 let committed=filter.values.filter(Boolean);entry.type='text';entry.placeholder='Add value…';entry.setAttribute('aria-label','Add '+savedViewFieldLabels[filter.field]+' value');
 const sync=()=>{if(!canEdit())return;filter.values=[...new Set([...committed,...entry.value.split('\n').map(value=>value.trim()).filter(Boolean)])];onchange();};
 const draw=()=>{chips.replaceChildren();for(const value of committed){const chip=el('span','saved-view-value-chip');const remove=savedViewButton('×',()=>{if(!canEdit())return;committed=committed.filter(item=>item!==value);draw();sync();entry.focus();},'saved-view-chip-remove');remove.setAttribute('aria-label','Remove '+value);chip.append(el('span','',value),remove);chips.append(chip);}};
 const commit=()=>{if(!canEdit())return;committed=[...new Set([...committed,...entry.value.split('\n').map(value=>value.trim()).filter(Boolean)])];entry.value='';draw();sync();};
 entry.oninput=sync;entry.onblur=()=>{if(entry.value.trim())commit();};entry.onkeydown=event=>{if(event.key==='Enter'&&!event.isComposing){event.preventDefault();commit();}};
 const options=savedViewValueOptions(filter.field),select=savedViewSelect([['','Choose value…'],...options.map(value=>[value,value])],'',value=>{if(!canEdit())return;if(value){committed=[...new Set([...committed,value])];draw();sync();select.value='';}},'Choose '+savedViewFieldLabels[filter.field]+' value');select.className='saved-view-value-select';
 draw();node.append(chips);if(options.length)node.append(select);node.append(entry);return node;
}
function savedViewFilterBuilder(filter,{depth=0,onchange=()=>{},canEdit}){
 const choose=(values,selected,change,label)=>savedViewSelect(values,selected,value=>{if(canEdit())change(value);},label);
 const grouped=['all','any'].includes(filter.op),node=el(grouped?'fieldset':'div',grouped?'saved-view-filter-node saved-view-filter-group':'saved-view-filter-row');
 if(grouped){
  const heading=el('legend','saved-view-group-heading'),children=el('div','saved-view-filter-children'),menu=el('details','saved-view-add-menu'),summary=depth<4?el('summary','','+ Add rule'):null;let rendered=[];
  const logic=choose([['all','All'],['any','Any']],filter.op,value=>{filter.op=value;draw();onchange();},'Group logic');heading.append(el('span','','Match'),logic,el('span','','of these rules'));node.append(heading,children);
  const add=(group)=>{if(!canEdit())return;if(filter.conditions.length>=32)return;const focusFromMenu=menu.contains(document.activeElement);filter.conditions.push(group?{op:'all',conditions:[]}:{op:'in',field:'status',values:['todo']});menu.open=false;draw();if(focusFromMenu)rendered.at(-1)?.row.querySelector('select')?.focus();onchange();};
  const condition=savedViewButton('Condition',()=>add(false)),group=savedViewButton('Condition group',()=>add(true));
  const draw=({removedIndex=0}={})=>{
   const focused=document.activeElement,previous=rendered.find(entry=>entry.row.contains(focused));
   const controlIndex=previous?[...previous.row.querySelectorAll('select,input,button,summary')].indexOf(focused):-1;
   children.replaceChildren();rendered=[];for(const [index,condition]of filter.conditions.entries()){ 
   const row=el('div','saved-view-condition'),join=el('span','saved-view-join',index?(filter.op==='all'?'AND':'OR'):'Where');join.setAttribute('aria-hidden','true');
   const remove=savedViewButton('×',()=>{if(!canEdit())return;const removedIndex=filter.conditions.indexOf(condition);filter.conditions.splice(removedIndex,1);draw({removedIndex});onchange();},'saved-view-remove-rule');const updateRemoveLabel=()=>remove.setAttribute('aria-label',['all','any'].includes(condition.op)?'Remove condition group':'Remove '+savedViewFieldLabels[condition.field]+' rule');updateRemoveLabel();
   row.append(join,savedViewFilterBuilder(condition,{depth:depth+1,canEdit,onchange:()=>{updateRemoveLabel();onchange();}}),remove);children.append(row);rendered.push({condition,row});
  }if(!filter.conditions.length)children.append(el('p','saved-view-empty',filter.op==='all'?'No rules · matches all items':'No rules · matches no items'));condition.disabled=group.disabled=filter.conditions.length>=32;
   if(previous){
    const retained=rendered.find(entry=>entry.condition===previous.condition);
    const target=retained?[...retained.row.querySelectorAll('select,input,button,summary')][controlIndex]:rendered[Math.min(removedIndex,rendered.length-1)]?.row.querySelector('select,input,button,summary')||summary;
    target?.focus();
   }
  };
  if(depth<4){const actions=el('div','saved-view-add-options');actions.append(condition);if(depth<3)actions.append(group);menu.append(summary,actions);node.append(menu);}
  draw();
 }else{
  const redraw=()=>{
   const focused=node.contains(document.activeElement)?document.activeElement.getAttribute('aria-label'):null;
   draw();if(focused)[...node.querySelectorAll('select')].find(control=>control.getAttribute('aria-label')===focused)?.focus();
  };
  const draw=()=>{
   node.replaceChildren(choose(Object.entries(savedViewFieldLabels),filter.field,value=>{for(const key of Object.keys(filter))delete filter[key];Object.assign(filter,savedViewDateFields.includes(value)?{op:'withinLast',field:value,days:7}:value==='pinned'?{op:'equals',field:value,value:true}:{op:value==='relations'?'containsAny':'in',field:value,values:[]});redraw();onchange();},'Filter field'));
   if(savedViewDateFields.includes(filter.field)){
    const relative=['withinLast','olderThan'].includes(filter.op);
    const operator=choose([['withinLast','in the last'],['olderThan','more than'],['beforeDate','before'],['onOrAfterDate','on or after'],['calendarPeriod','during']],filter.op,value=>{const {field,days,date,period,timezone}=filter;for(const key of Object.keys(filter))delete filter[key];Object.assign(filter,{field,op:value},value==='calendarPeriod'?{period:period||'yesterday',timezone:timezone||savedViewLocalTimeZone()}:['withinLast','olderThan'].includes(value)?{days:days||7}:{date:date||new Date().toISOString().slice(0,10)});redraw();onchange();},'Date operator');
    if(filter.op==='calendarPeriod'){
     const period=choose(Object.entries(savedViewCalendarPeriods),filter.period,value=>{filter.period=value;onchange();},'Calendar period'),zone=el('input','saved-view-time-zone'),choices=el('datalist');choices.id='saved-view-timezones-'+(++savedViewTimeZoneInputId);for(const name of new Set([savedViewLocalTimeZone(),'UTC',filter.timezone])){const option=el('option');option.value=name;choices.append(option);}zone.type='text';zone.value=filter.timezone;zone.maxLength=100;zone.setAttribute('list',choices.id);zone.setAttribute('aria-label','Calendar time zone (IANA)');zone.oninput=()=>{if(!canEdit())return;filter.timezone=zone.value.trim();zone.setCustomValidity?.(savedViewCalendarFormatter(filter.timezone)?'':'Choose a supported IANA time zone.');onchange();};const hint=el('span','saved-view-date-hint','Time zone · weeks start Monday');
     node.append(operator,period,zone,choices,hint);return;
    }
    const value=el('input','saved-view-date-value');value.type=relative?'number':'date';value.value=relative?filter.days:filter.date;if(relative){value.min=1;value.max=3650;value.step=1;}else{value.min='0000-01-01';value.max='9999-12-31';}value.setAttribute('aria-label',relative?'Number of rolling days':'Date in UTC');value.oninput=()=>{if(!canEdit())return;if(relative)filter.days=value.value===''?null:Number(value.value);else filter.date=value.value;onchange();};
    const hint=el('span','saved-view-date-hint',relative?'days'+(filter.op==='olderThan'?' ago':'')+' · rolling 24h':'UTC');hint.title='Items without a valid timestamp do not match. Created dates become available after the next source sync.';node.append(operator,value,hint);return;
   }
   if(filter.field==='pinned'){node.append(el('span','saved-view-operator-label','is'),choose([['true','Pinned'],['false','Not pinned']],String(filter.value),value=>{filter.value=value==='true';onchange();},'Pinned value'));return;}
   const ops=filter.field==='relations'?[['containsAny','includes any'],['containsAll','includes all'],['containsNone','includes none']]:[['in','is any of'],['notIn','is not']];
   node.append(choose(ops,filter.op,value=>{filter.op=value;onchange();},'Filter operator'),savedViewValueEditor(filter,onchange,canEdit));
  };draw();
 }
 return node;
}
async function saveSavedViewDefinition(definition,{replaceId='',baseline=savedViewsDocument}={}){
 if(!baseline)throw new Error('Load saved views before saving.');
 const views=cloneSavedView(baseline.views),index=views.findIndex(view=>view.id===replaceId);
 if(replaceId&&index<0)throw new Error('This view was removed. Save a new view.');
 if(index>=0)views[index]=definition;else views.push(definition);
 const next=validateSavedViewsDocument({...baseline,views});
 savedViewsDocument=validateSavedViewsDocument(await api('/api/views',{revision:next.revision,contentHash:next.contentHash,views:next.views}));savedViewsError='';return savedViewsDocument;
}
async function deleteSavedViewDefinition(id,baseline){
 if(!baseline||!id||!baseline.views.some(view=>view.id===id))throw new Error('This saved view is unavailable. Reload views before deleting.');
 const views=baseline.views.filter(view=>view.id!==id);
 savedViewsDocument=validateSavedViewsDocument(await api('/api/views',{revision:baseline.revision,contentHash:baseline.contentHash,views}));
 savedViewsError='';return savedViewsDocument;
}
function openSavedViewEditor(){
 const state=currentSavedViewState();if(!state)return;savedViewCapturePresentation();const page=source;
 let baseline=savedViewsDocument?cloneSavedView(savedViewsDocument):null,copyBaseline=null,conflictingView=false,deletionConfirmation=null,saving=false,accepted=null;
 const draft=cloneSavedView(state.draft),view=baseline?.views.find(view=>view.id===state.id),dialog=el('dialog','saved-view-editor');dialog.setAttribute('aria-label','Edit saved view');
 const header=el('div','saved-view-editor-heading'),close=savedViewButton('×',()=>requestDialogClose(dialog));close.setAttribute('aria-label','Close filter editor');close.className+=' close-dialog';header.append(el('h2','',view?'Edit '+view.name:'Filter work'),close);dialog.append(header);
 const name=el('input');name.value=view?.name||'';name.placeholder='View name';name.maxLength=120;name.setAttribute('aria-label','View name');const options=el('details','saved-view-options');options.open=Boolean(view);options.append(el('summary','','View options'),name);dialog.append(options);
 const error=el('p','saved-view-error');error.setAttribute('role','alert');
 const status=el('p','form-hint');status.tabIndex=-1;status.setAttribute('role','status');
 const current={host:dialog,dialog,controls:new Map(),pending:false,refreshing:false,ui:{message:status},state};savedViewEditor=current;current.isCurrent=()=>savedViewEditor===current&&dialog.open&&dialog.isConnected&&source===page&&currentSavedViewState(page)===current.state;
 const mutable=()=>current.isCurrent()&&!saving&&!accepted;
 const begin=message=>{current.controls.clear();saving=true;current.pending=true;error.textContent='';status.textContent=message;holdFormFocus(dialog,current);freezeFeatureForm(current,true);retry.disabled=true;};
 const finish=()=>{saving=false;current.pending=false;freezeFeatureForm(current,Boolean(accepted));retry.disabled=false;if(!current.isCurrent())return;if(!accepted)status.textContent=error.textContent;restoreFormFocus(current,accepted?retry:current.focusReturn);if(!accepted&&document.activeElement!==status)status.textContent='';};
 const summary=el('p','form-hint'),changed=()=>{summary.textContent=savedViewSummary(draft.filter);};
 if(!['all','any'].includes(draft.filter.op))draft.filter={op:'all',conditions:[draft.filter]};
 const filterEditor=savedViewFilterBuilder(draft.filter,{onchange:changed,canEdit:mutable});dialog.append(filterEditor);changed();dialog.append(summary);
 const group=savedViewSelect(savedViewGroups.filter(value=>value!=='stack'||draft.parent==='github').map(value=>[value,'Group: '+value]),draft.groupBy,value=>{if(mutable())draft.groupBy=value;},'Group work');options.append(group);
 const presentAccepted=()=>{
  if(!current.isCurrent())return;
  if([...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog&&!open.contains(dialog)))throw new Error('Close the open dialog before refreshing this view.');
  if(accepted.kind==='save'){
   if(!savedViewsDocument?.views.some(row=>row.id===accepted.id&&row.parent===page))throw new Error('The saved view is no longer available.');
   try{activateSavedView(accepted.id);}finally{const selected=currentSavedViewState(page);if(selected.id===accepted.id)current.state=selected;}
  }else{
   current.state.id='';current.state.draft=cloneSavedView(accepted.draft);delete current.state.base;delete current.state.stale;
   applySavedViewPresentation(current.state.draft);savedViewRefresh();
  }
  if(current.isCurrent())dialog.close();
 };
 const paintFailed=()=>{if(!current.isCurrent())return;error.textContent='';status.textContent=(accepted.kind==='save'?'View saved.':'View deleted.')+' The view could not refresh. Use Refresh view; do not repeat this action.';retry.hidden=false;};
 const retry=savedViewControl('Refresh view',async()=>{
  if(!current.isCurrent()||saving||!accepted)return;begin('Refreshing saved view…');retry.disabled=true;
  try{await loadSavedViews({canApply:current.isCurrent});if(!current.isCurrent())return;if(savedViewsError)throw new Error(savedViewsError);presentAccepted();}catch(_){paintFailed();}finally{finish();}
 });retry.hidden=true;retry.dataset.featureRecovery='true';
 const footer=el('div','saved-view-editor-actions ui-actiongroup');footer.setAttribute('role','group');footer.setAttribute('aria-label','Saved view actions');footer.dataset.align='end';footer.append(savedViewControl('Reload views',async()=>{
  if(!mutable())return;begin('Loading saved views…');
  try{await loadSavedViews({canApply:current.isCurrent});if(!current.isCurrent())return;if(savedViewsError){error.textContent=savedViewsError;return;}
   const fresh=savedViewsDocument?cloneSavedView(savedViewsDocument):null,latest=fresh?.views.find(candidate=>candidate.id===view?.id);
   conflictingView=Boolean(view&&(!latest||view.name!==latest.name||!sameSavedViewDefinition(view,latest)));
   if(conflictingView){copyBaseline=fresh;if(deletionConfirmation)deletionConfirmation.hidden=true;error.textContent='Saved view changed or was removed. Your draft is kept. Close and reopen to edit the latest view, or choose Save as new view to keep this draft separately.';}
   else{baseline=fresh;copyBaseline=null;error.textContent='Saved views reloaded. Your filter edits remain here.';}
  }catch(e){if(current.isCurrent())error.textContent=e.message;}finally{finish();}
 }));
 footer.append(savedViewControl('Apply filters',()=>{if(!mutable())return;try{validateSavedViewFilter(draft.filter);state.draft=cloneSavedView(draft);applySavedViewPresentation(state.draft);savedViewRefresh();dialog.close();}catch(e){error.textContent=e.message;}}));
 const save=async(asNew)=>{
  if(!mutable())return;
  try{if(!asNew&&conflictingView)throw new Error('Close and reopen to edit the latest saved view, or choose Save as new view to keep this draft separately.');validateSavedViewFilter(draft.filter);if(!savedViewText(name.value,120)){options.open=true;name.focus();throw new Error('Enter a view name.');}
   const id=asNew||!view?'view-'+crypto.randomUUID():view.id,definition=cloneSavedView({...draft,id,name:name.value.trim()}),capturedBaseline=cloneSavedView(asNew&&copyBaseline?copyBaseline:baseline);begin('Saving saved view…');
   await saveSavedViewDefinition(definition,{replaceId:asNew?'':view?.id||'',baseline:capturedBaseline});accepted={kind:'save',id};if(current.isCurrent()){try{presentAccepted();}catch(_){paintFailed();}}
  }catch(e){if(current.isCurrent())error.textContent=e.message+' Your draft is kept. Reload views to check for changes; if this view changed, close and reopen it or save a new view.';}finally{if(saving)finish();}
 };
 const saveButton=savedViewControl(view?'Save changes':'Save view',()=>save(false),'primary');saveButton.disabled=!savedViewsDocument;footer.append(saveButton);
 if(view){
  footer.append(savedViewControl('Save as new view',()=>save(true)),savedViewControl('Reset',()=>{if(!mutable())return;activateSavedView(view.id);dialog.close();}));
  const confirmation=el('div','saved-view-delete-confirmation');deletionConfirmation=confirmation;confirmation.hidden=true;
  confirmation.append(el('p','','Delete “'+view.name+'”? Your current filters will remain as an unsaved view.'));
  const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label','Delete view actions');
  actions.append(savedViewControl('Keep view',()=>{if(!mutable())return;confirmation.hidden=true;}),savedViewControl('Confirm delete',async()=>{
   if(!mutable())return;if(conflictingView){error.textContent='Close and reopen the latest saved view before deleting it.';return;}
   const capturedDraft=cloneSavedView(draft),capturedBaseline=cloneSavedView(baseline);error.textContent='';begin('Deleting saved view…');
   try{await deleteSavedViewDefinition(view.id,capturedBaseline);accepted={kind:'delete',draft:capturedDraft};if(current.isCurrent()){try{presentAccepted();}catch(_){paintFailed();}}}
   catch(e){if(current.isCurrent())error.textContent=e.message+' Your draft is kept. Reload views to check for changes, or close and reopen the latest view before deleting.';}
   finally{finish();}
  },'destructive'));
  confirmation.append(actions);dialog.append(confirmation);
  footer.append(savedViewControl('Delete view',()=>{if(!mutable())return;if(conflictingView){error.textContent='Close and reopen the latest saved view before deleting it.';return;}confirmation.hidden=false;},'destructive'));
 }
 footer.append(retry);
 dialog.addEventListener('cancel',event=>{if(saving)event.preventDefault();});
 dialog.append(error,status,footer);dialog.addEventListener('close',()=>{if(savedViewEditor===current)savedViewEditor=null;dialog.remove();},{once:true});document.body.append(dialog);dialog.showModal();filterEditor.querySelector('select')?.focus();
}

function applyAgentWorkView(command){
 if(!savedViewPages.includes(command.source))return;
 const previous=currentSavedViewState(command.source),conditions=[];
 const lifecycle=command.status==='active'?'active':'any';
 if(!['active','all'].includes(command.status))conditions.push({op:'in',field:'status',values:[command.status]});
 const draft={parent:command.source,lifecycle,filter:{op:'all',conditions},query:command.query,groupBy:previous.draft.groupBy};
 savedViewPageStates[command.source]={id:'',draft};applySavedViewPresentation(draft);persistSavedViewState();
}
if(typeof window!=='undefined'&&window.addEventListener)window.addEventListener('DOMContentLoaded',()=>void loadSavedViews(),{once:true});
