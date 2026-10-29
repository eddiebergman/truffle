/* UI-independent service client. Browser HTTP and desktop IPC implement the same contract. */
'use strict';
function createDashboardClient({fetch:fetcher=globalThis.fetch,native=null,onConnection=()=>{},clipboard=globalThis.navigator?.clipboard}={}){
  let csrf='';
  function failure(message,data,status){const error=new Error(message);error.data=data;if(Number.isInteger(status))error.status=status;return error;}
  async function request(path,payload){
    let data,status;
    try{
      if(native){({status,data}=await native('service_request',{path,payload:payload??null}));}
      else{
        const response=await fetcher(path,payload===undefined?{}:{method:'POST',headers:{'Content-Type':'application/json','X-CSRF-Token':csrf},body:JSON.stringify(payload)});
        status=response.status;data=await response.json();
        if(data.csrf){csrf=data.csrf;delete data.csrf;}
      }
    }catch(_){onConnection(false);throw failure('Cannot reach the local mydash service.',{});}
    onConnection(status!==503);
    if(status<200||status>=300)throw failure(data.error||'Request failed. Try again.',data,status);
    return data;
  }
  function artifactUrl(id){if(!/^[a-f0-9]{24}$/i.test(id))throw new Error('Invalid artifact ID.');return native?'mydash-artifact://localhost/'+id:'/artifacts/'+id;}
  async function readArtifact(id){
    const path=artifactUrl(id);
    if(native)return native('read_artifact',{id});
    const response=await fetcher(path);if(!response.ok)throw new Error('Artifact unavailable.');return response.text();
  }
  async function copyText(text){
    if(native)return native('copy_text',{text:await text});
    if(!clipboard?.writeText)throw new Error('Clipboard access is unavailable. Select the text and copy it.');
    if(typeof text!=='string'&&clipboard.write&&typeof ClipboardItem!=='undefined')return clipboard.write([new ClipboardItem({'text/plain':Promise.resolve(text).then(value=>new Blob([value],{type:'text/plain'}))})]);
    return clipboard.writeText(await text);
  }
  async function copyArtifactImage(id){
    const url=artifactUrl(id);
    if(native)return native('copy_artifact_image',{id});
    if(!clipboard?.write||typeof ClipboardItem==='undefined')throw new Error('This browser cannot copy images. Try the desktop app.');
    const image=(async()=>{const response=await fetcher(url);if(!response.ok)throw new Error('Artifact unavailable.');const blob=await response.blob();if(!['image/png','image/jpeg','image/webp'].includes(blob.type)||blob.size>10*1024*1024)throw new Error('Unsupported clipboard image.');return clipboardPNG(blob);})();
    // Pass the promise immediately, preserving Safari's user-gesture requirement.
    return clipboard.write([new ClipboardItem({'image/png':image})]);
  }
  async function restartPreview(){if(!native)throw new Error('Restart controls require the desktop app.');return native('restart_dashboard_preview');}
  async function restartStatus(){if(!native)throw new Error('Restart controls require the desktop app.');return native('restart_dashboard_status');}
  async function restartStart({mode,confirmed=false,snapshot}){if(!native)throw new Error('Restart controls require the desktop app.');if(!['service','terminals'].includes(mode)||typeof snapshot!=='string')throw new Error('Review the restart first.');return native('restart_dashboard_start',{mode,confirmed,snapshot});}
  async function openDiff(path){
    if(!native)throw new Error('Open in Zed is available in the desktop app.');
    if(typeof path!=='string'||!path.startsWith('/'))throw new Error('The agent has no absolute working directory.');
    return native('open_git_diff_in_zed',{path});
  }
  return {request,artifactUrl,readArtifact,copyText,copyArtifactImage,restartPreview,restartStatus,restartStart,openDiff,isDesktop:!!native};
}
if(typeof module!=='undefined')module.exports={createDashboardClient};
