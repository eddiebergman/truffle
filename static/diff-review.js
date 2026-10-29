'use strict';
let reviewState=null,reviewRenderer=null,reviewObserver=null,reviewSizeObserver=null,reviewFileSizeObserver=null,reviewScrollFrame=null;
const sharedReviewDrafts=new Map();
function reviewDrafts(){return Object.fromEntries(sharedReviewDrafts);}
function acceptReviewDraft(id,draft){const previous=sharedReviewDrafts.get(id);if((previous?.revision||0)>(draft.revision||0))return;if(JSON.stringify(sharedReviewDrafts.get(id))===JSON.stringify(draft))return;sharedReviewDrafts.set(id,draft);while(sharedReviewDrafts.size>20)sharedReviewDrafts.delete(sharedReviewDrafts.keys().next().value);window.dispatchEvent(new Event('mydash-review-draft'));if(reviewState?.record.id===id){reviewState.draft=draft;paintReviewComments();if(JSON.stringify(previous?.comments)!==JSON.stringify(draft.comments))updateReviewAnnotations();if(!reviewState.noteDirty&&document.activeElement!==$('#diff-review-note'))$('#diff-review-note').value=draft.note||'';}}
async function reviewAction(id,action,fields={}){const result=await api('/api/session/review-draft',{id,action,...fields});const previous=sharedReviewDrafts.get(id);if(action==='read'&&previous?.revision===result.review.revision){for(const key of ['freshness','freshnessCheckedAt'])if(previous[key]!==undefined)result.review[key]=previous[key];for(const key of ['comments','assessments'])for(const entry of result.review[key]||[]){const old=(previous[key]||[]).find(x=>x.id===entry.id);if(old?.freshness)entry.freshness=old.freshness;}}acceptReviewDraft(id,result.review);return result.review;}
const activeDiffCache={key:'',entries:new Map(),bytes:0,epoch:0};
function activateDiffCache(key){if(activeDiffCache.key===key)return;activeDiffCache.key=key;activeDiffCache.entries.clear();activeDiffCache.bytes=0;activeDiffCache.epoch++;}
async function cachedReviewFile(record,path,directory){
  const key=record.id+'\0'+directory,cache=activeDiffCache,epoch=cache.epoch,entry=cache.entries.get(path);
  if(cache.key===key&&entry&&Date.now()-entry.at<10000)return entry.promise;
  const promise=api('/api/session/git-file',{id:record.id,path,cwd:directory});
  const next={promise,at:Date.now(),bytes:0};if(cache.key===key)cache.entries.set(path,next);
  try{const data=await promise;if(cache.key===key&&cache.epoch===epoch&&cache.entries.get(path)===next){next.bytes=(data.oldContent.length+data.newContent.length)*2;cache.bytes+=next.bytes-(entry?.bytes||0);while(cache.entries.size>24||cache.bytes>8*1024*1024){const [old,item]=cache.entries.entries().next().value;cache.bytes-=item.bytes;cache.entries.delete(old);}}return data;}catch(error){if(cache.entries.get(path)===next)cache.entries.delete(path);throw error;}
}
let reviewRefreshAt=0,reviewRefreshPending=false,reviewFreshnessAt=0;
function syncActiveDiff(){
  const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey);
  const record=source==='agents'?(selectedAgentKey.startsWith('record:')?state.sessions.find(r=>r.id===selectedAgentKey.slice(7)):candidate&&recordForCandidate(candidate)):null;
  const directory=record&&selectedGitDirectory(record),key=record?record.id+'\0'+directory:'';
  if(activeDiffCache.key!==key){activateDiffCache(key);reviewRefreshAt=0;reviewFreshnessAt=0;if(reviewState&&reviewState.record.id!==record?.id)$('#diff-review-dialog').close();if(record)void warmActiveDiff(record,directory,key);}
  if(record&&!document.hidden&&!reviewRefreshPending&&Date.now()-reviewRefreshAt>5000){reviewRefreshPending=true;const draft=sharedReviewDrafts.get(record.id),check=Date.now()-reviewFreshnessAt>30000&&(draft?.run||draft?.comments?.length);if(check)reviewFreshnessAt=Date.now();void reviewAction(record.id,check?'check':'read').catch(()=>{}).finally(()=>{reviewRefreshPending=false;reviewRefreshAt=Date.now();});}
}
async function warmActiveDiff(record,directory,key){
  try{const overview=await api('/api/session/git-diff',{id:record.id,cwd:directory});if(activeDiffCache.key!==key)return;void import('/vendor/diffs/diff-viewer.js');for(const file of overview.files.filter(f=>f.additions!==null).slice(0,4)){if(activeDiffCache.key!==key)return;await cachedReviewFile(record,file.path,directory).catch(()=>{});}}catch(_){}
}
setInterval(syncActiveDiff,1000);
function reviewRunLabel(draft){
  const run=draft?.run;if(!run||run.state==='cancelled')return '';
  if(['requested','reviewing'].includes(run.state)&&Date.now()/1000-run.updatedAt>7200)return 'Review unverified';
  if(run.state==='complete'&&draft.freshness&&draft.freshness!=='current')return draft.freshness==='outdated'?'Review outdated':'Review unverified';
  return {requested:'Review requested',reviewing:'Reviewing',complete:'Review done',failed:'Review failed'}[run.state]||'';
}
function reviewDispatchCurrent(current){return reviewState===current&&$('#diff-review-dialog').open;}
async function refreshReviewDispatch(current,receipt){
 return refreshChatDispatch(receipt,{canPresent:()=>reviewDispatchCurrent(current),refresh:async()=>{await reviewAction(current.record.id,'read');return loadStateSnapshot({attempts:2});}});
}
function paintReviewDispatch(current){
 if(!current.dispatchPanel){
  const panel=chatDispatchRecovery(current.record.id,{canPresent:()=>reviewDispatchCurrent(current),refresh:async()=>{await reviewAction(current.record.id,'read');return loadStateSnapshot({attempts:2});},onChange:()=>{if(reviewDispatchCurrent(current))paintReviewComments();},onInspect:()=>{if(!reviewDispatchCurrent(current))return;const record=state.sessions.find(row=>row.id===current.record.id);if(record){$('#diff-review-dialog').close();selectAgent(record,candidateForRecord(record));}}});
  panel.id='diff-review-dispatch';current.dispatchPanel=panel;$('#diff-review-error').insertAdjacentElement('afterend',panel);
 }
 const blocked=chatDispatchBlocked(current.record.id)||!Number.isSafeInteger(current.draft.revision);$('#diff-review-clear-progress').disabled=blocked;if(blocked){$('#diff-review-agent').disabled=true;$('#diff-review-send').disabled=true;$('#diff-review-new').disabled=true;}
 current.dispatchPanel.refreshDispatch();
}
async function startAgentReview(){
 const current=reviewState;if(!current||current.reviewStarting||current.pending)return;current.reviewStarting=true;paintReviewComments();
 try{
  await saveReviewNote(current);if(!reviewDispatchCurrent(current))return;
  const receipt=await submitChatDispatch('review.request','review-request',{recordId:current.record.id,revision:current.draft.revision});
  if(receipt.finalized){await refreshReviewDispatch(current,receipt);if(reviewDispatchCurrent(current)){window.truffleGuidance?.complete('review.agent-scope');window.truffleGuidance?.complete('review.pending-comments');}}
 }catch(error){if(reviewDispatchCurrent(current))$('#diff-review-error').textContent=error.message;}
 finally{current.reviewStarting=false;if(reviewDispatchCurrent(current))paintReviewComments();}
}
async function sendReviewFeedback(){
 const current=reviewState;if(!current||current.pending||current.reviewStarting)return;current.pending=true;$('#diff-review-note').disabled=true;paintReviewComments();
 try{
  await saveReviewNote(current);if(!reviewDispatchCurrent(current))return;
  const receipt=await submitChatDispatch('review.send','review-feedback',{recordId:current.record.id,revision:current.draft.revision});
  if(receipt.finalized&&await refreshReviewDispatch(current,receipt)&&reviewDispatchCurrent(current)){
   current.noteDirty=false;current.noteBase=current.draft.note;$('#diff-review-note').value=current.draft.note||'';hideReviewComposer();window.truffleGuidance?.complete('review.pending-comments');$('#diff-review-dialog').close();toast(receipt.message);
  }
 }catch(error){if(reviewDispatchCurrent(current))$('#diff-review-error').textContent=error.message;}
 finally{current.pending=false;if(reviewState===current){$('#diff-review-note').disabled=false;if($('#diff-review-dialog').open)paintReviewComments();}}
}
let reviewBackgroundPoll=false;
setInterval(()=>{if(document.hidden||reviewBackgroundPoll)return;const ids=[...sharedReviewDrafts].filter(([id,draft])=>id!==activeDiffCache.key.split('\0')[0]&&['requested','reviewing'].includes(draft.run?.state)).slice(0,4).map(([id])=>id);if(ids.length){reviewBackgroundPoll=true;void Promise.all(ids.map(id=>reviewAction(id,'read').catch(()=>{}))).finally(()=>{reviewBackgroundPoll=false;});}},5000);
function reviewEntryPending(draft,entry){return !entry.reviewedAt&&!(draft.run?.state==='complete'&&!Array.isArray(draft.run.inputCommentIds)&&entry.createdAt&&entry.createdAt<=(draft.run.completedAt||draft.run.updatedAt));}
function pendingReviewComments(draft){return draft.comments.filter(entry=>reviewEntryPending(draft,entry));}
function pendingReviewAssessments(draft){return (draft.assessments||[]).filter(entry=>!entry.sentAt&&reviewEntryPending(draft,entry));}
function pendingReviewNote(draft){return draft.note&&draft.note!==draft.reviewedNote?draft.note:'';}
function disposeReview(){if(reviewScrollFrame!==null)cancelAnimationFrame(reviewScrollFrame);reviewScrollFrame=null;reviewFileSizeObserver?.disconnect();reviewFileSizeObserver=null;reviewSizeObserver?.disconnect();reviewSizeObserver=null;reviewObserver?.disconnect();reviewObserver=null;reviewRenderer?.dispose();reviewRenderer=null;}
function reviewAuthorPill(author){return el('span','review-author-pill '+((author||'You')==='You'?'you':'agent'),author||'You');}
function highlightReviewMarkup(node,path){
  if(node.querySelector('code'))void import('/vendor/diffs/diff-viewer.js').then(viewer=>viewer.highlightReviewCode(node,path)).catch(()=>{});
}
function reviewCommentBody(text,path){
  const node=el('div','review-comment-markdown');node.innerHTML=renderBriefingMarkdown(String(text||'').slice(0,16000));
  highlightReviewMarkup(node,path);
  return node;
}
function reviewSourceExcerpt(text,path,label='Earlier file version'){
  const source=String(text||''),bounded=source.slice(0,16000),node=el('figure','review-source-excerpt');
  const caption=el('figcaption','',label),pre=el('pre'),code=el('code');
  // Source is literal text, never Markdown or HTML; keep indentation and newlines.
  code.textContent=bounded;pre.append(code);node.append(caption,pre);
  if(bounded.length<source.length)node.append(el('small','form-hint','Snippet truncated after 16,000 characters.'));
  highlightReviewMarkup(node,path);
  return node;
}
function renderReviewAnnotations(comments,path){
  const node=el('div','review-line-comments');
  for(const comment of comments){const entry=el('article','review-inline-comment');entry.append(reviewAuthorPill(comment.author),reviewCommentBody(comment.text,path));node.append(entry);}
  return node;
}
function reviewAnnotations(file){const groups=new Map();for(const c of reviewState.draft.comments.filter(c=>c.path===file.path&&c.root===file.root&&c.fingerprint===file.fingerprint)){const key=c.side+':'+c.end;if(!groups.has(key))groups.set(key,{side:c.side,lineNumber:c.end,metadata:[]});groups.get(key).metadata.push({author:c.author||'You',text:c.text});}return [...groups.values()];}
function updateReviewAnnotations(){if(!reviewState)return;for(const entry of reviewState.files.values())if(entry.data)reviewRenderer?.annotate(entry.data.path,reviewAnnotations(entry.data));}
function positionQuickReview(current,anchor){
  if(reviewState!==current||!current.quick)return;
  const conversation=$('#agent-workspace .agent-conversation');if(!conversation)return;
  const bounds=conversation.getBoundingClientRect(),dialog=$('#diff-review-dialog');
  const scale=Number(getComputedStyle(document.documentElement).getPropertyValue('--ui-scale'))||1;
  const area={left:bounds.left/scale,right:bounds.right/scale,top:bounds.top/scale,height:bounds.height/scale};
  const viewport={width:window.innerWidth/scale,height:window.innerHeight/scale};
  const desired=anchor?.getBoundingClientRect?.().top/scale||current.inspectorTop||area.top;
  const height=(typeof setting==='function'?setting('previewHeight',60):60)/100*area.height;
  current.inspectorTop=desired;dialog.style.setProperty('--inspector-height',height+'px');
  dialog.style.setProperty('--inspector-left',Math.max(8,area.left)+'px');
  dialog.style.setProperty('--inspector-right',Math.max(8,viewport.width-area.right)+'px');
  dialog.style.setProperty('--inspector-top',Math.max(Math.max(38,area.top),Math.min(desired,viewport.height-height-8))+'px');
}
function savedReviewUI(record,directory){try{return JSON.parse(localStorage.getItem('mydash-review-ui')||'{}')[record.id+'\0'+directory]||{};}catch(_){return {};}}
function rememberReviewUI(){
  const current=reviewState;if(!current)return;
  const body=$('#diff-review-body'),bounds=body.getBoundingClientRect(),top=bounds.top;
  const visible=bounds.width===0?null:[...current.files.values()].find(entry=>entry.section.getBoundingClientRect().bottom>top);
  const previous=savedReviewUI(current.record,current.directory);
  const file=current.file;
  const value={mode:current.mode,drawer:$('#diff-review-drawer').open,expanded:[...$('#diff-review-assessments').querySelectorAll('details[open]')].map(d=>d.dataset.assessmentId),collapsed:[...$('#diff-review-files').querySelectorAll('details:not([open])')].map(d=>d.querySelector('summary')?.textContent),path:visible?.section.dataset.path||previous.path,offset:visible?top-visible.section.getBoundingClientRect().top:previous.offset||0,comment:$('#diff-review-comment').value,range:current.range,file:file?{path:file.path,fingerprint:file.fingerprint}:null,note:$('#diff-review-note').value,noteBase:current.noteBase??current.draft.note??'',noteDirty:!!current.noteDirty};
  try{const key=current.record.id+'\0'+current.directory,all=JSON.parse(localStorage.getItem('mydash-review-ui')||'{}');delete all[key];all[key]=value;const entries=Object.entries(all).slice(-30);localStorage.setItem('mydash-review-ui',JSON.stringify(Object.fromEntries(entries)));}catch(_){}
}
let reviewUISaveTimer;
function scheduleReviewUISave(){clearTimeout(reviewUISaveTimer);reviewUISaveTimer=setTimeout(rememberReviewUI,150);}
function hideReviewComposer(){const composer=$('#diff-review-composer');composer.hidden=true;if(reviewState){reviewState.range=null;reviewState.file=null;}}
function showReviewComposer(current){
  const composer=$('#diff-review-composer'),dialog=$('#diff-review-dialog'),bounds=dialog.getBoundingClientRect(),point=current.selectionPoint||{x:bounds.left+bounds.width/2,y:bounds.top+70};
  composer.hidden=false;
  const width=Math.min(380,window.innerWidth-24);composer.style.width=width+'px';
  const height=composer.getBoundingClientRect().height;
  composer.style.left=Math.max(12,Math.min(point.x,bounds.right-width-12))+'px';
  const below=point.y+12;composer.style.top=Math.max(12,Math.min(below+height<window.innerHeight-12?below:point.y-height-12,window.innerHeight-height-12))+'px';
  $('#diff-review-comment').focus({preventScroll:true});
}
function setReviewMode(mode){hideReviewComposer();if(!reviewState)return;reviewState.mode=mode;try{localStorage.setItem('mydash-diff-layout',mode);}catch(_){}for(const name of ['unified','split'])$('#diff-mode-'+name).setAttribute('aria-pressed',String(name===mode));reviewRenderer?.setStyle(mode);scheduleReviewUISave();}
async function openDiffReview(record,path,directory=selectedGitDirectory(record),anchor=null){
  const returnDialog=$('#diff-review-dialog'),returnTrigger=anchor&&typeof anchor==='object'?anchor:document.activeElement,preserveReturn=returnDialog.open;
  if(!requestDialogNavigation())return false;
  captureDialogReturn(returnDialog,{trigger:returnTrigger,scope:gitReviewFocusKey(record,directory),fallbackKey:gitReviewFocusKey(record,directory),preserve:preserveReturn,isCurrent:()=>{
    const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey),selected=selectedAgentKey.startsWith('record:')?state.sessions.find(row=>row.id===selectedAgentKey.slice(7)):candidate&&recordForCandidate(candidate);
    return source==='agents'&&selected?.id===record.id&&selectedGitDirectory(selected)===directory;
  }});
  rememberReviewUI();disposeReview();$('#diff-review-dispatch')?.remove();
  const quick=!!path&&anchor!=='full';let mode=typeof setting==='function'?setting(quick?'fileDiffLayout':'diffLayout',quick?'split':'unified'):(quick?'split':'unified');try{if(!quick&&localStorage.getItem('mydash-diff-layout')==='split')mode='split';}catch(_){}
  const ui=savedReviewUI(record,directory);if(['split','unified'].includes(ui.mode))mode=ui.mode;
  const stored=sharedReviewDrafts.get(record.id);
  const current=reviewState={record,directory,draft:stored||{comments:[],note:''},files:new Map(),file:null,range:null,pending:false,mode,queue:[],loading:false,quick,noteDirty:!!ui.noteDirty,noteBase:ui.noteBase,restoredExpanded:new Set(ui.expanded||[])};
  const dialog=$('#diff-review-dialog');dialog.classList.toggle('quick-file-review',quick);
  $('#diff-review-title').textContent=quick?path:'Review';const subtitle=$('#diff-review-subtitle');subtitle.replaceChildren();if(!quick&&gitDiffDirectories(record).length>1)subtitle.append(gitDirectorySelector(record,directory,next=>openDiffReview(record,undefined,next)));else subtitle.textContent=record.title;subtitle.title=record.title;
  $('#diff-review-expand').hidden=!quick;$('#diff-review-expand').onclick=()=>openDiffReview(record,path, directory, 'full');
  hideReviewComposer();$('#diff-review-history').open=false;$('#diff-review-drawer').open=false;$('#diff-review-error').textContent='';$('#diff-review-body').replaceChildren(el('p','form-hint','Loading local diff…'));$('#diff-review-files').replaceChildren();$('#diff-review-note').disabled=false;$('#diff-review-note').value=current.draft.note||'';$('#diff-review-comment').value=ui.comment||'';if(ui.noteDirty)$('#diff-review-note').value=ui.note||'';$('#diff-review-add').disabled=true;$('#diff-review-range').textContent='Select a line or range to comment';
  if(current.quick){positionQuickReview(current,anchor);dialog.show();reviewSizeObserver=new ResizeObserver(()=>positionQuickReview(current));reviewSizeObserver.observe($('#agent-workspace .agent-conversation'));}else dialog.showModal();paintReviewComments();setReviewMode(mode);
  try{
    const filePromise=path?cachedReviewFile(record,path,directory).catch(error=>({loadError:error})):null;
    const [overview,viewer]=await Promise.all([api('/api/session/git-diff',{id:record.id,cwd:directory}),import('/vendor/diffs/diff-viewer.js'),reviewAction(record.id,'check')]);
    if(reviewState!==current||!dialog.open)return;
    current.root=overview.root;paintReviewComments();current.restoredExpanded=null;const body=$('#diff-review-body');body.onkeydown=()=>{current.selectionPoint=null;};body.replaceChildren();reviewRenderer=viewer.createDiffReview(body,current.mode,renderReviewAnnotations);
    const files=current.quick?overview.files.filter(f=>f.path===path):overview.files;
    for(const file of files){const section=el('section','review-file-section');section.dataset.path=file.path;const heading=el('h3','review-file-heading');heading.append(el('span','',file.path),el('small','',file.additions===null?'Binary':`+${file.additions} −${file.deletions}`));const content=el('div','review-file-content');content.append(el('p','form-hint','Loading diff…'));section.append(heading,content);body.append(section);current.files.set(file.path,{section,content,status:'idle',data:null});}
    appendGitFileTree($('#diff-review-files'),files,file=>{const button=featureButton('',()=>loadReviewFile(file.path),'review-file');button.append(el('span','review-tree-file-name',file.path.split('/').pop()),gitChangeCounts(file.additions,file.deletions));button.title=file.path;button.dataset.path=file.path;button.dataset.reviewFile=file.path;button.dataset.reviewRoot=overview.root;button.dataset.reviewRecord=record.id;paintReviewFileMarkers(button,current.draft);return button;});
    const first=files.some(f=>f.path===path)?path:files.some(f=>f.path===ui.path)?ui.path:files[0]?.path;
    if(first){if(filePromise){const preload=await filePromise;if(!preload.loadError&&current.files.has(path))current.files.get(path).preloaded=preload;}await loadReviewFile(first);}
    else body.append(el('p','form-hint','No changes against main.'));
    if(reviewState!==current)return;
    if(!path&&first===ui.path&&ui.offset)body.scrollTop+=ui.offset;
    for(const folder of $('#diff-review-files').querySelectorAll('details'))if((ui.collapsed||[]).includes(folder.querySelector('summary')?.textContent))folder.open=false;
    $('#diff-review-drawer').open=!!ui.drawer;
    if(ui.range&&ui.file){const entry=current.files.get(ui.file.path);if(entry?.data?.fingerprint===ui.file.fingerprint){current.file=entry.data;current.range=ui.range;$('#diff-review-range').textContent=ui.file.path+' · '+ui.range.start+'–'+ui.range.end;$('#diff-review-add').disabled=false;showReviewComposer(current);}}

    if(reviewState!==current||!dialog.open)return;
    reviewObserver=new IntersectionObserver(entries=>{for(const entry of entries)if(entry.isIntersecting)queueReviewFile(entry.target.dataset.path,false);},{root:body,rootMargin:'400px 0px'});
    for(const entry of current.files.values())if(entry.status==='idle')reviewObserver.observe(entry.section);
    reviewFileSizeObserver=new ResizeObserver(scheduleReviewFileHighlight);
    for(const entry of current.files.values())reviewFileSizeObserver.observe(entry.section);
    scheduleReviewFileHighlight();
  }catch(error){if(reviewState===current)$('#diff-review-error').textContent=error.message;}
}
function highlightReviewFile(path){
  const current=reviewState;if(!current||current.activePath===path)return;current.activePath=path;
  const tree=$('#diff-review-files');
  for(const button of tree.querySelectorAll('button[data-path]')){
    const active=button.dataset.path===path;button.classList.toggle('selected',active);
    if(!active){button.removeAttribute('aria-current');continue;}
    button.setAttribute('aria-current','location');
    for(let parent=button.parentElement;parent&&parent!==tree;parent=parent.parentElement)if(parent.tagName==='DETAILS')parent.open=true;
    const bounds=tree.getBoundingClientRect(),row=button.getBoundingClientRect();
    if(row.top<bounds.top)tree.scrollTop+=row.top-bounds.top;
    else if(row.bottom>bounds.bottom)tree.scrollTop+=row.bottom-bounds.bottom;
  }
}
function syncReviewFileHighlight(){
  if(!reviewState||reviewState.quick||!$('#diff-review-dialog').open)return;
  const bounds=$('#diff-review-body').getBoundingClientRect(),line=bounds.top+Math.min(64,bounds.height/4);
  for(const [path,entry] of reviewState.files){const rect=entry.section.getBoundingClientRect();if(rect.bottom>line&&rect.top<bounds.bottom){highlightReviewFile(path);return;}}
}
function scheduleReviewFileHighlight(){
  if(reviewScrollFrame!==null)return;
  reviewScrollFrame=requestAnimationFrame(()=>{reviewScrollFrame=null;syncReviewFileHighlight();});
}
function loadReviewFile(path){$('#diff-review-drawer').open=false;const entry=reviewState.files.get(path);if(!entry)return;entry.section.scrollIntoView({block:'start'});highlightReviewFile(path);return queueReviewFile(path,true);}
function queueReviewFile(path,forced){const current=reviewState,entry=current.files.get(path);if(!entry||entry.status==='loaded'||entry.status==='loading'||entry.status==='error'&&!forced)return;current.queue=current.queue.filter(x=>x.path!==path);if(forced)current.queue.unshift({path,forced});else current.queue.push({path,forced});entry.status='queued';return drainReviewFiles(current);}
async function drainReviewFiles(current){
  if(current.loading)return;current.loading=true;
  try{while(current.queue.length&&reviewState===current&&$('#diff-review-dialog').open){
    const {path,forced}=current.queue.shift(),entry=current.files.get(path);
    entry.status='loading';
    try{const data=entry.preloaded||await cachedReviewFile(current.record,path,current.directory);delete entry.preloaded;if(reviewState!==current||!$('#diff-review-dialog').open)return;
      entry.data=data;entry.status='loaded';entry.content.replaceChildren();entry.content.classList.add('loaded');reviewObserver?.unobserve(entry.section);
      reviewRenderer.mount(entry.content,data,reviewAnnotations(data),range=>{if(!range){hideReviewComposer();return;}if(range.endSide&&range.side!==range.endSide){$('#diff-review-range').textContent='Select lines on one side of the diff';return;}current.file=data;current.range={start:Math.min(range.start,range.end),end:Math.max(range.start,range.end),side:range.side||'additions'};$('#diff-review-range').textContent=`${path} · ${current.range.side==='deletions'?'old':'new'} ${current.range.start}–${current.range.end}`;$('#diff-review-add').disabled=false;showReviewComposer(current);});if(forced)entry.section.scrollIntoView({block:'start'});
    }catch(error){if(reviewState!==current||!$('#diff-review-dialog').open)return;entry.status='error';entry.content.replaceChildren(el('p','form-hint',error.message));reviewObserver?.unobserve(entry.section);}
  }}finally{current.loading=false;}
}
function paintReviewComments(){
  const current=reviewState,list=$('#diff-review-comments'),assessments=$('#diff-review-assessments'),dialog=$('#diff-review-dialog'),focused=document.activeElement;
  const region=focused&&dialog?.open&&[list,assessments].find(node=>node.contains(focused)),entry=region&&focused.closest('[data-review-entry]');
  const focus=entry?{node:focused,kind:entry.dataset.reviewEntry,id:entry.dataset.reviewId,control:focused.closest('[data-review-control]')?.dataset.reviewControl,index:[...region.children].indexOf(entry)}:null,controls=[];
  const control=(node,kind,id,role)=>{node.dataset.reviewControl=role;controls.push({node,kind,id,role});return node;};
  const body=(text,path,kind,id)=>{const node=reviewCommentBody(text,path),occurrences=new Map();for(const link of node.querySelectorAll('a[href]')){const href=link.getAttribute('href'),index=occurrences.get(href)||0;occurrences.set(href,index+1);control(link,kind,id,JSON.stringify(['link',href,index]));}return node;};
  list.replaceChildren();
  const expanded=new Set([...assessments.querySelectorAll('details[open]')].map(d=>d.dataset.assessmentId));assessments.replaceChildren();
  for(const a of current.draft.assessments||[]){const section=el('details','review-assessment'),summary=el('summary','',(a.author.toLowerCase().replace(/[-_.]/g,' ').includes(a.pass.replace(/[-_.]/g,' '))?'':' · '+a.pass)+(a.sentAt?' · sent':''));section.dataset.assessmentId=a.id;section.dataset.reviewEntry='assessment';section.dataset.reviewId=a.id;control(summary,'assessment',a.id,'summary');section.open=expanded.has(a.id)||current.restoredExpanded?.has(a.id);if(a.freshness&&a.freshness!=='current')summary.append(el('small','review-freshness '+a.freshness,a.freshness==='outdated'?' · outdated':' · unverified'));summary.prepend(reviewAuthorPill(a.author));section.append(summary,control(featureButton('Remove',async()=>{try{await reviewAction(current.record.id,'remove',{assessmentIds:[a.id]});}catch(error){$('#diff-review-error').textContent=error.message;}},'text-button'),'assessment',a.id,'remove'),el('small','form-hint',(a.root||'Conversation')+' · '+new Date(a.createdAt*1000).toLocaleString()),body(a.text,undefined,'assessment',a.id));assessments.append(section);}
  const unsent=pendingReviewAssessments(current.draft).length,pendingComments=pendingReviewComments(current.draft).length,pendingNote=current.noteDirty?($('#diff-review-note').value!==current.draft.reviewedNote?$('#diff-review-note').value:''):pendingReviewNote(current.draft);
  for(const [index,c]of current.draft.comments.entries()){const row=el('div','review-comment'),head=el('div','review-comment-head');const anchor=featureButton(`${c.path}:${c.start}${c.end!==c.start?'–'+c.end:''}`,()=>{if(current.directory===c.root&&current.files.has(c.path))loadReviewFile(c.path);else openDiffReview(current.record,c.path,c.root);},'review-comment-anchor');row.dataset.reviewEntry='comment';row.dataset.reviewId=c.id;control(anchor,'comment',c.id,'anchor');const stale=current.files.get(c.path)?.data;head.append(reviewAuthorPill(c.author),anchor,control(featureIconButton('×',`Remove review comment ${index+1} at ${c.path}:${c.start}${c.end!==c.start?'–'+c.end:''}`,async()=>{try{await reviewAction(current.record.id,'remove',{commentIds:[c.id]});}catch(error){$('#diff-review-error').textContent=error.message;}},'text-button'),'comment',c.id,'remove'));row.append(head,body(c.text,c.path,'comment',c.id));if(c.freshness&&c.freshness!=='current')row.append(el('small','review-freshness '+c.freshness,c.freshness==='outdated'?'File changed since this comment':'File version not verified'));if(stale&&stale.fingerprint!==c.fingerprint)row.append(reviewSourceExcerpt(c.excerpt,c.path));list.append(row);}
  const active=['requested','reviewing'].includes(current.draft.run?.state);$('#diff-review-new').disabled=current.pending||current.reviewStarting||active;$('#diff-review-new').title=active?'Finish or cancel the current review before starting another.':'Archive this review and start a fresh one';$('#diff-review-agent').disabled=current.reviewStarting||active;$('#diff-review-progress').textContent=reviewRunLabel(current.draft);$('#diff-review-progress').title='Review of this conversation across repositories';$('#diff-review-clear-progress').hidden=!active;
  $('#diff-review-add').textContent=current.draft.comments.length||current.draft.note?'Comment':'Start review';
  $('#diff-review-drawer-toggle').textContent=(current.draft.run?.state==='complete'&&!pendingComments&&!unsent&&!pendingNote?'Review done':'Review comments')+(current.draft.comments.length?' · '+current.draft.comments.length:unsent?' · '+unsent+' assessments':current.draft.assessments?.length?' · assessments':current.draft.note?.trim()?' · draft':'');$('#diff-review-send').textContent='Send comments';$('#diff-review-send').disabled=current.pending||current.reviewStarting||(!pendingComments&&!unsent&&!pendingNote?.trim());paintReviewDispatch(current);
  if(focus&&reviewState===current&&dialog.open&&dialog.isConnected&&(document.activeElement===focus.node||document.activeElement===document.body||!document.activeElement?.isConnected)&&![...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog&&!open.contains(dialog))){
    const visible=node=>{
      if(!node?.isConnected||node.disabled||node.closest('[hidden]'))return false;
      for(let child=node,parent=node.parentElement;parent;child=parent,parent=parent.parentElement)if(parent.tagName==='DETAILS'&&!parent.open&&child!==[...parent.children].find(entry=>entry.tagName==='SUMMARY'))return false;
      return node.getClientRects().length>0;
    };
    const rows=[...(focus.kind==='comment'?list:assessments).children],row=rows.find(node=>node.dataset.reviewId===focus.id)||rows[Math.min(focus.index,rows.length-1)];
    const candidates=controls.filter(item=>item.kind===focus.kind&&item.id===row?.dataset.reviewId);
    const target=candidates.find(item=>item.role===focus.control&&visible(item.node))?.node||candidates.find(item=>visible(item.node))?.node||$('#diff-review-drawer-toggle');
    if(visible(target))target.focus({preventScroll:true});
  }
}
async function saveReviewNote(current){const input=$('#diff-review-note'),previous=current.noteBase??current.draft.note??'';const note=input.value;if(note===previous){current.noteDirty=false;return;}await reviewAction(current.record.id,'note',{note,previousNote:previous});current.noteBase=current.draft.note;current.noteDirty=input.value!==note;rememberReviewUI();}
$('#diff-review-add').onclick=async()=>{const current=reviewState,{range,file}=current,text=$('#diff-review-comment').value.trim();if(!range||!file||!text)return;const button=$('#diff-review-add');button.disabled=true;try{await reviewAction(current.record.id,'append',{comments:[{id:crypto.randomUUID(),author:'You',path:file.path,root:file.root,...range,text,baseCommit:file.baseCommit,fingerprint:file.fingerprint}]});window.truffleGuidance?.complete('review.comment-selection');if(reviewState===current){$('#diff-review-comment').value='';hideReviewComposer();}}catch(error){$('#diff-review-error').textContent=error.message;}finally{button.disabled=false;}};
function reviewCommentKeydown(event){
  if((typeof setting==='function'&&!setting('enterToComment',true))||event.key!=='Enter'||event.shiftKey||event.altKey||event.ctrlKey||event.metaKey||event.isComposing||event.keyCode===229)return;
  event.preventDefault();event.stopPropagation();
  if(!event.repeat)$('#diff-review-add').click();
}
$('#diff-review-comment').addEventListener('keydown',reviewCommentKeydown);
$('#diff-review-body').addEventListener('pointerup',event=>{if(reviewState)reviewState.selectionPoint={x:event.clientX,y:event.clientY};},true);
window.addEventListener('resize',()=>{if(reviewState?.quick)positionQuickReview(reviewState);});
$('#diff-review-cancel').onclick=()=>{hideReviewComposer();$('#diff-review-comment').value='';};
function showReviewHistoryEntry(entry){
  const content=$('#diff-review-history-content');content.replaceChildren();
  content.append(featureButton('All previous reviews',()=>paintReviewHistoryList(reviewState.history),'text-button'));
  content.append(el('small','review-history-label','Saved '+new Date(entry.archivedAt*1000).toLocaleString()+' · Read only'));
  const draft=entry.review;
  if(draft.note?.trim()){content.append(el('strong','','Overall review'),reviewCommentBody(draft.note));}
  for(const assessment of draft.assessments||[]){
    const section=el('article','review-assessment');section.append(reviewAuthorPill(assessment.author),el('small','review-history-meta',assessment.pass||''),reviewCommentBody(assessment.text));content.append(section);
  }
  for(const comment of draft.comments||[]){
    const section=el('article','review-comment'),head=el('div','review-comment-head');
    head.append(reviewAuthorPill(comment.author),el('span','review-comment-anchor',`${comment.path} · ${comment.side==='deletions'?'old':'new'} ${comment.start}${comment.end!==comment.start?'–'+comment.end:''}`));
    section.append(head,reviewCommentBody(comment.text,comment.path),el('small','review-history-meta',[comment.root,comment.baseCommit?'Base '+comment.baseCommit:''].filter(Boolean).join(' · ')));if(comment.excerpt)section.append(reviewSourceExcerpt(comment.excerpt,comment.path,'Commented file version'));content.append(section);
  }
  if(!draft.note?.trim()&&!draft.comments?.length&&!draft.assessments?.length)content.append(el('p','form-hint','This review had no saved comments.'));
}
function paintReviewHistoryList(reviews){
  const content=$('#diff-review-history-content');content.replaceChildren();
  if(!reviews.length){content.append(el('p','form-hint','No previous reviews yet. New review archives the current review here.'));return;}
  for(const entry of reviews){
    const button=featureButton('',()=>showReviewHistoryEntry(entry),'review-history-choice'),draft=entry.review;
    button.append(el('span','',new Date(entry.archivedAt*1000).toLocaleString()),el('small','',`${draft.comments?.length||0} comments · ${draft.assessments?.length||0} assessments`));content.append(button);
  }
}
async function loadReviewHistory(){
  const current=reviewState;if(!current||!$('#diff-review-history').open)return;
  $('#diff-review-drawer').open=false;
  const content=$('#diff-review-history-content');content.replaceChildren(el('p','form-hint','Loading previous reviews…'));
  try{
    const result=await api('/api/session/review-draft',{id:current.record.id,action:'history'});
    if(reviewState!==current||!$('#diff-review-history').open)return;
    current.history=result.reviews||[];paintReviewHistoryList(current.history);
  }catch(error){if(reviewState===current)content.replaceChildren(el('p','form-hint',error.message));}
}
$('#diff-review-history').addEventListener('toggle',()=>{if($('#diff-review-history').open)void loadReviewHistory();});
$('#diff-review-history-current').onclick=()=>{$('#diff-review-history').open=false;$('#diff-review-drawer').open=true;$('#diff-review-drawer-toggle').focus();};
$('#diff-review-drawer').addEventListener('toggle',()=>{if($('#diff-review-drawer').open)$('#diff-review-history').open=false;});
async function startNewReview(){
  const current=reviewState;if(!current||current.pending||current.reviewStarting||['requested','reviewing'].includes(current.draft.run?.state))return;
  if($('#diff-review-comment').value.trim()){$('#diff-review-error').textContent='Add or cancel your draft comment before starting a new review.';return;}
  current.pending=true;paintReviewComments();
  try{
    await saveReviewNote(current);
    await reviewAction(current.record.id,'new',{revision:current.draft.revision});
    if(reviewState!==current)return;
    current.noteDirty=false;current.history=null;$('#diff-review-history').open=false;current.noteBase=current.draft.note||'';$('#diff-review-note').value=current.noteBase;$('#diff-review-comment').value='';$('#diff-review-error').textContent='';hideReviewComposer();updateReviewAnnotations();rememberReviewUI();
    toast('New review started. The previous review was archived.');
  }catch(error){if(reviewState===current)$('#diff-review-error').textContent=error.message;}
  finally{current.pending=false;if(reviewState===current)paintReviewComments();}
}
$('#diff-review-new').onclick=startNewReview;
$('#diff-review-agent').onclick=startAgentReview;
$('#diff-review-clear-progress').onclick=async()=>{try{await reviewAction(reviewState.record.id,'status',{requestId:reviewState.draft.run.id,state:'cancelled'});}catch(error){$('#diff-review-error').textContent=error.message;}};
$('#diff-review-drawer-close').onclick=()=>{$('#diff-review-drawer').open=false;$('#diff-review-drawer-toggle').focus();};
$('#diff-review-note').oninput=()=>{if(!reviewState)return;reviewState.noteDirty=true;scheduleReviewUISave();$('#diff-review-send').disabled=reviewState.pending||(!pendingReviewComments(reviewState.draft).length&&!pendingReviewAssessments(reviewState.draft).length&&!($('#diff-review-note').value!==reviewState.draft.reviewedNote&&$('#diff-review-note').value.trim()));};
$('#diff-review-note').onfocus=()=>{if(reviewState&&!reviewState.noteDirty)reviewState.noteBase=reviewState.draft.note||'';};
$('#diff-review-note').onchange=()=>{void saveReviewNote(reviewState).catch(error=>{$('#diff-review-error').textContent=error.message;});};
$('#diff-review-send').onclick=sendReviewFeedback;
$('#diff-review-dialog').addEventListener('close',()=>{if(!$('#diff-review-dialog').open){rememberReviewUI();disposeReview();reviewState=null;render();restoreDialogReturn($('#diff-review-dialog'));}});
$('#diff-review-dialog').dismissLayer=()=>{if(!$('#diff-review-composer').hidden){hideReviewComposer();return true;}if($('#diff-review-history').open){$('#diff-review-history').open=false;return true;}if($('#diff-review-drawer').open){$('#diff-review-drawer').open=false;$('#diff-review-drawer-toggle').focus();return true;}return false;};
$('#diff-mode-unified').onclick=()=>setReviewMode('unified');$('#diff-mode-split').onclick=()=>setReviewMode('split');
$('#diff-review-comment').addEventListener('input',scheduleReviewUISave);
$('#diff-review-dialog').addEventListener('toggle',scheduleReviewUISave,true);
$('#diff-review-body').addEventListener('scroll',()=>{scheduleReviewUISave();scheduleReviewFileHighlight();},{passive:true});
$('#diff-review-dialog').addEventListener('pointerdown',rememberReviewUI,true);
$('#diff-review-dialog').addEventListener('cancel',rememberReviewUI);
window.addEventListener('beforeunload',rememberReviewUI);

