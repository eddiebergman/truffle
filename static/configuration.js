'use strict';
function createConfigurationRefresh({fetchStatus,resources,blocked,onError}){
 const applied=new Map(),failures=new Set();let pending=null;
 return function refreshConfiguration(){
  if(blocked())return Promise.resolve(false);
  if(pending)return pending;
  pending=(async()=>{
   try{
    const status=await fetchStatus();
    if(status?.version!==1||!status.resources||typeof status.resources!=='object')throw new Error('Unsupported configuration status.');
    failures.delete('status');
    for(const [name,load]of Object.entries(resources)){
     if(blocked())return false;
     const resource=status.resources[name];
     if(!resource||typeof resource.token!=='string'||!/^[a-f0-9]{64}$/.test(resource.token)){
      if(!failures.has(name)){failures.add(name);onError(name);}continue;
     }
     if(applied.get(name)===resource.token){failures.delete(name);continue;}
     try{if(await load()===false)continue;applied.set(name,resource.token);failures.delete(name);}
     catch{if(!failures.has(name)){failures.add(name);onError(name);}}
    }
    return true;
   }catch{if(!failures.has('status')){failures.add('status');onError('status');}return false;}
  })().finally(()=>{pending=null;});return pending;
 };
}
function configurationRefreshBlocked(){return settingsDirty||Boolean(document.querySelector('dialog[open]'));}
if(typeof module!=='undefined')module.exports={createConfigurationRefresh};
if(typeof window!=='undefined'){
 const canApply=()=>!configurationRefreshBlocked();
 const refreshConfiguration=createConfigurationRefresh({fetchStatus:()=>api('/api/config-status'),blocked:configurationRefreshBlocked,resources:{
  settings:async()=>{const next=await loadAppSettings({canApply});if(!canApply())return false;if(!next||next.configError)throw new Error('Invalid settings configuration.');},
  views:async()=>{const next=await loadSavedViews({canApply});if(!canApply())return false;if(!next||savedViewsError)throw new Error('Invalid saved views.');},
  pageSettings:async()=>{const next=await loadPageSettings({canApply});if(!canApply())return false;if(!next)throw new Error('Page visibility could not load.');}
 },onError:name=>toast('Could not refresh '+({settings:'settings',views:'saved views',pageSettings:'page visibility',status:'configuration status'}[name])+'. Changes will be retried.')});
 window.refreshConfiguration=refreshConfiguration;
 window.addEventListener('focus',()=>{if(!document.hidden)void refreshConfiguration();});
 document.addEventListener('visibilitychange',()=>{if(!document.hidden)void refreshConfiguration();});
}
