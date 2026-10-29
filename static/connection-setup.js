'use strict';
const connectionSetupProviders=[
 {id:'github',label:'GitHub',url:'https://github.com/settings/personal-access-tokens/new?name=truffle&description=Read+work+items+in+truffle&expires_in=30&issues=read&pull_requests=read&checks=read&statuses=read',permissions:'Choose the account or organization and selected repositories. Grant Issues: Read, Pull requests: Read, Checks: Read and Commit statuses: Read. Metadata read access is included. Organization approval may be required.'},
 {id:'linear',label:'Linear',url:'https://linear.app/settings/account/security',permissions:'Create a personal API key with Read permission. Restrict it to the teams you want to use. Write and Admin permissions are not required.'},
 {id:'sentry',label:'Sentry',url:'https://sentry.io/settings/account/api/auth-tokens/new-token/',permissions:'Create a personal token with org:read, project:read and event:read. Enter your organization slug and choose its cloud region. An organization token identifies the organization rather than a person.'}
];
function connectionSetupPayload(provider,{token='',source='manual',org='',base=''}={}){
 if(!connectionSetupProviders.some(item=>item.id===provider))throw new Error('Unknown connection provider.');
 const payload={provider,source};
 if(source==='manual'){if(!token.trim())throw new Error('Enter a token first.');payload.token=token.trim();}
 else if(source!=='github-cli'||provider!=='github')throw new Error('GitHub CLI is available only for GitHub.');
 if(provider==='sentry'){payload.org=org.trim();payload.base=base;if(!payload.org)throw new Error('Enter the Sentry organization slug.');}
 return payload;
}
window.renderConnectionSetup=async function(container,{onSaved}={}){
 const make=(tag,text,className)=>{const node=document.createElement(tag);if(text)node.textContent=text;if(className)node.className=className;return node;};
 const native=window.__TAURI__?.core,invoke=native?.invoke?.bind(native);
 const root=make('section',null,'connection-setup');root.setAttribute('aria-label','Secure integration setup');container.append(root);
 const status=make('p',null,'connection-setup-status');status.setAttribute('role','status');root.append(status);
 let view={revision:0,providers:[],available:false},busy=false,pending=null,disposed=false;
 const controls=[],previews=[];
 const setBusy=value=>{busy=value;root.setAttribute('aria-busy',String(value));for(const control of controls)control.disabled=value||!view.available||disposed;};
 const clearPreview=()=>{pending=null;for(const preview of previews)preview.replaceChildren();};
 const cancel=async()=>{clearPreview();if(invoke)await invoke('connection_setup_cancel');};
 const dialog=container.closest('dialog'),onClose=()=>void root.disposeConnectionSetup().catch(()=>{});
 root.disposeConnectionSetup=async()=>{if(disposed)return;disposed=true;setBusy(false);for(const input of root.querySelectorAll('input[type="password"]'))input.value='';dialog?.removeEventListener('close',onClose);await cancel();};
 dialog?.addEventListener('close',onClose,{once:true});
 if(invoke){try{view=await invoke('connection_setup_view');}catch(error){if(!disposed)status.textContent='Secure connection setup is unavailable: '+String(error);}}
 if(disposed)return root;
 const action=(label,handler,{primary=false}={})=>{
  const button=make('button',label,'ui-control connection-setup-button');button.type='button';button.dataset.variant=primary?'primary':'secondary';controls.push(button);
  button.onclick=async event=>{if(!event.isTrusted||busy||disposed||!view.available)return;setBusy(true);status.textContent='';try{await handler();}catch(error){status.textContent=String(error?.message||error);}finally{setBusy(false);}};return button;
 };
 if(!view.available)root.append(make('p','Secure token entry requires the macOS desktop app. This preview does not accept or store tokens. You can skip connections and return later.','form-hint'));
 else root.append(make('p','Tokens go directly to the native connection broker. They are not sent to a chat or included in exports. The app assistant can see saved connection and synchronization status, never the token. Review the account below, then approve saving in the macOS login Keychain. macOS may ask you to allow access again after an app update.','form-hint'));
 for(const provider of connectionSetupProviders){
  const configured=view.providers.find(item=>item.id===provider.id),card=make('section',null,'connection-setup-card'),heading=make('h4',provider.label),link=make('a','Create a '+provider.label+' token');link.href=provider.url;link.target='_blank';link.rel='noopener noreferrer';card.append(heading,make('p',provider.permissions,'form-hint'),link);
  const summary=make('p',configured?.configured?'Saved account: '+configured.identity:'No connection saved through this setup.','connection-setup-account');card.append(summary);
  const tokenLabel=make('label','API token'),input=make('input');input.type='password';input.autocomplete='off';input.spellcheck=false;input.maxLength=8192;input.setAttribute('aria-label',provider.label+' API token');input.setAttribute('data-agent-context-exclude','true');input.setAttribute('data-private','true');input.setAttribute('autocapitalize','off');input.setAttribute('autocorrect','off');input.name='connection-'+provider.id+'-secret';input.className='ui-field';controls.push(input);tokenLabel.append(input);card.append(tokenLabel);
  let org,base,importButton=null;
  if(provider.id==='sentry'){
   const orgLabel=make('label','Organization slug');org=make('input');org.className='ui-field';org.maxLength=100;org.placeholder='your-organization';org.value=configured?.org||'';org.setAttribute('aria-label','Sentry organization slug');orgLabel.append(org);
   const baseLabel=make('label','Cloud region');base=make('select');base.className='ui-field';for(const [value,label]of [['https://sentry.io','Sentry global'],['https://us.sentry.io','United States'],['https://de.sentry.io','Germany']]){const option=make('option',label);option.value=value;base.append(option);}base.value=configured?.base||'https://sentry.io';base.setAttribute('aria-label','Sentry cloud region');baseLabel.append(base);controls.push(org,base);card.append(orgLabel,baseLabel);
  }
  const preview=make('div',null,'connection-setup-preview');preview.setAttribute('aria-live','polite');previews.push(preview);
  const offerImport=()=>{if(importButton)return;importButton=action('Import work items',async()=>{const refreshed=await api('/api/connections/reload',{provider:provider.id,activate:true});if(refreshed.errors?.[provider.id])throw new Error(refreshed.errors[provider.id]);status.textContent=provider.label+' import requested. Available work items depend on your account and token permissions.';if(onSaved)await onSaved(provider.id);});buttons.append(importButton);};
  const validate=async(source)=>{
   const entered=input.value;input.value='';clearPreview();await invoke('connection_setup_cancel');if(disposed)return;
   const payload=connectionSetupPayload(provider.id,{token:entered,source,org:org?.value,base:base?.value});
   const result=await invoke('connection_setup_preview',{payload});if(disposed){await invoke('connection_setup_cancel');return;}pending=result;preview.append(make('strong',(result.identityKind==='organization'?'Organization: ':'Account: ')+result.identity));
   const list=make('ul');for(const name of result.teams||[])list.append(make('li','Team: '+name));for(const name of result.resources||[])list.append(make('li',name));preview.append(make('p',result.resourceLabel||'Resources'),list);
   if(!(result.resources||[]).length)preview.append(make('p','No resources returned. Check the selected account and token access before saving.','form-hint'));
   if(result.truncated)preview.append(make('p','Showing up to 20 resources. More may be available.','form-hint'));
   preview.append(make('p','This preview does not import work items or prove access to every item. Saving connects this account; synchronization uses its existing permissions.','form-hint'));
   preview.append(action('Save connection',async()=>{
    if(pending!==result)throw new Error('Validate this connection again before saving.');
    view=await invoke('connection_setup_commit',{payload:{previewId:result.previewId,expectedRevision:result.expectedRevision}});clearPreview();summary.textContent='Saved account: '+result.identity;
    try{const refreshed=await api('/api/connections/reload',{provider:provider.id});if(refreshed.errors?.[provider.id])throw new Error(refreshed.errors[provider.id]);}catch(error){status.textContent='Saved in Keychain. The service could not activate this connection: '+String(error?.message||error);return;}
    offerImport();
    status.textContent=view.warning||provider.label+' connection saved in macOS Keychain. Existing synchronization settings still apply.';if(onSaved)await onSaved(provider.id);
   },{primary:true}),action('Cancel preview',cancel));
  };
  const buttons=make('div',null,'connection-setup-actions');buttons.append(action('Validate token',()=>validate('manual'),{primary:true}));
  if(provider.id==='github'){buttons.append(action('Use GitHub CLI login',()=>validate('github-cli')));card.append(make('p','The desktop app asks before reading gh auth token --hostname github.com --no-refresh. The token is captured privately; no command output enters chat.','form-hint'));}
  if(configured?.configured)offerImport();
  buttons.append(action('Skip '+provider.label,async()=>{input.value='';await cancel();status.textContent=provider.label+' skipped. Existing connections remain unchanged.';}));card.append(buttons,preview);root.append(card);
 }
 setBusy(false);
 return root;
};
