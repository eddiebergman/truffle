const {test}=require('node:test'),assert=require('node:assert/strict');const {restartImpact}=require('../static/restart.js');
test('service restart preserves daemon terminals; legacy and terminal-host restarts require consent',()=>{
  const current={liveCount:3,preserveTerminals:true,problems:[]};assert.equal(restartImpact(current,'service').interrupts,false);assert.match(restartImpact(current,'service').message,/keep running/);
  assert.equal(restartImpact(current,'terminals').interrupts,true);assert.match(restartImpact(current,'terminals').message,/subagents/);
  assert.equal(restartImpact({...current,preserveTerminals:false},'service').interrupts,true);assert.equal(restartImpact({liveCount:0},'terminals').interrupts,false);
});
test('missing resume metadata blocks disruptive actions while allowing a preserving restart',()=>{
  const current={liveCount:1,preserveTerminals:true,problems:['Missing chat UUID']};assert.equal(restartImpact(current,'service').blocked,false);assert.equal(restartImpact(current,'terminals').blocked,true);
});
