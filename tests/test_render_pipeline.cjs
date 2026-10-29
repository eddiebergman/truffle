const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),workspace=fs.readFileSync('static/workspace.js','utf8');
const pipeline=app.slice(app.indexOf('const renderHooks='),app.indexOf('function renderWorkPage()'));
function fixture(){const events=[],c=vm.createContext({state:{},source:'github',renderWorkspacePage:()=>{events.push('page');return 42;}});vm.runInContext(pipeline,c);return {c,events};}
test('named render hooks run in explicit order regardless of registration order',()=>{
 for(const reverse of [false,true]){
  const f=fixture(),registers=[['prepare','before',0],['beta','after',20],['alpha','after',20],['last','finally',0]];
  for(const [name,phase,order]of reverse?registers.toReversed():registers)f.c.registerRenderHook(name,()=>f.events.push(name),{phase,order});
  assert.equal(f.c.render(),42);assert.deepEqual(f.events,['prepare','page','alpha','beta','last']);assert.throws(()=>f.c.registerRenderHook('alpha',()=>{}),/already registered/);
 }
});
test('page errors still run all shared cleanup hooks and preserve the original error',()=>{
 const f=fixture(),original=new Error('page failed');f.c.renderWorkspacePage=()=>{throw original;};
 f.c.registerRenderHook('first',()=>{f.events.push('first');throw new Error('cleanup failed');},{order:10});f.c.registerRenderHook('second',()=>f.events.push('second'),{order:20});f.c.registerRenderHook('timing',()=>f.events.push('timing'),{phase:'finally'});
 assert.throws(()=>f.c.render(),error=>error===original);assert.deepEqual(f.events,['first','second','timing']);
});
test('before errors stop page updates but still clean up; an unloaded state does no work',()=>{
 const f=fixture();f.c.registerRenderHook('prepare',()=>{throw new Error('prepare failed');},{phase:'before'});f.c.registerRenderHook('cleanup',()=>f.events.push('cleanup'));assert.throws(()=>f.c.render(),/prepare failed/);assert.deepEqual(f.events,['cleanup']);f.c.state=null;assert.equal(f.c.render(),undefined);assert.deepEqual(f.events,['cleanup']);
});
test('workspace owner routes each page while shared controls and gear always refresh',()=>{
 const nodes=new Map(),events=[],node=()=>({hidden:false,textContent:'',classList:{toggle(){}},dataset:{}}),$=key=>{if(!nodes.has(key))nodes.set(key,node());return nodes.get(key);};
 const c=vm.createContext({$,state:{sessions:[],changes:[]},source:'agents',selectedAgentKey:'',document:{body:{classList:{toggle(){}}},querySelectorAll:()=>[]},workspaceIndexedState:null,terminalFocus:true,allItems:()=>[],buildBundleIndex(){},workViewCount:()=>0,candidateForRecord:()=>null,renderLiveBar(){},renderSourceSubmenus(){},prepareSavedViewPage:()=>events.push('saved-state'),renderSavedViewControls:()=>events.push('saved-controls'),renderSavedViewSidebar:()=>events.push('saved-sidebar'),renderPageSettingsGear:()=>events.push('gear'),renderFolderSelector(){},renderAgentWorkspace:()=>events.push('agents'),renderSkills:()=>events.push('skills'),renderChatSearchResults:()=>events.push('chat-results'),renderHistory:()=>events.push('sessions'),parkTerminals:()=>events.push('park'),renderSync(){},paintWorkSelection(){},renderWorkPage:()=>events.push('work')});
 vm.runInContext(pipeline,c);vm.runInContext(workspace.slice(workspace.indexOf('let agentNavigationEpoch='),workspace.indexOf('function chooseDefaultAgentSelection(')),c);vm.runInContext(workspace.slice(workspace.indexOf('function prepareWorkspaceRender()'),workspace.indexOf("for(const button of document.querySelectorAll('[data-source]'))if",workspace.indexOf('function prepareWorkspaceRender()'))),c);
 for(const page of ['agents','skills','sessions','chat-results','github','linear','sentry','all','notes','logs','changes','archive']){events.length=0;c.source=page;c.render();const owner=['agents','skills','sessions','chat-results'].includes(page)?page:'work';assert.ok(events.includes(owner),page+' page renderer');assert.deepEqual(events.slice(-3),['saved-controls','saved-sidebar','gear'],page+' shared controls');assert.equal(events[0],'saved-state');assert.equal(events.includes('park'),page!=='agents',page+' terminal lifecycle');}
});

test('shared UX hook cleans active chat effects on every page, including a failed page render',()=>{
 const f=fixture(),nodes=new Map(),effects=[],node=()=>({hidden:false,classList:{toggle(){}},replaceChildren(){},append(){}});f.c.$=key=>{if(!nodes.has(key))nodes.set(key,node());return nodes.get(key);};f.c.document={body:{classList:{toggle:(name,value)=>effects.push([name,value])}},querySelectorAll:()=>[]};f.c.syncActiveDiff=()=>effects.push('diff');f.c.syncActiveChatCI=()=>effects.push('ci');f.c.leftSidebarCollapsed=true;f.c.featureButton=()=>({setAttribute(){}});f.c.activityView='sessions';
 const ux=fs.readFileSync('static/ux.js','utf8');vm.runInContext(ux.slice(ux.indexOf('function renderSharedUX()'),ux.indexOf("let snoozeDueSignature=")),f.c);
 for(const page of ['agents','github','logs']){effects.length=0;f.c.source=page;f.c.render();assert.ok(effects.includes('diff'));assert.ok(effects.includes('ci'));assert.equal(f.c.$('#activity-tabs').hidden,page!=='logs');assert.equal(f.c.$('.launcher').hidden,false);assert.ok(effects.some(value=>Array.isArray(value)&&value[0]==='left-sidebar-collapsed'&&value[1]));}
 effects.length=0;f.c.renderWorkspacePage=()=>{throw new Error('page');};assert.throws(()=>f.c.render(),/page/);assert.ok(effects.includes('diff'));assert.ok(effects.includes('ci'));
});
test('restart availability and GitHub attention hooks retain their actual UI effects',()=>{
 const f=fixture(),nodes=new Map(),node=()=>({attrs:{},setAttribute(name,value){this.attrs[name]=value;},querySelectorAll:()=>[]}),$=key=>{if(!nodes.has(key))nodes.set(key,node());return nodes.get(key);};f.c.$=$;f.c.document={addEventListener(){},querySelectorAll:()=>[]};f.c.serviceClient={isDesktop:false};
 vm.runInContext(fs.readFileSync('static/restart.js','utf8'),f.c);assert.equal($('#restart-toggle').disabled,true);f.c.state.demo=true;f.c.render();assert.equal($('#restart-toggle').disabled,false);assert.match($('#restart-toggle').title,/Preview/);
 f.c.state.githubReviewInbox={pr:{active:true,seenAt:null,version:1}};f.c.allItems=()=>[{id:'pr',source:'github',kind:'Pull request',relations:['reviews'],status:'todo'}];vm.runInContext(fs.readFileSync('static/github-actions.js','utf8'),f.c);f.c.render();assert.equal($('#github-review-dot').hidden,false);assert.equal($('#github-review-dot').attrs['aria-label'],'1 new PR ready for your review');f.c.state.githubReviewInbox.pr.seenAt=1;f.c.render();assert.equal($('#github-review-dot').hidden,true);
});
