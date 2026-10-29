'use strict';
let gitDiffSelections=prefs.gitDiffSelections||{};
function gitDiffDirectories(record){const paths=new Map();if(record.cwd)paths.set(record.cwd,{path:record.cwd,branch:record.cwd.split('/').pop()});for(const tree of record.worktrees||[])paths.set(tree.path,tree);return [...paths.values()];}
function selectedGitDirectory(record){const paths=gitDiffDirectories(record);return paths.some(t=>t.path===gitDiffSelections[record.id])?gitDiffSelections[record.id]:record.cwd;}
function gitDirectorySelector(record,directory,onChange){const select=el('select','git-worktree-select');select.setAttribute('aria-label','Diff worktree');for(const tree of gitDiffDirectories(record)){const refs=(tree.itemIds||[]).map(id=>(record.items||[]).find(i=>i.id===id)).filter(Boolean).map(workReference).filter(Boolean).join(', ');select.add(new Option((tree.branch||tree.path.split('/').pop())+(refs?' · '+refs:''),tree.path));}select.value=directory;select.title=directory;select.onchange=()=>{gitDiffSelections[record.id]=select.value;persist();onChange(select.value);};return select;}
const gitDiffViews=new Map();let activeGitDiffKey='';
function gitReviewFocusKey(record,directory,path){return JSON.stringify(['review',record.id,directory,path??null]);}
function gitFileTree(files){
  const root={folders:new Map(),files:[]};
  for(const file of files){const parts=file.path.split('/');let node=root;for(const folder of parts.slice(0,-1)){if(!node.folders.has(folder))node.folders.set(folder,{folders:new Map(),files:[]});node=node.folders.get(folder);}node.files.push(file);}
  return root;
}
function appendGitFileTree(container,files,makeRow,collapsed=new Set()){
  function append(parent,node,depth,prefix){
    for(const [first,child]of [...node.folders].sort(([a],[b])=>a.localeCompare(b))){
      let name=first,folder=child;while(!folder.files.length&&folder.folders.size===1){const [part,next]=folder.folders.entries().next().value;name+='/'+part;folder=next;}
      const path=prefix+name,details=el('details','git-tree-folder'),summary=el('summary','',name);details.open=!collapsed.has(path);details.dataset.folder=path;summary.title=path;summary.style.paddingLeft=(depth*7+4)+'px';details.append(summary);details.ontoggle=()=>{if(details.open)collapsed.delete(path);else collapsed.add(path);};append(details,folder,depth+1,path+'/');parent.append(details);
    }
    for(const file of node.files.sort((a,b)=>a.path.localeCompare(b.path))){const row=makeRow(file);row.style.paddingLeft=(depth*7+15)+'px';parent.append(row);}
  }
  append(container,gitFileTree(files),0,'');
}
function gitChangeCounts(additions,deletions){
  const counts=el('span','git-change-counts');counts.setAttribute('aria-label',additions==null?'Binary file':additions+' added, '+deletions+' removed');
  counts.append(el('span','git-added',additions==null?'—':String(additions)),el('span','git-count-separator',additions==null?'':'/'),el('span','git-deleted',deletions==null?'':String(deletions)));
  return counts;
}
function paintReviewFileMarkers(row,draft){
  const counts={you:0,agent:0};
  for(const comment of draft?.comments||[])if(comment.path===row.dataset.reviewFile&&comment.root===row.dataset.reviewRoot)counts[(comment.author||'You')==='You'?'you':'agent']++;
  let markers=row.querySelector('.review-file-counts');if(!markers){markers=el('span','review-file-counts');row.insertBefore(markers,row.querySelector('.git-change-counts'));}
  markers.replaceChildren();row.classList.toggle('has-review-comments',!!(counts.you+counts.agent));
  for(const [kind,label] of [['you','Your comments'],['agent','Agent comments']])if(counts[kind]){const marker=el('small','review-file-count '+kind);marker.innerHTML='<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M4 4h16v12H9l-5 4V4Z"/></svg>'+counts[kind];marker.title=label+': '+counts[kind];marker.setAttribute('aria-label',marker.title);markers.append(marker);}
}
function paintGitDiff(body,view){
  const focused=document.activeElement,focusKey=body.contains(focused)?focused?.dataset.focusKey:null;
  const scroll=$('.git-file-list',body)?.scrollTop??view.scrollTop??0;
  body.replaceChildren();
  if(gitDiffDirectories(view.record).length)body.append(gitDirectorySelector(view.record,view.directory,()=>render()));
  if(view.data){
    const data=view.data,totals=$('.git-header-summary',body.closest('.agent-git-section'));
    if(totals){totals.replaceChildren(gitChangeCounts(data.additions,data.deletions));totals.title='Changes against '+data.baseRef+' · '+data.root;}
    const files=el('div','git-file-list');files.onscroll=()=>{view.scrollTop=files.scrollTop;};
    view.collapsedFolders||=new Set();appendGitFileTree(files,data.files,file=>{const row=featureButton('',event=>openDiffReview(view.record,file.path,view.directory,event.currentTarget),'git-file-row'),name=el('span','git-file-path',file.path.split('/').pop());row.dataset.focusKey=gitReviewFocusKey(view.record,view.directory,file.path);row.onpointerenter=()=>{void cachedReviewFile(view.record,file.path,view.directory).catch(()=>{});};row.dataset.reviewFile=file.path;row.dataset.reviewRoot=data.root;row.dataset.reviewRecord=view.record.id;row.title=file.path+(file.untracked?' · untracked':'');row.append(name,gitChangeCounts(file.additions,file.deletions));paintReviewFileMarkers(row,reviewDrafts()[view.record.id]);return row;},view.collapsedFolders);
    body.append(files);requestAnimationFrame(()=>{if(files.isConnected)files.scrollTop=scroll;});if(!data.files.length)body.append(el('div','git-diff-note','No changes against main.'));if(data.truncated)body.append(el('div','git-diff-note','More than 500 files. Showing a limited scan.'));
  }
  if(view.error)body.append(el('div','git-diff-note',view.error));else if(!view.data)body.append(el('div','git-diff-note','Loading diff…'));
  if(focusKey&&body.isConnected&&source==='agents'&&activeGitDiffKey===view.record.id+'\0'+view.directory&&!document.querySelector('dialog[open]')&&(document.activeElement===focused||document.activeElement===document.body||!document.activeElement?.isConnected)){
    const candidate=liveCandidates().find(c=>candidateKey(c)===selectedAgentKey),selected=selectedAgentKey.startsWith('record:')?state.sessions.find(row=>row.id===selectedAgentKey.slice(7)):candidate&&recordForCandidate(candidate);
    if(selected?.id===view.record.id&&selectedGitDirectory(selected)===view.directory)[...body.querySelectorAll('[data-focus-key]')].find(node=>node.dataset.focusKey===focusKey&&!node.disabled&&!node.matches(':disabled')&&!node.closest('[hidden]')&&node.getClientRects().length>0&&getComputedStyle(node).visibility==='visible')?.focus({preventScroll:true});
  }
}
function agentGitSection(record,directory){
  directory=selectedGitDirectory(record);
  const key=record.id+'\0'+directory;let view=gitDiffViews.get(key);
  if(!view){view={data:null,error:'',at:0,pending:false};gitDiffViews.set(key,view);if(gitDiffViews.size>20)gitDiffViews.delete(gitDiffViews.keys().next().value);}
  view.record=record;view.directory=directory;
  if(activeGitDiffKey!==key){activeGitDiffKey=key;view.at=0;}
  const section=el('section','agent-git-section'),head=el('div','section-heading');head.append(el('h3','','Git diff'));const controls=el('div','git-header-actions');const draft=reviewDrafts()[record.id],count=draft?.comments?.length||0;controls.append(featureButton('Review',event=>openDiffReview(record,undefined,directory,event.currentTarget),'text-button'));controls.firstChild.dataset.reviewFor=record.id;controls.firstChild.dataset.focusKey=gitReviewFocusKey(record,directory);controls.prepend(el('span','git-header-summary'));head.append(controls);
  section.append(head);
  if(contextSectionsCollapsed['git-diff']===undefined)contextSectionsCollapsed['git-diff']=false;
  const result=collapsibleContextSection(section,'git-diff',view.data?.files.length),body=$('.context-section-body',result);
  paintGitDiff(body,view);
  view.paint=()=>{if(!result.isConnected)return;paintGitDiff(body,view);if(view.data){const toggle=$('.work-section-toggle',head);let count=$('small',toggle);if(!count){count=el('small');toggle.append(count);}count.textContent=view.data.files.length;}};
  result.refreshGitDiff=()=>{
    if(source!=='agents'||agentSidebarCollapsed||terminalFocus||body.hidden||document.hidden||view.pending||Date.now()-view.at<10000)return;
    view.pending=true;let changed=false;
    void api('/api/session/git-diff',{id:record.id,cwd:directory}).then(data=>{changed=!!view.error||JSON.stringify(data)!==JSON.stringify(view.data);view.data=data;view.error='';}).catch(error=>{changed=true;view.error=error.message==='Not found.'?'Restart the updated service to load Git diffs.':error.message;}).finally(()=>{view.pending=false;view.at=Date.now();if(changed)view.paint();});
  };
  head.addEventListener('click',()=>queueMicrotask(()=>result.refreshGitDiff()));
  queueMicrotask(()=>result.refreshGitDiff());return result;
}
setInterval(()=>{document.querySelector('#agent-workspace .agent-git-section')?.refreshGitDiff();},1000);
