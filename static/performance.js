/* Opt-in bounded local timings. No titles, paths, payloads, or tokens. */
'use strict';
let profiling=false, profileStarted=0, profileBuild=null;
const profileSamples=new Map();
function profileRecord(name,duration){
  if(!profiling||!Number.isFinite(duration))return;
  const values=profileSamples.get(name)||[];values.push(duration);
  if(values.length>120)values.shift();profileSamples.set(name,values);
}
function profileSummary(){
  return [...profileSamples].map(([name,values])=>{
    const sorted=[...values].sort((a,b)=>a-b);
    return {name,samples:values.length,medianMs:+sorted[Math.floor(sorted.length/2)].toFixed(2),p95Ms:+sorted[Math.min(sorted.length-1,Math.ceil(sorted.length*.95)-1)].toFixed(2),maxMs:+sorted.at(-1).toFixed(2)};
  });
}
function profileWrap(name,fn){return function(...args){
  if(!profiling)return fn.apply(this,args);
  const start=performance.now();try{return fn.apply(this,args);}finally{profileRecord(name,performance.now()-start);}
};}
let profileRenderSequence=0;
registerRenderHook('performance:start',context=>{
  if(!profiling)return;
  context.profileStart=performance.now();context.profileMark='mydash-render-'+(++profileRenderSequence);performance.mark(context.profileMark);
},{phase:'before',order:-1000});
registerRenderHook('performance:finish',context=>{
  if(context.profileStart===undefined)return;
  profileRecord('render:'+context.source,performance.now()-context.profileStart);
  performance.clearMeasures('mydash:render');performance.measure('mydash:render',context.profileMark);performance.clearMarks(context.profileMark);
},{phase:'finally'});
allItems=profileWrap('projection:allItems',allItems);
buildBundleIndex=profileWrap('projection:associations',buildBundleIndex);
renderRow=profileWrap('DOM:work row',renderRow);
const unprofiledApi=api;
api=async function(path,payload){
  if(!profiling)return unprofiledApi(path,payload);
  const start=performance.now();try{return await unprofiledApi(path,payload);}finally{profileRecord('request:'+path,performance.now()-start);}
};
for(const type of ['click','input','change'])document.addEventListener(type,event=>{
  if(!profiling||document.hidden||event.target.closest?.('.performance-panel'))return;
  const start=performance.now(),started=profileStarted;
  // Two animation frames approximate presentation; this is not INP or pure CPU time.
  requestAnimationFrame(()=>requestAnimationFrame(()=>{
    if(profiling&&started===profileStarted)profileRecord(type+':next frame',performance.now()-start);
  }));
},true);
function renderPerformancePanel(){
  const panel=el('details','performance-panel');panel.open=profiling;
  panel.append(el('summary','','UI performance · local diagnostics'));
  panel.append(el('p','form-hint','Start recording, use the slow view or search, then return here. Timings stay in memory; up to 120 samples per category. Next-frame timings include presentation delay and are approximate.'));
  const actions=el('div','performance-actions');
  actions.append(featureButton(profiling?'Stop recording':'Start recording',async()=>{
    profiling=!profiling;
    if(profiling){profileSamples.clear();profileStarted=performance.now();if(serviceClient.isDesktop)try{profileBuild=await window.__TAURI__.core.invoke('desktop_info');}catch(_){} }
    render();
  },'button'),featureButton('Refresh report',()=>render(),'text-button'));
  panel.append(actions);
  const report={host:serviceClient.isDesktop?'Tauri/WKWebView':'Browser',build:profileBuild,recording:profiling,seconds:profileStarted?+((performance.now()-profileStarted)/1000).toFixed(1):0,items:state?.items?.length||0,renderedWorkRows:document.querySelectorAll('#work-list .row').length,timings:profileSummary()};
  panel.append(el('p','form-hint',`${report.host}${profileBuild?' · '+profileBuild.version+' '+profileBuild.profile:''} · ${report.items} cached items · ${report.seconds}s recorded`));
  const table=el('table','performance-table'),head=el('thead'),header=el('tr');
  for(const label of ['Operation','Samples','Median ms','p95 ms','Max ms'])header.append(el('th','',label));
  head.append(header);table.append(head);const body=el('tbody');
  for(const timing of report.timings){const row=el('tr');for(const value of [timing.name,timing.samples,timing.medianMs,timing.p95Ms,timing.maxMs])row.append(el('td','',String(value)));body.append(row);}
  table.append(body);panel.append(table);
  const raw=el('details'),summary=el('summary','','Raw timing report');raw.append(summary,el('pre','performance-report',JSON.stringify(report,null,2)));panel.append(raw);return panel;
}
