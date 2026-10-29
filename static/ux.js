'use strict';
const agentContextReveals=new Set();
function snoozeDue(entry){return Number.isFinite(entry.until)&&entry.until<=Date.now();}
function nextSnoozeTime(choice){
  const date=new Date();date.setHours(9,0,0,0);
  if(choice==='tomorrow')date.setDate(date.getDate()+1);
  else if(choice==='monday')date.setDate(date.getDate()+((8-date.getDay())%7||7));
  else if(choice==='three')date.setDate(date.getDate()+3);
  else return null;
  return date.getTime();
}
function snoozeScheduleControl(key,entry){
  const row=el('div','snooze-schedule'),select=el('select');select.setAttribute('aria-label','Return date for '+entry.title);
  select.add(new Option(entry.until?'Return '+new Date(entry.until).toLocaleString():'Until restored','current'));
  for(const [value,label]of [['none','Until restored'],['tomorrow','Tomorrow'],['three','In 3 days'],['monday','Monday'],['custom','Choose date…']])select.add(new Option(label,value));
  const input=el('input');input.type='datetime-local';input.hidden=true;input.setAttribute('aria-label','Custom return date');
  const save=until=>{if(until!==null&&(!Number.isFinite(until)||until<=Date.now())){toast('Choose a future return date.');return;}entry.until=until;persist();liveBarSignature='';renderLiveBar();};
  select.onchange=()=>{if(select.value==='custom'){input.hidden=false;input.focus();return;}save(nextSnoozeTime(select.value));};input.onchange=()=>save(new Date(input.value).getTime());row.append(select,input);return row;
}
function detachLinkButton(record,field,key){
  const button=featureButton('×',async event=>{
    button.disabled=true;
    try{
      const before=(await api('/api/session/links',{id:record.id})).context;
      const identity=value=>field==='resources'?value.resourceKey:field==='worktrees'?value.path:value;
      const previous=before[field],next=previous.filter(value=>identity(value)!==key);
      const receipt=await api('/api/session/links',{id:record.id,context:{revision:before.revision,[field]:next}});
      const refreshLinks=async()=>{
        const results=await Promise.allSettled([loadStateSnapshot({attempts:2}),field==='artifactIds'?loadArtifactLibrary(true):Promise.resolve({accepted:true})]);
        if(results[0].status==='fulfilled'&&results[0].value.accepted)render();
        if(results.some(result=>result.status==='rejected'))throw new Error('Dashboard refresh failed.');
        return results.every(result=>result.value.accepted);
      };
      let undoing=false,undone=false;
      const undo=async()=>{
        if(undoing||undone)return;undoing=true;
        try{
          await api('/api/session/links',{id:record.id,context:{revision:receipt.revision,[field]:previous,...(field==='itemIds'?{worktrees:before.worktrees}:{})}});
          undone=true;
          const refreshed=await refreshLinks();
          if(!refreshed) {toast('Link restored. Newer dashboard changes were kept; refresh to see the result.');return false;}
        }catch(error){toast(undone?'Link restored; dashboard refresh failed. Do not repeat Undo.':error.status===409?'This chat’s links changed after removal. Undo was not applied; newer links were kept.':error.message);return false;}
        finally{undoing=false;}
      };
      let message='Dashboard link removed';
      try{if(!await refreshLinks())message+='; refresh to update this view';}
      catch(error){message+='; dashboard refresh failed';}
      toast(message,undo);
    }catch(error){toast(error.message);button.disabled=false;}
  },'detach-link');
  button.title='Remove dashboard link';button.setAttribute('aria-label','Remove '+(field==='worktrees'?key.split('/').pop():field==='resources'?'resource':field==='itemIds'?'work item':'artifact')+' link');
  button.addEventListener('click',event=>{event.stopPropagation();event.preventDefault();});return button;
}
function toggleSidebar(side){
  if(side==='left')leftSidebarCollapsed=!leftSidebarCollapsed;
  else agentSidebarCollapsed=!agentSidebarCollapsed;
  persist();render();requestAnimationFrame(()=>{for(const view of terminalViews.values())view.resize();});
}
function positionAgentMenu(menu){const rect=menu.getBoundingClientRect(),options=menu.querySelector('.agent-menu-options');options.style.left=Math.max(8,Math.min(window.innerWidth-175,rect.right-165))+'px';}
let activityView=['sessions','changes','logs'].includes(source)?source:'sessions';
$('#activity-open').onclick=()=>{source=activityView;persist();render();};
function renderSharedUX(){
  syncActiveDiff();
  void syncActiveChatCI();
  document.body.classList.toggle('left-sidebar-collapsed',leftSidebarCollapsed);
  for(const menu of document.querySelectorAll('.agent-tab .agent-menu'))menu.ontoggle=()=>positionAgentMenu(menu);
  $('.launcher').hidden=false;
  const active=['sessions','changes','logs'].includes(source),tabs=$('#activity-tabs');tabs.hidden=!active;
  $('#activity-open').classList.toggle('active',active);
  if(active){activityView=source;tabs.replaceChildren();for(const [view,label]of [['sessions','History'],['changes','Changes'],['logs','Logs']]){const button=featureButton(label,()=>{source=view;persist();render();},'text-button');button.setAttribute('aria-pressed',String(view===source));tabs.append(button);}}
}
registerRenderHook('shared-ux',renderSharedUX,{order:40});
let snoozeDueSignature='';
setInterval(()=>{const signature=JSON.stringify(Object.entries(snoozedAgentTabs).map(([key,value])=>[key,snoozeDue(value)]));if(signature!==snoozeDueSignature){snoozeDueSignature=signature;liveBarSignature='';renderLiveBar();}},30000);
document.addEventListener('click',event=>{for(const menu of document.querySelectorAll('.agent-menu[open]'))if(!menu.contains(event.target))menu.open=false;});
document.addEventListener('keydown',event=>{if(event.key==='Escape')for(const menu of document.querySelectorAll('.agent-menu[open]'))menu.open=false;});

function paintReviewDraftBadges(){
 for(const button of document.querySelectorAll('[data-review-for]')){const draft=reviewDrafts()[button.dataset.reviewFor],count=draft?.comments?.length||0,active=['requested','reviewing'].includes(draft?.run?.state);button.textContent='Review';button.title=reviewRunLabel(draft)||'Review changes';}
 for(const tab of document.querySelectorAll('[data-review-agent]')){const draft=reviewDrafts()[tab.dataset.reviewAgent],active=['requested','reviewing'].includes(draft?.run?.state);let badge=tab.querySelector('.agent-review-indicator');if(active){if(!badge){badge=el('small','review-run-status agent-review-indicator');tab.querySelector('.live-agent-state').append(badge);}badge.textContent=reviewRunLabel(draft);}else badge?.remove();}
}
window.addEventListener('truffle-review-draft',()=>{paintReviewDraftBadges();for(const row of document.querySelectorAll('[data-review-file]'))paintReviewFileMarkers(row,reviewDrafts()[row.dataset.reviewRecord]);});

function sidebarGlyph(name){
 const paths={agents:'M4 4h16v16H4z m3 4 4 4-4 4 m7 0h3',skills:'M12 8v4l3 2 M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',all:'M3 3h18v18H3z M3 9h18 M3 15h18 M9 3v18 M15 3v18',linear:'M4 4h16v16H4z M4 20 20 4',github:'M6 3v12a3 3 0 1 0 3 3H6 M18 3v4c0 4-9 2-9 6 M8 3a2 2 0 1 1-4 0 2 2 0 0 1 4 0 M20 3a2 2 0 1 1-4 0 2 2 0 0 1 4 0',sentry:'m12 3 10 18H2L12 3Zm0 6v5 M12 17v.1',notes:'M4 3h16v18H4z M7 7h10 M7 11h10 M7 15h7',activity:'M12 8v4l3 2 M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',commands:'m5 8 4 4-4 4 M13 16h6',refresh:'M20 7v5h-5 M20 12a8 8 0 1 0-2 5',settings:'M9 3h6l1 4 4 2v6l-4 2-1 4H9l-1-4-4-2V9l4-2 1-4Z M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',restart:'M12 3v9 M7 5a8 8 0 1 0 10 0',cloud:'M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-1 5 5 0 0 1 0 11 M12 21V11m-3 3 3-3 3 3'};
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');for(const [key,value]of Object.entries({viewBox:'0 0 24 24',fill:'none',stroke:'currentColor','stroke-width':'1.6','stroke-linecap':'round','stroke-linejoin':'round','aria-hidden':'true'}))svg.setAttribute(key,value);const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d',paths[name]);svg.append(path);return svg;
}
for(const button of document.querySelectorAll('#sources .primary-nav>.nav-item[data-source]')){const name=button.dataset.source;if(!['agents','skills','all','linear','github','sentry','notes'].includes(name))continue;const icon=button.querySelector('span');icon.className='sidebar-icon';icon.replaceChildren(sidebarGlyph(name));}
for(const [id,name,hasIcon]of [['activity-open','activity',true],['command-open','commands',false],['force-refresh','refresh',false],['gcp-login','cloud',true],['restart-toggle','restart',true],['settings','settings',false]]){
 const button=$('#'+id);let icon;if(hasIcon)icon=button.querySelector('span');else{if(button.firstChild?.nodeType===3)button.firstChild.remove();icon=el('span');button.prepend(icon);}icon.className='sidebar-icon';icon.replaceChildren(sidebarGlyph(name));
}
