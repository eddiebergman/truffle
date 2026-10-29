'use strict';
function resourceTime(value){if(typeof value==='number'&&Number.isFinite(value)&&value>0)return value*1000;if(typeof value==='string'&&/^\d{4}-\d\d-\d\dT/.test(value)){const time=Date.parse(value);if(Number.isFinite(time))return time;}return null;}
function resourceLinks(resource){const result=[];for(const [category,values]of Object.entries(resource.uris||{})){for(const entry of Array.isArray(values)?values:[values]){const value=typeof entry==='string'?entry:entry?.uri;if(typeof value==='string'&&value)result.push({category,value,title:entry?.title||category});}}return result;}
function resourceIsPast(resource){return ['stopped','deleted','terminated','closed','completed','done','cancelled','expired'].includes(String(resource.state).toLowerCase());}
function resourceMayCost(resource){return !resourceIsPast(resource)&&(/(?:gpu|virtual.machine|compute|instance|dev.env|cluster|node|vm)/i.test(resource.kind||'')||resource.cost?.billable===true);}
function resourceCheckPrompt(resource){return 'Check this existing resource and report its current state to truffle with an observation timestamp: '+JSON.stringify(resource)+'. Inspect only. Do not create, stop, delete or change resources. Include expiry and cleanup instructions if known.';}
if(typeof module!=='undefined')module.exports={resourceTime,resourceLinks,resourceIsPast,resourceMayCost,resourceCheckPrompt};
function resourceSection(record,expanded=new Set()){
  const section=el('section','agent-resource-section'),head=el('div','section-heading'),rows=record.resources||[];
  head.append(el('h3','','Resources'),el('small','',rows.filter(r=>!resourceIsPast(r)).length||''));section.append(head);
  if(!rows.length){section.append(el('p','resource-empty','Agent-reported VMs, GPUs and environments appear here.'));return section;}
  const recent=rows.filter(r=>!resourceIsPast(r)).sort((a,b)=>Number(resourceMayCost(b))-Number(resourceMayCost(a))),past=rows.filter(resourceIsPast);
  function row(resource){
    const details=el('details','resource-entry');details.dataset.resourceKey=resource.resourceKey;details.open=expanded.has(resource.resourceKey);
    const summary=el('summary'),name=el('strong','',resource.name||resource.id),status=el('span','resource-state',resource.state||'unknown');
    name.title=resource.id;status.title=resource.state;summary.append(name,status,detachLinkButton(record,'resources',resource.resourceKey));details.append(summary);
    details.append(el('small','resource-kind',(resource.kind||'resource').replace(/[_-]/g,' ')+' · '+resource.provider));
    if(resource.purpose)details.append(el('p','',resource.purpose));
    const observed=resourceTime(resource.observedAt||resource.checked_at),reported=resourceTime(resource.reportedAt);
    const known=resource.verification==='checked'||resource.state_source==='checked-now';
    const time=observed||reported;const stale=!time||Date.now()-time>15*60*1000;
    if(!observed){const badge=el('small','context-stale','Unverified');badge.title='No observation timestamp reported';summary.append(badge);}
    else if(stale){const age=el('small','context-stale',relative(time/1000));age.title=(known&&observed?'Last checked ':'Last reported ')+new Date(time).toLocaleString();summary.append(age);}
    const note=el('small','resource-observed',(known&&observed?'Checked ':'Reported ')+(time?new Date(time).toLocaleString():'at unknown time')+(stale?' · stale':''));details.append(note);
    if(resourceMayCost(resource)){const cost=el('small','context-stale','May incur cost');cost.title='Active compute reported by an agent. Billing and current state are not verified.';details.append(cost);const hint=el('small','resource-cost','$?');hint.title=cost.title;summary.append(hint);}
    const actions=el('div','resource-actions');actions.append(featureButton('Ask to check',async()=>{try{await loadInventory();if(candidateForRecord(record)?.host!=='embedded')throw new Error('Resume this chat before asking it to check a resource.');await api('/api/terminal/input',{id:record.id,data:'\x1b[200~'+resourceCheckPrompt(resource)+'\x1b[201~\r'});toast('Resource check sent to chat.');}catch(error){toast(error.message);}},'text-button'));details.append(actions);
    const expires=resourceTime(resource.expiresAt);
    if(expires){const expiry=el('small','resource-expiry',(expires<Date.now()?'Deadline passed · ':'Expires ')+new Date(expires).toLocaleString());expiry.classList.toggle('due',expires-Date.now()<3600000);details.append(expiry);}
    else if(resource.expiry)details.append(el('small','resource-expiry','Expiry: '+resource.expiry));
    const cleanup=resource.cleanup||resource.cleanup_responsibility;if(cleanup){details.append(el('p','resource-cleanup','Cleanup: '+cleanup));actions.append(featureButton('Copy cleanup',async()=>{await serviceClient.copyText(cleanup);toast('Copied cleanup instructions.');},'text-button'));}
    for(const ref of resourceLinks(resource)){
      let url=null;try{url=new URL(ref.value);}catch(_){}
      if(url&&['https:','http:'].includes(url.protocol)&&!url.username&&!url.password){const a=el('a','resource-link',ref.title);a.href=url.href;a.target='_blank';a.rel='noopener noreferrer';a.title=ref.value;details.append(a);}
      else{const copy=featureButton(ref.category+' ⧉',async()=>{try{await serviceClient.copyText(ref.value);toast('Copied resource reference.');}catch(_){toast(ref.value);}},'text-button resource-link');copy.title=ref.value;details.append(copy);}
    }
    const rawLinks=resource.workUrls||resource.work_links||resource.links||[];
    const workLinks=Array.isArray(rawLinks)?rawLinks:typeof rawLinks==='string'?[rawLinks]:[];
    for(const value of workLinks){try{const url=new URL(value);if(!['https:','http:'].includes(url.protocol)||url.username||url.password)continue;const a=el('a','resource-link',url.hostname==='github.com'?url.pathname.split('/').slice(-3).join(' '):url.pathname.match(/[A-Z]+-\d+/)?.[0]||'Related work');a.href=url.href;a.target='_blank';a.rel='noopener noreferrer';details.append(a);}catch(_){}}
    const metadata=el('details','resource-metadata'),label=el('summary','','Details'),pre=el('pre','',JSON.stringify(resource,null,2));metadata.append(label,pre);details.append(metadata);return details;
  }
  for(const resource of recent)section.append(row(resource));
  if(past.length){const history=el('details','resource-history');history.append(el('summary','','Past resources · '+past.length));for(const resource of past)history.append(row(resource));section.append(history);}
  return section;
}
