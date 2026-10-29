'use strict';
(()=>{
 let value=null,lastInteraction=Date.now(),visitAt=0,writing=Promise.resolve();
 const mark=action=>{writing=writing.catch(()=>{}).then(async()=>{if(!value)return;try{value.progress=await api('/api/feedback',{revision:value.progress.revision,action});}catch(error){await load();throw error;}});return writing;};
 async function load(){try{value=await api('/api/feedback');}catch{value=null;}}
 function link(url,label){const a=el('a','button',label);a.href=url;a.target='_blank';a.rel='noopener noreferrer';return a;}
 function preserveConversationBottom(change){
  const panel=$('#app-chat-panel'),conversation=$('.app-chat-conversation');
  const follow=panel&&!panel.hidden&&conversation?.clientHeight>0&&$('#app-chat-setup')?.hidden!==false&&conversation.scrollHeight-conversation.clientHeight-conversation.scrollTop<=2;
  change();if(follow)conversation.scrollTop=conversation.scrollHeight;
 }
 window.openFeedbackDraft=(draft={})=>{
  if(!value){toast('Feedback links are unavailable. Try again in a moment.');return;}
  let dialog=$('#feedback-draft');if(!dialog){dialog=el('dialog','feedback-dialog');dialog.id='feedback-draft';document.body.append(dialog);}dialog.replaceChildren();
  const title=el('input'),body=el('textarea'),error=el('p','form-hint');title.value=draft.title||'';title.placeholder='What could be better?';title.setAttribute('aria-label','Issue title');body.value=draft.body||'';body.placeholder='What happened, what you expected, and how to reproduce it. Review any private information before sharing.';body.rows=7;body.setAttribute('aria-label','Issue description');error.setAttribute('role','alert');
  const open=link(value.config.repository+'/issues/new','Open GitHub draft');const update=()=>{open.href=value.config.repository+'/issues/new?'+new URLSearchParams({title:title.value,body:body.value}).toString();};title.oninput=body.oninput=update;update();
  const close=featureButton('Cancel',()=>dialog.close(),'button'),ask=featureButton('Ask the app assistant',()=>{
   try{if(typeof window.prepareAppChatMessage!=='function')throw new Error('App chat is unavailable. Your feedback is kept here.');window.prepareAppChatMessage('Help me draft a Truffle issue about: '+title.value+'\n\n'+body.value);error.textContent='';
    dialog.addEventListener('close',()=>{if(!document.querySelector('dialog[open]')&&!$('#app-chat-panel')?.hidden)$('.app-chat-form')?.querySelector('textarea')?.focus();},{once:true});dialog.close();}
   catch(problem){error.textContent=problem.message||'The feedback could not be prepared. Your draft is kept here.';}
  },'button');
  open.onclick=()=>{if(!title.value.trim()){error.textContent='Add a short title before opening the draft.';return false;}dialog.close();};
  dialog.append(el('h2','','Report an issue'),el('p','form-hint','Review the draft here. GitHub opens next; you choose whether to publish it.'),title,body,error,ask,close,open);dialog.showModal();title.focus();
 };
 const feedback=featureButton('Feedback',()=>openFeedbackDraft(),'text-button app-feedback-button');feedback.title='Report an issue or ask the assistant for help';$('.app-chat-new')?.before(feedback);
 async function check(){
  if(!value||document.hidden||value.demo)return;
  if(Date.now()-visitAt>3600000){visitAt=Date.now();await mark('visit').catch(()=>{});await load();}
  const panel=$('#app-chat-panel');
  if(panel&&!panel.hidden&&value.progress.hint==='unseen'&&$('.app-chat-messages')?.getAttribute('aria-busy')!=='true'&&panel.querySelector('.is-assistant')&&!panel.querySelector('.app-feedback-hint')){
   const hint=el('div','app-feedback-hint'),copy=el('span','',value.config.feedbackText),report=featureButton('Report an issue',()=>openFeedbackDraft(),'text-button'),close=featureButton('×',async()=>{try{await mark('hint-dismiss');preserveConversationBottom(()=>hint.remove());}catch(error){toast(error.message);}},'text-button');close.setAttribute('aria-label','Dismiss feedback suggestion');hint.append(copy,report,close);preserveConversationBottom(()=>$('.app-chat-form').before(hint));
  }
  if(!value.starEligible||Date.now()-lastInteraction<15000||source==='agents'&&(liveCandidates().length>0||currentAgentRecord())||document.querySelector('dialog[open]')||panel&&!panel.hidden||$('#truffle-guidance-tip')||document.activeElement?.closest('input,textarea,[contenteditable="true"],.embedded-terminal'))return;
  let dialog=$('#feedback-star');if(dialog)return;dialog=el('dialog','feedback-dialog');dialog.id='feedback-star';let opened=false;const star=link(value.config.repository,'Star on GitHub'),later=featureButton('Not now',()=>dialog.close(),'button');star.onclick=()=>{opened=true;void mark('star-open').catch(()=>{});dialog.close();};dialog.onclose=()=>{if(!opened&&value.progress.star==='unseen')void mark('star-dismiss').catch(()=>{});};dialog.append(el('h2','',value.config.starTitle),el('p','',value.config.starText),later,star);document.body.append(dialog);dialog.showModal();later.focus();
 }
 document.addEventListener('pointerdown',()=>lastInteraction=Date.now(),true);document.addEventListener('keydown',()=>lastInteraction=Date.now(),true);
 void load();setInterval(check,5000);setInterval(()=>{if(!document.hidden)void load();},60000);
})();
