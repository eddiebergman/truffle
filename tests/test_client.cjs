const {test}=require('node:test');const assert=require('node:assert/strict');
const {createDashboardClient}=require('../static/client.js');
test('browser transport keeps CSRF inside the client and preserves structured errors',async()=>{
  const calls=[];const client=createDashboardClient({fetch:async(path,options)=>{calls.push({path,options});return {status:path==='/api/state'?200:409,json:async()=>path==='/api/state'?{csrf:'private',items:[]}:{error:'Choose a folder.',location:{root:'/code'}}};}});
  assert.deepEqual(await client.request('/api/state'),{items:[]});
  await assert.rejects(client.request('/api/session/quick',{agent:'claude'}),error=>error.data.location.root==='/code'&&error.status===409);
  assert.equal(calls[1].options.headers['X-CSRF-Token'],'private');
});
test('desktop transport uses the service port without HTTP or credentials in the UI',async()=>{
  const calls=[];const client=createDashboardClient({fetch:()=>{throw Error('HTTP should not run');},native:async(command,args)=>{calls.push({command,args});return {status:200,data:{items:[]}};}});
  assert.deepEqual(await client.request('/api/state'),{items:[]});assert.equal(calls[0].command,'service_request');
  assert.deepEqual(calls[0].args,{path:'/api/state',payload:null});assert.equal(client.isDesktop,true);
});
test('artifact views work through either transport and reject paths',async()=>{
  const id='d82f1f3d3b8f1abc330d8205';
  const browser=createDashboardClient({fetch:async()=>({ok:true,text:async()=>'# Report'})});
  const native=createDashboardClient({native:async(command,args)=>{assert.equal(command,'read_artifact');assert.equal(args.id,id);return '# Report';}});
  assert.equal(browser.artifactUrl(id),'/artifacts/'+id);assert.equal(native.artifactUrl(id),'mydash-artifact://localhost/'+id);
  assert.equal(await browser.readArtifact(id),'# Report');assert.equal(await native.readArtifact(id),'# Report');
  assert.throws(()=>native.artifactUrl('../../.env'));
});
test('offline connections report unavailability without losing operation error details',async()=>{
  const states=[];const client=createDashboardClient({native:async()=>({status:503,data:{error:'Service unavailable'}}),onConnection:value=>states.push(value)});
  await assert.rejects(client.request('/api/state'),/Service unavailable/);assert.deepEqual(states,[false]);
});

test('editor launch uses native IPC with the exact worktree path and preserves failures',async()=>{
  const calls=[];const client=createDashboardClient({native:async(command,args)=>calls.push({command,args})});
  await client.openDiff('/code/repo/.worktrees/task with spaces');assert.deepEqual(calls,[{command:'open_git_diff_in_zed',args:{path:'/code/repo/.worktrees/task with spaces'}}]);
  await assert.rejects(client.openDiff('relative'),/absolute/);
  await assert.rejects(createDashboardClient().openDiff('/code/repo'),/desktop app/);
  await assert.rejects(createDashboardClient({native:async()=>{throw 'Zed unavailable';}}).openDiff('/code/repo'),error=>error==='Zed unavailable');
});

test('clipboard copies text or image content through the right transport without accepting file paths',async()=>{
  const calls=[],id='d82f1f3d3b8f1abc330d8205';
  const native=createDashboardClient({native:async(command,args)=>calls.push({command,args})});
  await native.copyText('line 1\nline 2');await native.copyArtifactImage(id);
  assert.deepEqual(calls,[{command:'copy_text',args:{text:'line 1\nline 2'}},{command:'copy_artifact_image',args:{id}}]);
  await assert.rejects(native.copyArtifactImage('/tmp/image.png'),/Invalid artifact/);assert.equal(calls.length,2);
  const copied=[];await createDashboardClient({clipboard:{writeText:async value=>copied.push(value)}}).copyText('table content');assert.deepEqual(copied,['table content']);
});

test('restart controls use fixed native commands and require a reviewed snapshot',async()=>{
  const calls=[];const client=createDashboardClient({native:async(command,args)=>{calls.push({command,args});return {status:'working'};}});
  await client.restartPreview();await client.restartStatus();await client.restartStart({mode:'service',snapshot:'reviewed'});
  assert.deepEqual(calls.map(c=>c.command),['restart_dashboard_preview','restart_dashboard_status','restart_dashboard_start']);assert.deepEqual(calls[2].args,{mode:'service',confirmed:false,snapshot:'reviewed'});
  await assert.rejects(client.restartStart({mode:'kill-all',snapshot:'reviewed'}),/Review/);await assert.rejects(createDashboardClient().restartPreview(),/desktop app/);
});

test('native transport preserves conflict status for explicit recovery controls',async()=>{
 const client=createDashboardClient({native:async()=>({status:409,data:{error:'Changed elsewhere'}})});
 await assert.rejects(client.request('/api/page-settings',{revision:1}),error=>error.status===409&&error.data.error==='Changed elsewhere');
});
