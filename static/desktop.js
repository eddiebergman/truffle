/* Native presentation/lifecycle only. Work records and sync remain in the service. */
'use strict';
if(serviceClient.isDesktop){
  document.documentElement.classList.add('desktop-app');
  if(/Mac/.test(navigator.platform))document.documentElement.classList.add('mac-titlebar');
  window.addEventListener('mydash-tab-command',event=>{void runTabCommand(event.detail);});
  const titlebar=$('#agent-topbar');
  titlebar.addEventListener('mousedown',event=>{
    if(event.button!==0||event.target.closest('button,a,input,select,textarea,summary,details,.agent-tab'))return;
    event.preventDefault();void window.__TAURI__.core.invoke('drag_titlebar').catch(()=>{});
  });
  const panel=el('section','desktop-connection');panel.id='desktop-connection';panel.hidden=true;
  panel.append(el('div','desktop-connection-mark','t'),el('h1','','Connect your local service'),el('p','','truffle has its own app window. Start the service inside cmux to keep your existing agent controls working.'));
  const code=el('pre','','cd ~/code/mydash\n./start.command');panel.append(code);
  const buttons=el('div','desktop-connection-actions');
  const retry=featureButton('Retry connection',()=>checkDesktopService(),'button primary');
  const terminal=featureButton('Open cmux',async()=>{try{await window.__TAURI__.core.invoke('open_cmux');}catch(error){toast(String(error));}},'button');
  const copy=featureButton('Copy start command',async()=>{try{await navigator.clipboard.writeText(code.textContent);toast('Start command copied.');}catch(_){toast('Select and copy the command above.');}},'text-button');
  buttons.append(retry,terminal,copy);panel.append(buttons,el('p','form-hint','Your cached work stays in ~/code/mydash/data. Closing this window does not stop the service.'));
  document.body.append(panel);
  async function checkDesktopService(){
    try{await window.__TAURI__.core.invoke('service_status');const reconnect=panel.hidden===false||!state;panel.hidden=true;if(reconnect)await refresh();}
    catch(error){if(!state)panel.hidden=false;panel.querySelector('h1').textContent=String(error).includes('update')?'Restart your local service':'Connect your local service';}
  }
  window.addEventListener('mydash-connection',event=>{if(event.detail){panel.hidden=true;}else if(!state)panel.hidden=false;});
  void checkDesktopService();setInterval(checkDesktopService,30000);
}
