'use strict';
// Result navigation only. Terminal opens and mutations remain with their owners.
let presentationEpoch=0,presentationLastSignature='';
function resultPresentationSignature(){
 const view=typeof currentSavedViewState==='function'?currentSavedViewState(source):null;
 return JSON.stringify([source,typeof selectedAgentKey==='undefined'?'':selectedAgentKey,$('#search')?.value||'',typeof statusFilter==='undefined'?'':statusFilter.value,$('#group-by')?.value||'',view?{id:view.id,draft:view.draft}:null,typeof scopePrefs==='undefined'?null:scopePrefs[source],typeof githubView==='undefined'?null:githubView,typeof sentryView==='undefined'?null:sentryView,typeof sentryFilters==='undefined'?null:sentryFilters,source==='chat-results'&&typeof chatSearchPage!=='undefined'?{command:chatSearchPage?.command,filter:document.querySelector('.chat-search-form input')?.value}:null]);
}
function updateResultPresentationContext(){const next=resultPresentationSignature();if(next!==presentationLastSignature){presentationLastSignature=next;presentationEpoch++;}}
function resultPresentationGuard(){
 if(typeof settingsDirty!=='undefined'&&settingsDirty)throw new Error('Save or discard your Settings changes, then show these results.');
 if(document.querySelector('dialog[open]'))throw new Error('Close the open dialog, then show these results.');
 if($('#app-chat-setup')&&!$('#app-chat-setup').hidden)throw new Error('Close setup, then show these results.');
}
async function applyResultPresentation(value,{isCurrent=()=>true,focusResults=false}={}){
 const command=appChatCommand(value);if(!['saved-view','display-items'].includes(command.type))throw new Error('Unsupported result presentation.');
 resultPresentationGuard();updateResultPresentationContext();const signature=presentationLastSignature,epoch=presentationEpoch;
 const focus=document.activeElement;
 const current=()=>{updateResultPresentationContext();const active=document.activeElement;return isCurrent()&&epoch===presentationEpoch&&signature===presentationLastSignature&&(!active||active===focus||active===document.body||!active.isConnected);};
 if(command.type==='saved-view'){
  await loadSavedViews();
  if(!current())throw new Error('The page changed. Choose Show results again.');
  resultPresentationGuard();
  if(savedViewsError)throw new Error('Saved views could not refresh. '+savedViewsError);
  if(!savedViewsDocument?.views.some(view=>view.id===command.id))throw new Error('This saved view is no longer available.');
  activateSavedView(command.id);
 }else{
  if(!current())throw new Error('The page changed. Choose Show results again.');
  resultPresentationGuard();showChatResults(command);
 }
 if(focusResults&&isCurrent()&&!document.querySelector('dialog[open]')){
  const active=document.activeElement;if(!active||active===focus||active===document.body||!active.isConnected){const target=source==='chat-results'?document.querySelector('.chat-search-form input'):$('#search');if(target?.isConnected&&!target.disabled&&target.getClientRects().length)target.focus({preventScroll:true});}
 }
}
const pendingPresentations=new Map();
let presentationReadEpoch=0;
function completePresentation(requestId){pendingPresentations.delete(requestId);presentationReadEpoch++;}
let presentationPoll=null,presentationLastPoll=-Infinity,presentationAction=false,presentationNotice=null;
function presentationRequestId(value){return typeof value==='string'&&/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);}
function presentationRead(value){
 if(!value||!Array.isArray(value.pending)||value.pending.length>8||!Array.isArray(value.receipts)||value.receipts.length>40)throw new Error('Invalid pending results.');
 const ids=new Set(),pending=value.pending.map(row=>{
  if(!presentationRequestId(row?.requestId)||ids.has(row.requestId.toLowerCase()))throw new Error('Invalid pending result identity.');ids.add(row.requestId.toLowerCase());
  const command=appChatCommand(row.presentation);if(!['saved-view','display-items'].includes(command.type))throw new Error('Unsupported pending results.');return {requestId:row.requestId,presentation:command};
 });
 const receipts=value.receipts.map(row=>{if(!presentationRequestId(row?.requestId)||!['queued','shown','dismissed'].includes(row.status))throw new Error('Invalid result acknowledgement.');return {requestId:row.requestId,status:row.status};});
 return {pending,receipts};
}
function ensurePresentationNotice(){
 if(presentationNotice?.host.isConnected)return presentationNotice;
 const parent=$('#sidebar-notices');if(!parent)return null;
 const host=el('section','ui-notice agent-results-notice'),header=el('div','ui-notice-header'),title=el('strong','ui-notice-title','Agent results ready'),message=el('div','ui-notice-body'),actions=el('div','ui-actiongroup ui-notice-actions');
 host.setAttribute('aria-label','Agent results');message.setAttribute('role','status');actions.setAttribute('role','group');actions.setAttribute('aria-label','Agent result actions');
 const show=el('button','ui-control','Show'),dismiss=el('button','ui-control','Dismiss');for(const button of [show,dismiss]){button.type='button';button.dataset.variant='secondary';}show.dataset.variant='primary';
 header.append(title);actions.append(show,dismiss);host.append(header,message,actions);parent.append(host);
 presentationNotice={host,title,message,show,dismiss};return presentationNotice;
}
function presentationLabel(entry){return (entry.presentation.type==='saved-view'?entry.presentation.name:entry.presentation.title)||(entry.presentation.type==='saved-view'?entry.presentation.id:'Selected results');}
function presentationCommands(){
 return [...pendingPresentations.values()].slice(0,8).map(entry=>{
  const status=entry.outcome==='shown'?'Results were shown.':'Results were dismissed.';
  return {verb:entry.expired?'dismiss':entry.outcome?'retry':'show',noun:'agent results · '+presentationLabel(entry),detail:entry.expired?status+' Dismiss this local notice; the acknowledgement receipt has expired.':entry.outcome?status+' Retry acknowledgement only; results will not be displayed again.':'Show these selected results, then acknowledge that they were shown.',run:async()=>{if(presentationAction)throw new Error('Wait for the current result action to finish.');await showPendingPresentation(entry,{focusResults:true});if(pendingPresentations.get(entry.requestId)===entry&&entry.error)throw new Error(String(entry.error).slice(0,500));}};
 });
}
function paintPendingPresentations(){
 const ui=ensurePresentationNotice();if(!ui)return;
 const entry=[...pendingPresentations.values()].find(row=>row.outcome)||pendingPresentations.values().next().value;
 ui.host.hidden=!entry;if(!entry)return;
 ui.title.textContent=entry.outcome==='shown'?'Agent results shown':entry.outcome==='dismissed'?'Agent results dismissed':'Agent results ready';
 const label=presentationLabel(entry);
 ui.message.textContent=entry.expired?(entry.outcome==='shown'?'Results were shown. ':'Results were dismissed. ')+'The acknowledgement receipt is no longer available. You can dismiss this local notice.':entry.error||(entry.outcome?'Acknowledgement pending. Retry only acknowledges this result.':label+(pendingPresentations.size>1?' · '+pendingPresentations.size+' pending':''));
 ui.show.textContent=entry.expired?'Dismiss notice':entry.outcome?'Retry acknowledgement':'Show';ui.dismiss.hidden=!!entry.outcome;
 ui.show.disabled=ui.dismiss.disabled=presentationAction;
 ui.show.onclick=()=>showPendingPresentation(entry);ui.dismiss.onclick=()=>dismissPendingPresentation(entry);
}
async function acknowledgePresentation(entry){
 const receipt=await api('/api/commands/invoke',{name:'ui.presentations.ack',args:{requestId:entry.requestId,outcome:entry.outcome}});
 if(receipt?.requestId!==entry.requestId||receipt.status!==entry.outcome)throw new Error('The result acknowledgement could not be confirmed. Retry acknowledgement.');
 if(pendingPresentations.get(entry.requestId)===entry)completePresentation(entry.requestId);
}
async function actOnPendingPresentation(entry,outcome,{focusResults=false}={}){
 if(presentationAction||pendingPresentations.get(entry.requestId)!==entry)return;
 presentationAction=true;entry.error='';const ui=ensurePresentationNotice(),ownedFocus=!!ui?.host.contains(document.activeElement);if(ownedFocus){ui.message.tabIndex=-1;ui.message.focus({preventScroll:true});}paintPendingPresentations();
 try{
  if(entry.expired){completePresentation(entry.requestId);return;}
  if(!entry.outcome){
   if(outcome==='shown')await applyResultPresentation(entry.presentation,{isCurrent:()=>pendingPresentations.get(entry.requestId)===entry,focusResults});
   entry.outcome=outcome;
  }
  await acknowledgePresentation(entry);
 }catch(error){entry.error=entry.outcome?(entry.outcome==='shown'?'Results were shown. ':'Results were dismissed. ')+'Acknowledgement failed. Retry acknowledgement; the results will not be shown again.':error.message;}
 finally{presentationAction=false;paintPendingPresentations();if(ownedFocus&&document.activeElement===ui.message){const target=ui.host.hidden?(source==='chat-results'?document.querySelector('.chat-search-form input'):$('#search')):ui.show;if(target&&!target.disabled&&!target.hidden)target.focus({preventScroll:true});}}
}
function showPendingPresentation(entry,options={}){return actOnPendingPresentation(entry,'shown',options);}
function dismissPendingPresentation(entry){return actOnPendingPresentation(entry,'dismissed');}
function refreshPresentations({force=false}={}){
 if(!state)return Promise.resolve();
 if(presentationPoll)return presentationPoll;
 if(!force&&Date.now()-presentationLastPoll<5000)return Promise.resolve();
 presentationLastPoll=Date.now();
 const epoch=presentationReadEpoch;
 presentationPoll=(async()=>{
  try{
   const result=presentationRead(await api('/api/commands/invoke',{name:'ui.presentations.read',args:{}}));
   if(epoch!==presentationReadEpoch)return;
   const pendingIds=new Set(result.pending.map(row=>row.requestId)),receiptIds=new Set(result.receipts.map(row=>row.requestId));
   for(const [id,entry]of pendingPresentations)if(!pendingIds.has(id)){if(!entry.outcome)pendingPresentations.delete(id);else if(!receiptIds.has(id))entry.expired=true;}
   const completed=new Set(result.receipts.filter(row=>row.status!=='queued').map(row=>row.requestId));
   for(const receipt of result.receipts)if(receipt.status!=='queued')completePresentation(receipt.requestId);
   // A local shown/dismissed entry stays ACK-only until its terminal receipt is read.
   for(const row of result.pending)if(!completed.has(row.requestId)&&!pendingPresentations.has(row.requestId)&&pendingPresentations.size<8)pendingPresentations.set(row.requestId,{...row,outcome:null,error:''});
   paintPendingPresentations();
  }catch(_){/* Preserve the last safe notice; ordinary polling does not interrupt work. */}
  finally{presentationPoll=null;}
 })();return presentationPoll;
}
registerRenderHook('presentation:context',updateResultPresentationContext,{phase:'before',order:-15});
registerRenderHook('presentation:notice',paintPendingPresentations,{order:60});
registerRenderHook('presentation:context-painted',updateResultPresentationContext,{order:100});
window.refreshPresentations=refreshPresentations;
window.addEventListener('focus',()=>{void refreshPresentations();});
document.addEventListener('DOMContentLoaded',()=>{void refreshPresentations();},{once:true});
