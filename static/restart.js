/* Native restart helper: service-only by default; explicit consent before ending terminals. */
'use strict';
function restartImpact(review,mode){
  const count=review?.liveCount||0,interrupts=count>0&&(mode==='terminals'||!review?.preserveTerminals);
  return {interrupts,blocked:interrupts&&!!review?.problems?.length,
    message:interrupts?`${count} terminal${count===1?'':'s'} will close, then saved chats will reopen in the same folders. Running commands and subagents may be interrupted.`:mode==='terminals'?'Restart the terminal host and dashboard service. No live terminals.':count?`${count} live agent${count===1?'':'s'} will keep running while the dashboard service restarts.`:'Restart the dashboard service. No live terminals.'};
}
if(typeof module!=='undefined')module.exports={restartImpact};
if(typeof document!=='undefined')(()=>{
  const toggle=$('#restart-toggle'),panel=$('#restart-panel'),start=$('#restart-start'),check=$('#restart-confirm'),impact=$('#restart-impact'),result=$('#restart-result');
  let review=null,mode='service',job=null,pending=false,timer=null,finished='';
  const demo=()=>!!state?.demo;
  function paint(){
    toggle.disabled=pending;toggle.setAttribute('aria-expanded',String(!panel.hidden));toggle.querySelector('span:last-child').textContent=job?.status==='working'?'Restarting…':'Restart';
    for(const button of panel.querySelectorAll('[data-restart-mode]')){button.setAttribute('aria-pressed',String(button.dataset.restartMode===mode));button.disabled=pending||job?.status==='working';}
    const info=restartImpact(review,mode);impact.textContent=review?info.message:'Checking the service…';
    $('#restart-consent').hidden=!info.interrupts;check.disabled=pending||job?.status==='working';
    $('#restart-problems').textContent=info.blocked?(review.problems||[]).join(' '):'';
    start.textContent=mode==='service'?'Restart service':'Restart & resume';start.disabled=!review||pending||job?.status==='working'||info.blocked||(info.interrupts&&!check.checked);
    if(job&&job.status!=='idle'){result.textContent=[job.message,...(job.errors||[]).filter(message=>message!==job.message)].filter(Boolean).join('\n');result.dataset.status=job.status;result.hidden=false;}
  }
  async function finish(){
    if(job?.status!=='succeeded'||finished===job.id)return;finished=job.id;
    await refresh();await loadInventory();await loadArtifactLibrary(true);window.dispatchEvent(new CustomEvent('mydash-service-restarted'));
    toast(job.message);
  }
  async function poll(){
    clearTimeout(timer);
    if(!serviceClient.isDesktop)return;
    try{job=await serviceClient.restartStatus();paint();if(job.status==='working')timer=setTimeout(poll,750);else await finish();}
    catch(error){result.hidden=false;result.textContent=error.message;timer=setTimeout(poll,2000);}
  }
  async function inspect(){
    pending=true;check.checked=false;result.hidden=true;paint();
    try{
      review=demo()?{liveCount:liveCandidates().length,preserveTerminals:true,problems:[],snapshot:'demo'}:await serviceClient.restartPreview();
      job=review.job||null;
      if(review.job?.status==='working'){job=review.job;void poll();}
    }catch(error){review=null;job={status:'failed',message:String(error.message||error)};}
    finally{pending=false;paint();}
  }
  toggle.onclick=()=>{panel.hidden=!panel.hidden;paint();if(!panel.hidden)void inspect();};
  $('#restart-close').onclick=()=>{panel.hidden=true;toggle.setAttribute('aria-expanded','false');toggle.focus();};
  for(const button of panel.querySelectorAll('[data-restart-mode]'))button.onclick=()=>{mode=button.dataset.restartMode;check.checked=false;paint();};
  check.onchange=paint;
  start.onclick=async()=>{
    if(start.disabled)return;pending=true;paint();
    try{
      job=demo()?{id:'demo',status:'succeeded',message:'Preview only; no service or terminals were restarted.'}:await serviceClient.restartStart({mode,confirmed:check.checked,snapshot:review.snapshot});
      if(!demo())void poll();
    }catch(error){job={status:'failed',message:String(error.message||error)};review=null;}
    finally{pending=false;paint();}
  };
  registerRenderHook('restart:availability',()=>{if(!serviceClient.isDesktop){toggle.disabled=!demo();if(demo())toggle.title='Preview restart controls; no processes will be restarted.';}},{order:20});
  if(!serviceClient.isDesktop&&!demo()){toggle.disabled=true;toggle.title='Restart controls are available in the desktop app.';}else if(serviceClient.isDesktop)void poll();
})();
