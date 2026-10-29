'use strict';
// Durable delivery receipts belong to the service. This bounded cache only presents them.
const chatDispatchCache=new Map(),chatDispatchStorageKey='truffle-dispatch-intents-v1';
let chatDispatchStorageError=false;
function storedChatDispatchIntents(){
 try{const raw=localStorage.getItem(chatDispatchStorageKey)||'[]';if(raw.length>32768)throw new Error('Recovery metadata is too large');const rows=JSON.parse(raw);if(!Array.isArray(rows)||rows.length>24||rows.some(row=>!row||Object.keys(row).some(key=>!['recordId','operationId','kind','itemId','revision','ticket','unavailable'].includes(key))||!['review-request','review-feedback','ci-investigation'].includes(row.kind)||typeof row.recordId!=='string'||row.recordId.length>80||typeof row.operationId!=='string'||row.operationId.length>80||row.unavailable!==undefined&&row.unavailable!==true||row.ticket!==undefined&&(typeof row.ticket!=='string'||!/^(0|[1-9][0-9]{0,19})$/.test(row.ticket))||row.revision!==undefined&&(!Number.isSafeInteger(row.revision)||row.revision<0)||row.itemId!==undefined&&(typeof row.itemId!=='string'||row.itemId.length>300)))throw new Error('Invalid recovery metadata');chatDispatchStorageError=false;return rows;}
 catch(_){chatDispatchStorageError=true;return [];}
}
function storeChatDispatchIntent(intent){
 const rows=storedChatDispatchIntents();if(chatDispatchStorageError)throw new Error('Delivery recovery storage is unavailable. Do not submit again.');const others=rows.filter(row=>row.recordId!==intent.recordId);if(others.length>=24)throw new Error('Resolve an earlier submission before starting another.');
 try{localStorage.setItem(chatDispatchStorageKey,JSON.stringify([...others,intent]));}catch(_){chatDispatchStorageError=true;throw new Error('Delivery recovery storage could not be saved. Nothing was submitted.');}
}
function clearChatDispatchIntent(recordId,operationId){const rows=storedChatDispatchIntents();if(!chatDispatchStorageError)try{localStorage.setItem(chatDispatchStorageKey,JSON.stringify(rows.filter(row=>row.recordId!==recordId||row.operationId!==operationId)));}catch(_){chatDispatchStorageError=true;}}
function unavailableChatDispatch(intent){return {type:'unavailable',recordId:intent.recordId,operationId:intent.operationId,reason:'possibly-expired',transport:'legacy-unfenced',message:'This operation is unavailable or its receipt expired. Prior delivery may have occurred. No input was sent by this submission. Inspect the chat before preparing any separate new request.'};}
function unknownChatDispatch(intent){return {...intent,status:'pending-receipt',finalized:false,message:'No delivery receipt is available yet. Check delivery or retry this same operation; do not start a new submission.'};}
function chatDispatchEntry(recordId){
 if(!chatDispatchCache.has(recordId)){const intent=storedChatDispatchIntents().find(row=>row.recordId===recordId);chatDispatchCache.set(recordId,{operations:[],loaded:false,readAt:0,pending:null,busy:false,unavailable:intent?.unavailable?unavailableChatDispatch(intent):null,intent:intent?{...intent,unknown:unknownChatDispatch(intent)}:null,presentation:null});while(chatDispatchCache.size>24)chatDispatchCache.delete(chatDispatchCache.keys().next().value);}
 return chatDispatchCache.get(recordId);
}
function unresolvedChatDispatch(recordId){const entry=chatDispatchEntry(recordId);return entry.unavailable||entry.operations.find(op=>!op.finalized&&(!op.resolved||op.resolution==='user-confirmed-submitted'))||entry.intent?.unknown||null;}
function chatDispatchBlocked(recordId){const entry=chatDispatchEntry(recordId);return chatDispatchStorageError||!entry.loaded||entry.busy||!!entry.presentation||!!unresolvedChatDispatch(recordId);}
function notifyChatDispatch(recordId){for(const node of document.querySelectorAll('[data-dispatch-record]'))if(node.dataset.dispatchRecord===recordId){node.refreshDispatch();node.dispatchChanged?.();}}
function rememberChatDispatch(receipt){
 const entry=chatDispatchEntry(receipt.recordId);
 if(receipt.type==='unavailable'){entry.unavailable=receipt;entry.operations=entry.operations.filter(op=>op.operationId!==receipt.operationId);if(entry.intent){const {unknown,...intent}=entry.intent;intent.unavailable=true;storeChatDispatchIntent(intent);entry.intent={...intent,unknown};}return receipt;}
 entry.unavailable=null;entry.operations=[receipt,...entry.operations.filter(op=>op.operationId!==receipt.operationId)].slice(0,32);
 if(receipt.finalized||receipt.resolution==='user-abandoned'){clearChatDispatchIntent(receipt.recordId,receipt.operationId);if(entry.intent?.operationId===receipt.operationId)entry.intent=null;}else if(entry.intent?.operationId===receipt.operationId)entry.intent.unknown=null;return receipt;
}
function persistChatDispatchIdentity(receipt){
 const entry=chatDispatchEntry(receipt.recordId),intent={recordId:receipt.recordId,operationId:receipt.operationId,kind:receipt.kind,...(receipt.itemId?{itemId:receipt.itemId}:{}),...(receipt.ticket!==undefined?{ticket:receipt.ticket}:{})};
 storeChatDispatchIntent(intent);entry.intent=intent;
}
async function lookupChatDispatch(receipt){
 const {recordId,operationId}=receipt,entry=chatDispatchEntry(recordId);
 if(entry.intent?.operationId!==operationId)persistChatDispatchIdentity(receipt);
 const capturedIntent=entry.intent;
 try{return rememberChatDispatch(await api('/api/commands/invoke',{name:'chats.dispatch.submit',args:{recordId,operationId}}));}
 catch(_){
  const known=entry.operations.find(op=>op.operationId===operationId);
  if(known?.finalized||known?.resolved||['uncertain','written'].includes(known?.status))return known;
  if(entry.unavailable?.operationId===operationId)return entry.unavailable;
  if(entry.intent?.operationId!==operationId)return known||unknownChatDispatch(capturedIntent);
  entry.intent.unknown=unknownChatDispatch(entry.intent);entry.operations=entry.operations.filter(op=>op.operationId!==operationId);try{await readChatDispatch(recordId,true);}catch(_){}return entry.operations.find(op=>op.operationId===operationId)||unresolvedChatDispatch(recordId)||entry.intent?.unknown;
 }
}
async function readChatDispatch(recordId,force=false){
 const entry=chatDispatchEntry(recordId);if(entry.pending)return entry.pending;if(!force&&Date.now()-entry.readAt<5000)return entry;
 entry.pending=(async()=>{try{const result=await api('/api/commands/invoke',{name:'chats.dispatch.read',args:{recordId}});entry.operations=result.operations.slice(0,32);entry.loaded=true;const found=entry.operations.find(op=>op.operationId===entry.intent?.operationId);if(found){entry.unavailable=null;if(found.finalized||found.resolution==='user-abandoned'){clearChatDispatchIntent(recordId,found.operationId);entry.intent=null;}else entry.intent.unknown=null;}return entry;}catch(error){entry.loaded=false;throw error;}finally{entry.readAt=Date.now();entry.pending=null;notifyChatDispatch(recordId);}})();return entry.pending;
}
async function submitChatDispatch(name,kind,args){
 const {recordId}=args,entry=chatDispatchEntry(recordId);if(chatDispatchStorageError)throw new Error('Delivery recovery storage is unavailable. Do not submit again.');if(entry.busy)throw new Error('A submission is already being checked.');entry.busy=true;
 try{
  await readChatDispatch(recordId,true);const blocked=unresolvedChatDispatch(recordId);if(blocked)return blocked;if(entry.presentation)return entry.presentation;
  const stored=storedChatDispatchIntents();if(chatDispatchStorageError)throw new Error('Delivery recovery storage is unavailable.');if(stored.filter(row=>row.recordId!==recordId).length>=24)throw new Error('Resolve an earlier submission before starting another.');
  let prepared;
  try{prepared=await api('/api/commands/invoke',{name:name+'.prepare',args});}
  catch(error){try{await readChatDispatch(recordId,true);}catch(_){}const recovered=unresolvedChatDispatch(recordId);if(recovered)return recovered;throw error;}
  if(prepared.type!=='operation'||prepared.recordId!==recordId||prepared.kind!==kind||(kind==='ci-investigation'&&prepared.itemId!==args.itemId)||typeof prepared.operationId!=='string'||!prepared.operationId||prepared.operationId.length>80||typeof prepared.ticket!=='string'||!/^(0|[1-9][0-9]{0,19})$/.test(prepared.ticket))throw new Error('Preparation returned an invalid delivery identity. Nothing was submitted. Check delivery before trying again.');
  rememberChatDispatch(prepared);if(prepared.finalized||prepared.resolved||prepared.status!=='prepared')return prepared;
  persistChatDispatchIdentity(prepared);return await lookupChatDispatch(prepared);
 }finally{entry.busy=false;notifyChatDispatch(recordId);}
}
async function refreshChatDispatch(receipt,{canPresent=()=>true,refresh=null}={}){
 const entry=chatDispatchEntry(receipt.recordId);
 try{const result=await (refresh?refresh():loadStateSnapshot({attempts:2}));if(!result.accepted)throw new Error('Dashboard changed.');entry.presentation=null;if(canPresent())render();notifyChatDispatch(receipt.recordId);return true;}
 catch(_){entry.presentation=receipt;notifyChatDispatch(receipt.recordId);return false;}
}
function chatDispatchLabel(receipt){
 const label={'review-request':'Review request','review-feedback':'Review feedback','ci-investigation':'CI investigation'}[receipt?.kind]||'Submission';
 const item=receipt?.kind==='ci-investigation'?state.items?.find(row=>row.id===receipt.itemId):null,reference=item?.reference?workReference(item):'';
 return reference?`${label} · ${reference.slice(0,100)}`:label;
}
function chatDispatchRecovery(recordId,{canPresent=()=>true,refresh=null,onInspect=null,onChange=null}={}){
 const host=el('div','chat-dispatch-recovery');host.dataset.dispatchRecord=recordId;host.dispatchChanged=onChange;
 let confirming=null,working=false,failure='';
 const current=()=>host.isConnected&&canPresent();
 const inspect=()=>{if(onInspect){onInspect();return;}const record=state.sessions.find(row=>row.id===recordId);if(record)selectAgent(record,candidateForRecord(record));else toast('This chat is unavailable.');};
 const act=async action=>{if(working||!current())return;working=true;failure='';const focused=host.contains(document.activeElement);host.tabIndex=-1;if(focused)host.focus({preventScroll:true});host.refreshDispatch();try{await action();}catch(_){failure='Delivery status could not be updated. Check delivery before trying again.';}finally{working=false;notifyChatDispatch(recordId);if(current()){host.refreshDispatch();if(focused&&host.contains(document.activeElement))host.querySelector('button:not(:disabled)')?.focus({preventScroll:true});}}};
 host.refreshDispatch=()=>{
  const entry=chatDispatchEntry(recordId),receipt=unresolvedChatDispatch(recordId),presentation=entry.presentation,focused=host.contains(document.activeElement)?document.activeElement.dataset.dispatchAction:null;host.replaceChildren();host.hidden=entry.loaded&&!receipt&&!presentation&&!working&&!failure&&!chatDispatchStorageError;if(host.hidden)return;
  const context=presentation||receipt,label=context?chatDispatchLabel(context.type==='unavailable'?entry.intent:context):'',text=failure||(chatDispatchStorageError?'Delivery recovery storage is unavailable. Do not submit again.':!entry.loaded?'Check delivery status before submitting.':presentation?(presentation.message+' The dashboard could not refresh. Refresh view; do not submit again.'):receipt?.message||'Checking delivery…');
  const message=el('p','form-hint',(label?label+': ':'')+text);message.setAttribute('role','status');host.append(message);
  const actions=el('div','ui-actiongroup');actions.setAttribute('role','group');actions.setAttribute('aria-label','Delivery recovery actions');host.append(actions);
  const button=(label,run,variant='secondary')=>{const control=featureButton(label,()=>void act(run),'ui-control');control.dataset.variant=variant;control.dataset.dispatchAction=label;control.disabled=working||entry.busy;actions.append(control);return control;};
  const restore=()=>{if(focused&&current()){const controls=[...host.querySelectorAll('button')];(controls.find(control=>control.dataset.dispatchAction===focused&&!control.disabled)||controls.find(control=>!control.disabled))?.focus({preventScroll:true});}};
  if(presentation){button('Refresh view',()=>refreshChatDispatch(presentation,{canPresent,refresh}));restore();return;}
  button('Check delivery',async()=>{const operationId=receipt?.operationId||entry.intent?.operationId;const read=await readChatDispatch(recordId,true),completed=read.operations.find(op=>op.operationId===operationId&&op.finalized);if(completed)await refreshChatDispatch(completed,{canPresent,refresh});});
  if(!receipt){restore();return;}
  if(!entry.loaded){button('Inspect chat',inspect);restore();return;}
  if(receipt.type==='unavailable'){
   button('Inspect chat',inspect);
   if(confirming==='unavailable'){const warning=el('p','form-hint',chatDispatchLabel(entry.intent)+': Prior delivery may have occurred. Acknowledging only clears this local recovery reminder. It does not undo delivery or send a new request.');warning.setAttribute('role','status');host.append(warning);button('Confirm acknowledgement',async()=>{clearChatDispatchIntent(recordId,receipt.operationId);entry.intent=null;entry.unavailable=null;confirming=null;});button('Cancel',async()=>{confirming=null;});}
   else button('Acknowledge unavailable receipt',async()=>{confirming='unavailable';});
   restore();return;
  }
  if((receipt.status==='written'||receipt.resolution==='user-confirmed-submitted')&&!receipt.finalized){button('Finish submission',async()=>{const result=rememberChatDispatch(await api('/api/commands/invoke',{name:'chats.dispatch.finalize',args:{recordId,operationId:receipt.operationId}}));if(result.finalized)await refreshChatDispatch(result,{canPresent,refresh});});}
  if(['prepared','pending-receipt'].includes(receipt.status)&&!receipt.resolved){button(receipt.status==='prepared'?'Submit prepared operation':'Check and submit same operation',async()=>{const result=await lookupChatDispatch(receipt);if(result.finalized)await refreshChatDispatch(result,{canPresent,refresh});});}
  button('Inspect chat',inspect);
  if(['uncertain','prepared'].includes(receipt.status)&&!receipt.resolved&&receipt!==entry.intent?.unknown){
   const resolve=async decision=>{const result=rememberChatDispatch(await api('/api/commands/invoke',{name:'chats.dispatch.resolve',args:{recordId,operationId:receipt.operationId,decision}}));confirming=null;if(result.finalized||result.resolved)await refreshChatDispatch(result,{canPresent,refresh});};
   if(confirming){const warning=el('p','form-hint',chatDispatchLabel(receipt)+': '+(confirming==='submitted'?'Only confirm after inspecting the chat. This records your judgment; the transport cannot verify delivery.':'This keeps the old operation as an audit record. It does not undo delivery or send again.'));warning.setAttribute('role','status');host.append(warning);button(confirming==='submitted'?'Confirm submitted':'Confirm abandon',()=>resolve(confirming),confirming==='abandon'?'destructive':'primary');button('Cancel',async()=>{confirming=null;});}
   else{if(receipt.status==='uncertain')button('I confirmed submission',async()=>{confirming='submitted';});button('Abandon submission',async()=>{confirming='abandon';});}
  }
  restore();
 };
 host.refreshDispatch();void readChatDispatch(recordId).catch(()=>{});return host;
}
