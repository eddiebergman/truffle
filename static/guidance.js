'use strict';
function guidanceEligible(progress,id){return progress?.enabled===true&&!['completed','dismissed'].includes(progress.tips?.[id]);}
function guidanceReadyToSnooze(report,startedAt,now){
 if(!report||!['waiting','idle'].includes(report.state)||!['ready','finished'].includes(report.reason)||!Number.isFinite(report.at)||report.at<=0||report.at>now||report.at<(startedAt||0))return false;
 if(report.subagents>0||report.scheduledWakeups>0||Object.values(report.backgroundCounts||{}).some(n=>n>0))return false;
 return now-report.at>=3*60*60;
}
if(typeof module!=='undefined')module.exports={guidanceEligible,guidanceReadyToSnooze};
if(typeof document!=='undefined')(()=>{
 let data=null,active=null,lastInput=Date.now(),epoch='',shownEpoch='',pausedUntil=0,loading=false,write=Promise.resolve();
 const sessionCompleted=new Set();
 async function load(){if(loading)return;loading=true;try{data=await api('/api/guidance');}catch{data=null;}finally{loading=false;}}
 function remove(){if(!active)return;const {anchor,node,highlight,previousDescription}=active;if(previousDescription)anchor.setAttribute('aria-describedby',previousDescription);else anchor.removeAttribute('aria-describedby');const focused=node.contains(document.activeElement);highlight.remove();node.remove();active=null;if(focused&&anchor.isConnected)anchor.focus?.({preventScroll:true});}
 function update(action,id,enabled){
  if(id)sessionCompleted.add(id);if(active?.id===id||action==='enabled'||action==='replay')remove();
  write=write.catch(()=>{}).then(async()=>{for(let attempt=0;attempt<2;attempt++){if(!data)await load();if(!data)return;try{data.progress=await api('/api/guidance',{revision:data.progress.revision,action,...(id?{id}:{}),...(enabled===undefined?{}:{enabled})});return;}catch(error){await load();if(attempt)throw error;}}});
  return write;
 }
 function dismissTip(){if(!active)return false;const id=active.id;remove();void update('dismiss',id).catch(()=>{});return true;}
 window.truffleGuidance={dismiss:dismissTip,complete:id=>{void update('complete',id).catch(()=>{});},refresh:load};
 window.renderGuidanceSettings=async panel=>{
  const section=el('section');panel.append(section);await load();if(!data){section.append(el('p','form-hint','Guidance settings are unavailable. Reopen Help to retry.'));return;}
  const label=el('label','preference-row'),text=el('span','','Show contextual tips'),toggle=el('input');toggle.type='checkbox';toggle.checked=data.progress.enabled;toggle.setAttribute('role','switch');toggle.onchange=async()=>{toggle.disabled=true;try{await update('enabled',undefined,toggle.checked);}catch(error){toast(error.message);toggle.checked=data?.progress.enabled===true;}finally{toggle.disabled=false;}};label.append(text,toggle);
  const replay=featureButton('Replay guidance',async()=>{replay.disabled=true;try{await update('replay');sessionCompleted.clear();shownEpoch='';pausedUntil=Date.now()+3000;toast('Guidance reset. Tips appear when their actions are available.');}catch(error){toast(error.message);}finally{replay.disabled=false;}},'button');
  section.append(label,el('p','form-hint','Tips appear once per profile. Replaying them keeps your setup and other preferences.'),replay);
 };
 function visible(node){
  if(!node?.isConnected||node.disabled)return false;
  const r=node.getBoundingClientRect();if(r.width<=0||r.height<=0||r.top<0||r.left<0||r.bottom>innerHeight||r.right>innerWidth)return false;
  for(let parent=node;parent;parent=parent.parentElement||parent.getRootNode?.().host){
   const style=getComputedStyle(parent);if(parent.hidden||style.visibility==='hidden'||style.display==='none')return false;
   if(parent===node)continue;const clip=parent.getBoundingClientRect();
   if(/auto|scroll|hidden|clip/.test(style.overflowX)&&(r.left<clip.left||r.right>clip.right))return false;
   if(/auto|scroll|hidden|clip/.test(style.overflowY)&&(r.top<clip.top||r.bottom>clip.bottom))return false;
  }
  return true;
 }
 function typing(){const a=document.activeElement;return !!a?.closest('input,textarea,[contenteditable="true"]')||!!window.getSelection()?.toString()||typeof shortcutRecording!=='undefined'&&!!shortcutRecording;}
 function candidate(){
  if(data.setupComplete&&!data.assistantUsed&&!sessionCompleted.has('assistant.after-setup')&&guidanceEligible(data.progress,'assistant.after-setup'))return ['assistant.after-setup',$('#app-chat-toggle')];
  if($('#app-chat-panel')&&!$('#app-chat-panel').hidden)return null;
  const review=typeof reviewState==='undefined'?null:reviewState,dialog=$('#diff-review-dialog');
  if(review&&dialog?.open){
   if(!$('#diff-review-composer')?.hidden||$('#diff-review-drawer')?.open)return null;
   const draft=review.draft;
   if(guidanceEligible(data.progress,'review.pending-comments')&&!sessionCompleted.has('review.pending-comments')&&draft.comments.some(c=>c.author==='You'&&reviewEntryPending(draft,c)))return ['review.pending-comments',$('#diff-review-drawer-toggle')];
   if(guidanceEligible(data.progress,'review.comment-selection')&&!sessionCompleted.has('review.comment-selection')){
    for(const host of dialog.querySelectorAll('diffs-container')){const gutter=[...host.shadowRoot?.querySelectorAll('[data-gutter] [data-line-number-content]')||[]].find(visible);if(gutter)return ['review.comment-selection',gutter];}
   }
   const record=review.record;if(setting(record.agent==='codex'?'codexReviewCommand':'claudeReviewCommand','')&&candidateForRecord(record)?.host==='embedded')return ['review.agent-scope',$('#diff-review-agent')];
   return null;
  }
  const snooze=document.querySelector('.agent-tab.snooze-tip-ready .agent-tab-snooze');if(snooze&&visible(snooze)&&guidanceEligible(data.progress,'chat.snooze')&&!sessionCompleted.has('chat.snooze'))return ['chat.snooze',snooze];
  if(source!=='agents'||candidateForRecord(currentAgentRecord()||{})?.host!=='embedded')return null;
  const ci=[...document.querySelectorAll('.investigate-ci')].find(node=>{const id=node.closest('.context-work-ci')?.querySelector('[data-chat-ci]')?.dataset.chatCi,check=typeof chatCIChecks==='undefined'?null:chatCIChecks.get(id);return visible(node)&&check?.at&&Date.now()/1000-check.at<90&&!check.error;});if(ci&&guidanceEligible(data.progress,'ci.investigate')&&!sessionCompleted.has('ci.investigate'))return ['ci.investigate',ci];
  if(guidanceEligible(data.progress,'chat.shell')&&!sessionCompleted.has('chat.shell'))return ['chat.shell',$('#chat-terminal-toggle')];
  return null;
 }
 function position(){
  if(!active)return;const {node,anchor,highlight}=active;if(!visible(anchor)){remove();return;}
  const zoom=parseFloat(getComputedStyle(document.body).zoom)||1,margin=12;node.style.maxWidth=Math.max(0,(innerWidth-margin*2)/zoom)+'px';
  const r=anchor.getBoundingClientRect(),bounds=node.getBoundingClientRect(),gap=10;
  let left=r.left-bounds.width-gap,top=r.top+(r.height-bounds.height)/2;
  if(left<margin){left=Math.min(innerWidth-bounds.width-margin,r.right+gap);if(left<r.right&&left+bounds.width>r.left){left=Math.max(margin,Math.min(r.left,innerWidth-bounds.width-margin));top=r.bottom+gap;if(top+bounds.height>innerHeight-margin)top=r.top-bounds.height-gap;}}
  left=Math.max(margin,Math.min(left,innerWidth-bounds.width-margin));top=Math.max(margin,Math.min(top,innerHeight-bounds.height-margin));node.style.left=left/zoom+'px';node.style.top=top/zoom+'px';
  const padding=4;Object.assign(highlight.style,{left:(r.left-padding)/zoom+'px',top:(r.top-padding)/zoom+'px',width:(r.width+padding*2)/zoom+'px',height:(r.height+padding*2)/zoom+'px'});
 }
 function show(id,anchor){
  const tip=data.definitions.tips.find(t=>t.id===id);if(!tip)return;
  const node=el('aside','guidance-tip');node.id='truffle-guidance-tip';node.setAttribute('aria-label',tip.title);const title=el('strong','',tip.title),copy=el('p','',tip.text.replace('{shortcut}',shortcutLabel(setting('keybindings',{})['chat-terminal'])||'the terminal button'));
  const done=featureButton('Got it',()=>{remove();void update('dismiss',id).catch(error=>toast(error.message));},'text-button');
  const live=el('span','guidance-announcement');live.setAttribute('role','status');live.setAttribute('aria-live','polite');node.append(title,copy,done,live);
  const root=anchor.getRootNode(),host=root instanceof ShadowRoot?root.host:anchor;const parent=host.closest('dialog[open]')||document.body;
  const highlight=el('div','guidance-target-ring');highlight.setAttribute('aria-hidden','true');parent.append(highlight,node);
  active={id,anchor,node,highlight,previousDescription:anchor.getAttribute('aria-describedby')};
  // ARIA ID references cannot cross a shadow-root boundary.
  if(!(root instanceof ShadowRoot))anchor.setAttribute('aria-describedby',[active.previousDescription,node.id].filter(Boolean).join(' '));
  shownEpoch=epoch;position();live.textContent=tip.title+'. '+copy.textContent;
 }
 function tick(){
  if(!data||data.demo||document.hidden)return remove();
  const reviewOpen=$('#diff-review-dialog')?.open,newEpoch=[source,typeof selectedAgentKey==='undefined'?'':selectedAgentKey,reviewOpen?'review':'page'].join('|');if(newEpoch!==epoch){epoch=newEpoch;remove();pausedUntil=Date.now()+1500;}
  const dialogs=[...document.querySelectorAll('dialog[open]')];if(!data.progress.enabled||dialogs.some(d=>d.id!=='diff-review-dialog')||typing()||Date.now()-lastInput<1500&&!active?.node.contains(document.activeElement)){remove();return;}
  const target=candidate();if(active){if(!target||target[0]!==active.id||target[1]!==active.anchor||!guidanceEligible(data.progress,active.id)||sessionCompleted.has(active.id))remove();else position();return;}
  if(Date.now()<pausedUntil||shownEpoch===epoch||!target||sessionCompleted.has(target[0])||!guidanceEligible(data.progress,target[0])||!visible(target[1]))return;
  show(...target);
 }
 document.addEventListener('pointerdown',event=>{if(!active?.node.contains(event.composedPath()[0]))lastInput=Date.now();},true);
 document.addEventListener('keydown',event=>{if(!active?.node.contains(event.composedPath()[0]))lastInput=Date.now();},true);
 // Observe bubble usage without depending on the assistant provider or request outcome.
 $('#app-chat-toggle')?.addEventListener('click',()=>window.truffleGuidance.complete('assistant.after-setup'));
 window.addEventListener('resize',position);document.addEventListener('scroll',position,true);
 setInterval(tick,500);setInterval(()=>{if(!document.hidden)void load();},15000);void load();
})();
