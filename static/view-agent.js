/* Ask about the actual filtered work set, independent of pagination and DOM layout. */
'use strict';
function workViewRequest(items,metadata,question){
  const itemIds=[...new Set(items.map(item=>item.id))];
  if(!itemIds.length)throw new Error('There are no items in this view.');
  if(itemIds.length>4096)throw new Error('Narrow this view to at most 4,096 items.');
  return {source:metadata.source,itemIds,filters:metadata.filters,question:question.trim()};
}
if(typeof module!=='undefined')module.exports={workViewRequest};
if(typeof document!=='undefined')(()=>{
  const toggle=$('#ask-view'),form=$('#ask-view-form'),question=$('#ask-view-question'),agent=$('#ask-view-agent'),folder=$('#ask-view-folder'),submit=$('#ask-view-start'),error=$('#ask-view-error');
  let open=false,pending=false,frozen=null,prepared=null,automatic='';
  function metadata(){return {source,filters:{query:$('#search').value,status:statusFilter.value,scope:scopePrefs[source]||'',group:$('#group-by').value,...(source==='github'?{githubView}:{}),...(source==='sentry'?{sentryView,sentryFilters}: {})}};}
  function update(){
    const allowed=['all','linear','github','sentry'].includes(source);
    toggle.hidden=!allowed;toggle.disabled=pending||!visibleWorkItems.length;
    toggle.setAttribute('aria-expanded',String(open&&allowed));form.hidden=!open||!allowed;
    if(!allowed)return;
    const manual=$('#launch-folder').value,hints=visibleWorkItems.map(folderHint);
    const repositories=new Set(visibleWorkItems.filter(item=>item.source==='github').map(scopeKey));
    const sameRepository=visibleWorkItems.length&&visibleWorkItems.every(item=>item.source==='github')&&repositories.size===1?[...repositories][0]:null;
    const repositoryFolder=sameRepository?(inventory.folders||[]).find(item=>item.name.toLowerCase()===sameRepository.split('/').pop().toLowerCase())?.path:null;
    automatic=manual||(hints.length&&hints.every(path=>path&&path===hints[0])?hints[0]:repositoryFolder||inventory.workRoot)||'';
    const auto=folder.options[0];if(auto)auto.textContent='Auto · '+(automatic.split('/').pop()||'code');
    $('#ask-view-count').textContent=visibleWorkItems.length+' matching items · snapshot on send';
    question.placeholder='Ask about these items, or describe a task…';
    submit.disabled=pending||!visibleWorkItems.length||!question.value.trim();
  }
  toggle.onclick=()=>{
    if(pending)return;
    open=!open;
    if(open){
      agent.value=$('#quick-agent').value;
      const previous=folder.value;
      folder.replaceChildren(new Option('Automatic',''),...(inventory.directories||[]).map(path=>new Option(path.replace(/^\/Users\/[^/]+/,'~'),path)));
      folder.value=previous;update();question.focus();
    }else update();
  };
  $('#ask-view-close').onclick=()=>{if(!pending){open=false;update();toggle.focus();}};
  form.oninput=()=>{frozen=null;prepared=null;error.hidden=true;update();};
  form.onchange=form.oninput;
  question.onkeydown=event=>{if((event.metaKey||event.ctrlKey)&&event.key==='Enter'){event.preventDefault();if(!submit.disabled)form.requestSubmit();}else if(event.key==='Escape'&&!pending){event.preventDefault();open=false;update();toggle.focus();}};
  form.onsubmit=async event=>{
    event.preventDefault();if(pending||!question.value.trim())return;
    pending=true;error.hidden=true;update();
    try{
      if(!frozen){frozen={...workViewRequest(visibleWorkItems,metadata(),question.value),requestId:crypto.randomUUID(),agent:agent.value,folderPath:folder.value||automatic};
        if(new TextEncoder().encode(JSON.stringify(frozen)).length>120*1024){frozen=null;throw new Error('This view is too large. Narrow the filters and try again.');}}
      if(!prepared){const result=await api('/api/session/from-view',frozen);state=result;prepared=state.sessions.find(s=>s.id===result.savedSessionId);if(!prepared)throw new Error('The saved view session is unavailable.');}
      open=false;update();
      if(state.demo){source='sessions';persist();render();toast('Preview saved with '+frozen.itemIds.length+' items · no agent launched');}
      else await openSavedSession(prepared);
    }catch(problem){open=true;error.textContent=problem.message==='Not found.'?'The running service needs an update for Ask agent. Use Restart → Service; your question is kept here.':problem.message;error.hidden=false;update();}
    finally{pending=false;update();}
  };
  registerRenderHook('view-agent',update,{order:60});
  window.addEventListener('truffle-service-restarted',()=>{if(!prepared)frozen=null;error.hidden=true;update();});
  update();
})();
