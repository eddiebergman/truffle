/* Explicit GitHub PR controls and durable review-request attention. */
'use strict';
function githubUnreadReviews(data,items){
  const ready=new Set(items.filter(i=>i.source==='github'&&i.kind==='Pull request'&&!i.hidden&&!i.snoozed&&!i.archivedAt&&i.isDraft!==true&&i.status!=='draft'&&i.status!=='done'&&i.relations?.includes('reviews')).map(i=>i.id));
  return Object.entries(data?.githubReviewInbox||{}).filter(([id,event])=>ready.has(id)&&event.active&&event.seenAt==null).map(([id,event])=>({id,version:event.version}));
}
function githubReviewerLabel(item){
  if(!Array.isArray(item.requestedReviewers))return 'Reviewers…';
  if(!item.requestedReviewers.length){const review=(item.latestReviews||[]).find(r=>r.author?.login);return review?(review.state==='APPROVED'?'✓ ':review.state==='CHANGES_REQUESTED'?'! ':'↳ ')+'@'+review.author.login:'None';}
  const first=item.requestedReviewers[0].login;
  return '@'+first+(item.reviewerCount>1?' +'+(item.reviewerCount-1):'');
}
if(typeof module!=='undefined')module.exports={githubUnreadReviews,githubReviewerLabel,githubStackGroups,orderedGithubItems};
let githubReviewPanel=null;
function githubReviewerChip(item){
  const button=el('button','github-reviewer-chip',githubReviewerLabel(item));button.type='button';
  button.title='Pending reviewers: '+(item.requestedReviewers?.map(r=>'@'+r.login).join(', ')|| (Array.isArray(item.requestedReviewers)?'None':'not yet loaded'))+' · PR actions';
  button.setAttribute('aria-label','Reviewers and PR actions for '+workReference(item));button.dataset.focusKey=item.id+':reviewers';
  button.onclick=()=>openGithubReviewPanel(item,button);return button;
}
function openGithubReviewPanel(initial,anchor){
  githubReviewPanel?.remove();
  const panel=el('section','github-review-panel');panel.setAttribute('aria-label','GitHub PR actions');panel.setAttribute('role','region');githubReviewPanel=panel;document.body.append(panel);
  const position=anchor.getBoundingClientRect();panel.style.top=Math.min(position.bottom+5,Math.max(40,window.innerHeight-350))+'px';panel.style.left=Math.max(8,Math.min(position.right-310,window.innerWidth-328))+'px';
  let item=initial,pending=true,login='',failure='',requiresRefresh=false,focusControls=[],usernameInput,usernameError;
  const active=()=>panel.isConnected&&githubReviewPanel===panel;
  const focusControl=key=>{const enabled=focusControls.filter(node=>node&&!node.disabled);(enabled.find(node=>node.dataset.githubFocus===key)||enabled[0])?.focus({preventScroll:true});};
  const close=()=>{const hadFocus=panel.contains(document.activeElement);panel.remove();if(githubReviewPanel===panel)githubReviewPanel=null;if(hadFocus){const trigger=anchor.isConnected?anchor:[...document.querySelectorAll('.github-reviewer-chip')].find(node=>node.dataset.focusKey===initial.id+':reviewers');(trigger||$('#activity-open'))?.focus({preventScroll:true});}};
  async function run(action){
    if(!active()||pending||(requiresRefresh&&action!=='refresh'))return;
    const normalizedLogin=login.trim().replace(/^@/,'');
    if(action==='request-review'&&!/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/.test(normalizedLogin)){
      usernameInput.setAttribute('aria-invalid','true');usernameInput.setAttribute('aria-describedby',usernameError.id);
      usernameError.textContent=normalizedLogin?'Enter a GitHub username with 1–39 letters, numbers or hyphens, starting and ending with a letter or number.':'Enter a GitHub username.';usernameError.hidden=false;
      if(panel.contains(document.activeElement))usernameInput.focus({preventScroll:true});return;
    }
    pending=true;failure='';paint();let receipt;
    const completedMessage=()=>receipt?.status==='preview'?'Preview updated; GitHub was unchanged.':action==='ready'?'Marked ready on GitHub.':'Review requested on GitHub.';
    try{
      receipt=await api('/api/commands/invoke',{name:'github.pr.'+action,args:{id:initial.id,...(action==='request-review'?{login:normalizedLogin}: {})}});
      if(!active())return;
      if(action!=='refresh')login='';
      const result=await loadStateSnapshot({attempts:2});
      if(!active())return;
      if(!result.accepted)throw new Error('Dashboard changed while refreshing. Your newer changes were kept.');
      const current=state.items.find(i=>i.id===initial.id);if(!current)throw new Error('This PR is unavailable in the refreshed dashboard.');
      item=current;requiresRefresh=false;render();if(action!=='refresh')toast(completedMessage());
    }
    catch(error){
      if(!active())return;
      if(receipt){requiresRefresh=true;failure=(action==='refresh'?'PR refresh was accepted.':completedMessage())+' '+error.message+' Use Refresh PR to update this view'+(action==='refresh'?'.':'; do not repeat the action.');}
      else if(['remote-confirmed-local-failed','confirmation-unknown'].includes(error.data?.githubReceipt?.status)){requiresRefresh=true;failure=error.message;}
      else failure=error.message==='Not found.'?'Restart the updated service to enable PR actions.':error.message;
    }
    finally{pending=false;if(active())paint();}
  }
  function paint(){
    const focused=document.activeElement,hadFocus=panel.contains(focused),focusKey=hadFocus?focused.dataset.githubFocus:null;
    panel.replaceChildren();const head=el('div','section-heading'),closeButton=featureButton('×',close,'icon-button');closeButton.dataset.githubFocus='close';closeButton.setAttribute('aria-label','Close PR actions');head.append(el('strong','',workReference(item)),closeButton);panel.append(head);
    const draft=item.isDraft===true||item.status==='draft',closed=item.githubState==='CLOSED'||item.githubState==='MERGED'||item.status==='done';
    const requested=el('div','github-review-people');requested.append(el('small','','Requested'));
    if(!Array.isArray(item.requestedReviewers))requested.append(el('span','','Loading…'));
    else if(!item.requestedReviewers.length)requested.append(el('span','muted','None'));
    else for(const person of item.requestedReviewers)requested.append(el('span','github-review-person','@'+person.login));panel.append(requested);
    for(const review of item.latestReviews||[]){if(review.author?.login)panel.append(el('div','github-review-done','@'+review.author.login+' · '+String(review.state||'reviewed').toLowerCase().replaceAll('_',' ')));}
    let ready;if(draft&&!closed){ready=featureButton('Mark ready for review',()=>run('ready'),'button primary');ready.dataset.githubFocus='ready';ready.disabled=pending||requiresRefresh||item.canUpdate===false;panel.append(ready);}
    const form=el('form','github-review-form'),input=el('input','ui-field');input.dataset.appearance='line';input.dataset.githubFocus='reviewer';input.placeholder='GitHub username';input.setAttribute('aria-label','Request review from GitHub username');input.value=login;input.maxLength=40;input.required=true;input.autocomplete='off';input.setAttribute('list','github-review-suggestions');input.oninput=()=>{login=input.value;input.removeAttribute('aria-invalid');input.removeAttribute('aria-describedby');usernameError.hidden=true;usernameError.textContent='';};
    const list=el('datalist');list.id='github-review-suggestions';const people=new Set();for(const row of state?.items||[]){if(row.source!=='github')continue;if(row.author)people.add(row.author);for(const person of row.requestedReviewers||[])if(person.kind==='user')people.add(person.login);for(const review of row.latestReviews||[])if(review.author?.login)people.add(review.author.login);}
    for(const person of [...people].filter(p=>p!==item.author).sort())list.append(new Option(person,person));
    const submit=el('button','button','Request review');submit.dataset.githubFocus='request';submit.type='submit';submit.disabled=pending||requiresRefresh||draft||closed||item.canUpdate===false;input.disabled=submit.disabled;form.noValidate=true;form.append(input,list,submit);form.onsubmit=event=>{event.preventDefault();void run('request-review');};panel.append(form);
    usernameInput=input;usernameError=el('p','inline-error');usernameError.id='github-review-username-error';usernameError.setAttribute('role','alert');usernameError.hidden=true;panel.append(usernameError);
    if(draft)panel.append(el('small','form-hint','Mark ready before requesting a reviewer.'));
    if(closed)panel.append(el('small','form-hint','This PR is closed.'));
    else if(item.canUpdate===false)panel.append(el('small','form-hint','Your account cannot edit this PR.'));
    if(failure){const error=el('p','inline-error',failure);error.setAttribute('role','alert');panel.append(error);}
    const refreshButton=featureButton(pending?'Loading…':'Refresh PR',()=>run('refresh'),'text-button');refreshButton.dataset.githubFocus='refresh';refreshButton.disabled=pending;panel.append(refreshButton);
    focusControls=[ready,input,submit,refreshButton,closeButton];if(hadFocus)focusControl(focusKey);
  }
  panel.onkeydown=event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();}};
  pending=false;const observed=item.reviewerUpdatedAt||state?.sources?.github?.lastSuccess||0;
  if(Array.isArray(item.requestedReviewers)&&item.canUpdate!=null&&Date.now()/1000-observed<300)paint();else void run('refresh');
  focusControl();
}
if(typeof document!=='undefined')(()=>{
  let acknowledging=false;
  registerRenderHook('github:review-attention',updateReviewAttention,{order:30});
  function updateReviewAttention(){
    if(!state)return;const unread=githubUnreadReviews(state,allItems()),dot=$('#github-review-dot');dot.hidden=!unread.length;dot.title=unread.length+' new PR'+(unread.length===1?'':'s')+' ready for your review';dot.setAttribute('aria-label',dot.title);
    if(source!=='github'||acknowledging||!unread.length)return;
    const visible=new Set([...document.querySelectorAll('#work-list .row[data-work-id]')].map(row=>row.dataset.workId));const reviews=unread.filter(r=>visible.has(r.id));if(!reviews.length)return;
    acknowledging=true;
    void api('/api/github/reviews/seen',{reviews}).then(async()=>{const result=await loadStateSnapshot({attempts:2});if(result.accepted)render();}).catch(()=>{}).finally(()=>{acknowledging=false;});
  }
  document.addEventListener('click',event=>{if(githubReviewPanel&&!githubReviewPanel.contains(event.target)&&!event.target.closest('.github-reviewer-chip')){githubReviewPanel.remove();githubReviewPanel=null;}},true);
})();

// Dependencies are branch targets within the base repository, not title/body guesses.
// Use the complete cached set to retain stack identity when filters hide a parent.
function githubStackGroups(visible,all){
  const prs=all.filter(i=>i.source==='github'&&i.kind==='Pull request'&&i.githubState!=='CLOSED'&&i.githubState!=='MERGED'&&i.status!=='done');
  const byHead=new Map(),parents=new Map(),children=new Set(),byId=new Map(prs.map(i=>[i.id,i]));
  const key=(repo,branch)=>JSON.stringify([String(repo||'').toLowerCase(),branch]);
  for(const item of prs){if(!item.headRepository||!item.headRefName)continue;const k=key(item.headRepository,item.headRefName);if(!byHead.has(k))byHead.set(k,[]);byHead.get(k).push(item);}
  for(const item of prs){if(!item.baseRefName||!item.project)continue;const matches=(byHead.get(key(item.project,item.baseRefName))||[]).filter(p=>p.id!==item.id&&String(p.project).toLowerCase()===String(item.project).toLowerCase());if(matches.length===1)parents.set(item.id,matches[0].id);}
  // Fail open for corrupt or ambiguous metadata: never invent a cyclic stack.
  for(const id of [...parents.keys()]){const seen=new Set([id]);let next=parents.get(id);while(next){if(seen.has(next)){for(const member of seen)parents.delete(member);break;}seen.add(next);next=parents.get(next);}}
  for(const parent of parents.values())children.add(parent);
  const groups=new Map();
  for(const item of visible){let root=item.id,depth=0;while(parents.has(root)){root=parents.get(root);depth++;}
    const stacked=parents.has(item.id)||children.has(item.id),groupKey=stacked?'stack:'+root:'other:'+item.project;
    if(!groups.has(groupKey))groups.set(groupKey,{root:stacked?byId.get(root):null,repo:item.project,rows:[]});
    groups.get(groupKey).rows.push({item,depth,parent:byId.get(parents.get(item.id))});
  }
  for(const group of groups.values())if(group.root)group.rows.sort((a,b)=>a.depth-b.depth||a.item.reference.localeCompare(b.item.reference,undefined,{numeric:true}));
  return [...groups.values()].sort((a,b)=>Number(!!b.root)-Number(!!a.root));
}
function orderedGithubItems(visible,all){return githubStackGroups(visible,all).flatMap(group=>group.rows.map(row=>row.item));}
function appendGithubStacks(list,visible,all,limit,compact){
  const visibleIds=new Set(visible.map(i=>i.id));let remaining=limit;
  for(const stack of githubStackGroups(visible,all)){
    if(!remaining)break;
    const group=el('section',(compact?'github-inline-stack':'group')+' github-stack'),heading=el('h2','group-header');
    const title=stack.root?(compact?`Stack ${stack.root.reference}`:`${scopeLabel(stack.repo)} · ${stack.root.reference} stack`):`${scopeLabel(stack.repo)} · Unstacked`;
    heading.append(document.createTextNode(title),el('span','group-count',stack.rows.length),el('span','group-line'));
    if(stack.root)heading.title='Base first · each indented PR targets the branch above it';
    if(!compact||stack.root)group.append(heading);
    for(const {item,depth,parent} of stack.rows.slice(0,remaining)){
      const wrap=el('div','github-stack-row'+(depth?' stack-child':''));wrap.style.setProperty('--stack-depth',Math.min(depth,4));
      if(parent&&!visibleIds.has(parent.id)){const note=el('div','stack-parent-note');note.append(document.createTextNode('↳ Based on '));const link=el('a','',workReference(parent));link.href=parent.url;link.target='_blank';link.rel='noopener noreferrer';note.append(link,document.createTextNode(' · outside current view'));wrap.append(note);}
      const row=renderRow(item);
      if(stack.root){const badge=el('span','github-stack-badge','⎇ '+(depth+1));badge.title=`Stack ${workReference(stack.root)} · layer ${depth+1}`+(parent?` · based on ${workReference(parent)}`:' · base PR');badge.setAttribute('aria-label',badge.title);$('.row-title',row).prepend(badge);}
      wrap.append(row);group.append(wrap);
    }
    remaining-=Math.min(remaining,stack.rows.length);list.append(group);
  }
}

const activeChatCI={key:'',at:0,pending:false,retryAt:0,retryReason:'',sourceAdmission:null,sourceAdmissionGeneration:0,presentation:null};
const chatCIChecks=new Map();
function chatCIContext(){
  if(document.hidden||source!=='agents')return null;
  const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);
  const record=selectedAgentKey.startsWith('record:')?state?.sessions.find(r=>r.id===selectedAgentKey.slice(7)):candidate&&recordForCandidate(candidate);
  if(!record)return null;
  return {record,key:record.id+'\0'+(record.items||[]).map(i=>i.id).sort().join(','),prs:agentWork(record).filter(i=>i.source==='github'&&i.kind==='Pull request')};
}
function syncChatCISourceAdmission(){
  const info=state.sources.github||{};
  // Healthy full-sync intervals are deliberately not part of CI admission.
  const admission=info.disabled===true?'disabled':info.failures>0&&info.nextAttempt>Date.now()/1000?'backoff:'+info.nextAttempt:'ready';
  if(activeChatCI.sourceAdmission!==null&&activeChatCI.sourceAdmission!==admission){
    activeChatCI.at=0;activeChatCI.sourceAdmissionGeneration++;
    if(['disabled','backoff'].includes(activeChatCI.retryReason)){activeChatCI.retryAt=0;activeChatCI.retryReason='';}
    for(const [id,checked]of chatCIChecks)if(['disabled','backoff'].includes(checked.reason)){const {reason,retryAt,...retained}=checked;chatCIChecks.set(id,retained);}
  }
  activeChatCI.sourceAdmission=admission;return activeChatCI.sourceAdmissionGeneration;
}
function chatCIStatus(item){
  const checked=chatCIChecks.get(item.id),info=state.sources.github||{};
  const reason=info.disabled===true?'disabled':info.failures>0&&info.nextAttempt>Date.now()/1000?'backoff':checked?.reason;
  const notices={disabled:'GitHub synchronization is disabled; showing cached checks.',backoff:'CI refresh is paused after a GitHub error; showing cached checks.','ci-backoff':'CI refresh is paused after a GitHub error; showing cached checks.',demo:'Sample checks; no GitHub request was made.','state-changed':'Checks changed during refresh; showing last loaded checks.',presentation:'Checks refreshed on server; showing last loaded checks.'};
  const notice=notices[reason]||(checked?.error?'CI refresh failed; showing cached checks.':'');
  const retryAt=reason==='backoff'&&info.failures>0?info.nextAttempt:checked?.retryAt;
  const retry=notice&&Number.isFinite(retryAt)&&retryAt>Date.now()/1000?' Next attempt '+new Date(retryAt*1000).toLocaleTimeString()+'.':'';
  const age=checked?.at?'Last checked '+relative(checked.at):info.lastSuccess?'Last synced '+relative(info.lastSuccess):'Checks have not been refreshed in this view.';
  return age+(notice?' · '+notice+retry:'');
}
function chatCIHint(item){
  const info=ciInfo(item.ci),failed=(item.ci?.checks||[]).filter(c=>c.state==='failed');
  return info.label+(item.ci?.total?' · '+item.ci.total+' checks':'')+(failed.length?'\n'+failed.map(c=>c.name).join('\n'):'')+'\n'+chatCIStatus(item);
}
function chatCIBadgeLabel(item){return 'CI for '+workReference(item)+': '+ciInfo(item.ci).label+'. '+chatCIStatus(item);}
function paintChatCIHints(){
  const linked=chatCIContext()?.prs||[];
  for(const badge of document.querySelectorAll('[data-chat-ci]')){const item=allItems().find(i=>i.id===badge.dataset.chatCi)||linked.find(i=>i.id===badge.dataset.chatCi);if(item){badge.title=chatCIHint(item);badge.setAttribute('aria-label',chatCIBadgeLabel(item));}}
}
async function syncActiveChatCI(){
  const context=chatCIContext();if(!context){activeChatCI.key='';activeChatCI.at=0;return;}
  const {record,key,prs}=context,admission=syncChatCISourceAdmission();
  if(activeChatCI.key!==key){activeChatCI.key=key;activeChatCI.at=0;activeChatCI.retryAt=0;activeChatCI.retryReason='';}
  if(activeChatCI.pending||!prs.length)return;
  const info=state.sources.github||{},time=Date.now()/1000;
  if(!activeChatCI.presentation&&(info.disabled===true||info.failures>0&&info.nextAttempt>time)){paintChatCIHints();return;}
  if(Date.now()-activeChatCI.at<30000||activeChatCI.retryAt>time)return;
  activeChatCI.at=Date.now();activeChatCI.pending=true;
  try{
    // One bounded pending presentation survives read failures and chat switches.
    // Recover it with reads only before asking the provider for any more checks.
    if(!activeChatCI.presentation){
      const result=await api('/api/commands/invoke',{name:'chats.ci.refresh',args:{recordId:record.id}});
      const currentAdmission=syncChatCISourceAdmission();
      if(result.status==='skipped'){
        if(['disabled','backoff'].includes(result.reason)&&currentAdmission!==admission)return;
        for(const id of result.itemIds)chatCIChecks.set(id,{...chatCIChecks.get(id),reason:result.reason,retryAt:result.retryAt});
        if(activeChatCI.key===key){activeChatCI.retryAt=Number.isFinite(result.retryAt)?result.retryAt:0;activeChatCI.retryReason=result.reason;}
        return;
      }
      for(const id of result.skippedItemIds||[])chatCIChecks.set(id,{...chatCIChecks.get(id),reason:'state-changed'});
      activeChatCI.presentation={key,itemIds:result.itemIds,checkedAt:result.checkedAt};
    }
    const presentation=activeChatCI.presentation;
    try{
      const snapshot=await loadStateSnapshot({attempts:2});
      if(!snapshot.accepted)throw new Error('Dashboard changed during refresh.');
      for(const id of presentation.itemIds)chatCIChecks.set(id,{at:presentation.checkedAt});
      activeChatCI.presentation=null;
      if(chatCIContext()?.key===presentation.key)render();
    }catch(_){for(const id of presentation.itemIds)chatCIChecks.set(id,{...chatCIChecks.get(id),reason:'presentation'});}
  }catch(_){for(const pr of prs)chatCIChecks.set(pr.id,{...chatCIChecks.get(pr.id),error:true});}
  finally{
    activeChatCI.pending=false;paintChatCIHints();
    if(activeChatCI.key!==key)void syncActiveChatCI();
  }
}
if(typeof window!=='undefined'){
  setInterval(()=>{void syncActiveChatCI();},1000);
  document.addEventListener('visibilitychange',()=>{void syncActiveChatCI();});
}
