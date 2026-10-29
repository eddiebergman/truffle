'use strict';
let chatSearchPage=null;
let chatResultIntent=0;
let chatSearchSurface=null;
try{const saved=sessionStorage.getItem('truffle-chat-results');if(saved&&saved.length<65536)chatSearchPage={...JSON.parse(saved),command:appChatCommand(JSON.parse(saved).command)};}catch{}
function saveChatResults(){try{sessionStorage.setItem('truffle-chat-results',JSON.stringify(chatSearchPage));}catch{}}
function rememberChatResults(command){chatSearchPage={command:appChatCommand(command),scroll:0,filter:'',selectedId:'',returning:false};saveChatResults();}
function showChatResults(command){chatResultIntent++;if(command)rememberChatResults(command);source='chat-results';$('#search').value=chatSearchPage?.command.query||'';persist();render();}
async function openChatSearchTarget(id){
 const intent=++chatResultIntent,page=chatSearchPage,origin=source,selected=selectedAgentKey;
 await loadInventory();if(intent!==chatResultIntent||page!==chatSearchPage||origin!==source||selected!==selectedAgentKey)return;
 const record=(state?.sessions||[]).find(r=>r.id===id),candidate=record?candidateForRecord(record):liveCandidates().find(c=>candidateKey(c)===id);
 if(!record&&!candidate)throw new Error('That chat is no longer available. Search again.');
 if(chatSearchPage){chatSearchPage.scroll=$('#chat-search-matches')?.scrollTop??chatSearchPage.scroll;chatSearchPage.selectedId=id;chatSearchPage.returning=true;saveChatResults();}
 $('#search').value='';selectAgent(record||recordForCandidate(candidate),candidate);updateChatResultsBack();
}
function resultWorkItem(result){return allItems().find(row=>row.id===result.id)||result.item;}
function resultChatRecord(result){return (state?.sessions||[]).find(row=>row.id===result.id);}
function resultChatLinks(result){return resultChatRecord(result)?.items||result.item.items||[];}
function resultSearchMatches(result,query){
 if(result.type==='work')return Number.isFinite(workSearchScore(query,resultWorkItem(result)));
 const item=result.item,links=resultChatLinks(result).flatMap(link=>[link.title,link.reference,link.url,link.project]);
 return Number.isFinite(fuzzyScore(query,[item.title,item.cwd,item.reference,item.project,item.source,...links].filter(Boolean).join(' ')));
}
const resultItemRenderers={
 work(result){return renderRow(resultWorkItem(result));},
 chat(result){
  const item=result.item,record=resultChatRecord(result),candidate=record?candidateForRecord(record):liveCandidates().find(c=>candidateKey(c)===result.id);
  const row=el('button','chat-search-match compact-chat-match');row.type='button';row.dataset.chatResult=result.id;
  const title=el('strong','',item.title),badges=el('span','skill-picker-pills'),status=candidate?runtimeInfo(candidate,record).summary:item.archived?'Archived':'Saved';
  badges.append(el('span','skill-picker-pill','Chat'),el('span','skill-picker-pill',agentName(item.agent)),el('span','skill-picker-pill skill-scope-pill',status));
  const context=el('span','result-chat-context');context.append(el('code','',item.cwd||''));
  const refs=resultChatLinks(result).slice(0,3).map(workReference).filter(Boolean);if(refs.length)context.append(el('small','',refs.join(' · ')));
  row.append(title,badges,context);row.onclick=()=>openChatSearchTarget(result.id).catch(error=>toast(error.message));return row;
 }
};
function renderResultItem(result){const renderer=resultItemRenderers[result.type];if(!renderer)throw new Error('Unsupported result type.');return renderer(result);}
function ownsChatSearchSurface(surface){
 return chatSearchSurface===surface&&surface.container===$('#work-list')&&surface.page===chatSearchPage&&surface.form.isConnected&&surface.form.parentElement===surface.container&&surface.input.parentElement===surface.form&&surface.status.parentElement===surface.container&&(!surface.list||surface.list.parentElement===surface.container);
}
function drawChatResultRows(surface){
 const {page,input,list,status}=surface;if(!page){status.textContent=surface.pending?'Searching chats…':surface.error||'Search live, snoozed, saved and archived chats.';return;}
 const value=page.command,scroll=list.scrollTop;list.replaceChildren();
 if(value.type==='display-items'){
  for(const result of value.items){
   if(!resultSearchMatches(result,input.value))continue;
   const recommended=value.recommended?.type===result.type&&value.recommended?.id===result.id;
   const row=renderResultItem(result);row.classList.toggle('agent-result-recommended',recommended);row.classList.toggle('recommended',recommended);
   if(recommended)($('.row-main',row)||row).append(el('span','chat-search-recommended','Suggested'));
   if(recommended&&value.reason)row.title=value.reason;list.append(row);
  }
  status.textContent=list.children.length+' of '+value.items.length+' selected results'+(value.reason?' · '+value.reason:'');
 }else{
  status.textContent=surface.pending?'Searching chats…':surface.error||(value.total?'Showing '+value.chats.length+' of '+value.total+' matching chats. Titles, folders and linked work are searched.':'No chats match. Try a shorter keyword or part of the working folder.');
  for(const row of value.chats){
   const recommended=row.id===value.recommendedId,card=el('button','chat-search-match'+(recommended?' recommended':''));card.type='button';card.dataset.chatResult=row.id;
   const heading=el('span','chat-search-match-heading');heading.append(el('strong','',row.title||'Untitled chat'));if(recommended)heading.append(el('span','chat-search-recommended','Suggested'));if(page.selectedId===row.id)heading.append(el('small','','Last viewed'));
   card.append(heading,el('code','',row.cwd),el('small','',[(row.agent||'Agent'),row.live?'Live · restores snoozed tabs':row.archived?'Archived':'Saved'].join(' · ')));
   if(recommended&&value.reason)card.append(el('span','chat-search-reason',value.reason));
   card.onclick=async()=>{card.disabled=true;try{await openChatSearchTarget(row.id);}catch(error){if(ownsChatSearchSurface(surface)){surface.error=error.message;status.textContent=error.message;card.disabled=false;}}};list.append(card);
  }
 }
 list.scrollTop=scroll;
}
function renderChatSearchResults(){
 const page=chatSearchPage,container=$('#work-list');$('#page-title').textContent=page?.command.type==='display-items'?(page.command.title||'Suggested results'):'Chat search';
 if(chatSearchSurface&&ownsChatSearchSurface(chatSearchSurface)){drawChatResultRows(chatSearchSurface);return;}
 const form=el('form','chat-search-form'),input=el('input'),submit=el('button','button','Search'),status=el('p','form-hint'),list=page?el('div','chat-search-matches'):null;
 const surface={page,container,form,input,submit,status,list,pending:false,error:''};chatSearchSurface=surface;
 input.type='search';input.value=page?.command.type==='display-items'?page.filter||'':page?.command.query||'';input.placeholder='Find a chat by title, folder, or linked work…';input.setAttribute('aria-label','Search all chats');submit.type='submit';form.append(input,submit);status.setAttribute('role','status');if(list)list.id='chat-search-matches';container.replaceChildren(form,status,...(list?[list]:[]));
 form.onsubmit=async event=>{
  event.preventDefault();if(surface.pending||!ownsChatSearchSurface(surface))return;const intent=++chatResultIntent,query=input.value;
  const current=()=>intent===chatResultIntent&&source==='chat-results'&&ownsChatSearchSurface(surface)&&input.value===query;
  surface.pending=true;surface.error='';submit.disabled=true;form.setAttribute('aria-busy','true');drawChatResultRows(surface);
  try{const result=await api('/api/commands/invoke',{name:'chats.search',args:{query}});if(current())showChatResults({type:'chat-results',query,chats:result.chats,total:result.total,recommendedId:'',reason:''});}
  catch(error){if(current()){surface.error=error.message;status.textContent=error.message;}}
  finally{surface.pending=false;submit.disabled=false;form.setAttribute('aria-busy','false');if(source==='chat-results'&&ownsChatSearchSurface(surface))drawChatResultRows(surface);}
 };
 if(page?.command.type==='display-items'){
  input.placeholder='Filter these results…';input.setAttribute('aria-label','Filter displayed results');submit.hidden=true;
  input.oninput=()=>{if(!ownsChatSearchSurface(surface))return;page.filter=input.value;page.scroll=0;drawChatResultRows(surface);list.scrollTop=0;saveChatResults();};form.onsubmit=event=>event.preventDefault();
 }
 drawChatResultRows(surface);if(list)requestAnimationFrame(()=>{if(source==='chat-results'&&ownsChatSearchSurface(surface))list.scrollTop=page.scroll||0;});
}
function invalidateChatSearchSurface(){if(source!=='chat-results')chatSearchSurface=null;}
function updateChatResultsBack(){
 let back=$('#chat-results-back');if(!back){back=featureButton('← Results',()=>showChatResults(),'text-button chat-results-back');back.id='chat-results-back';back.title='Back to chat search results';back.setAttribute('aria-label','Back to chat search results');$('#live-agent-bar').before(back);}
 back.hidden=source!=='agents'||!chatSearchPage?.returning;
}
registerRenderHook('chat-results:surface',invalidateChatSearchSurface,{phase:'before',order:-10});
new MutationObserver(updateChatResultsBack).observe(document.body,{attributes:true,attributeFilter:['class']});updateChatResultsBack();
