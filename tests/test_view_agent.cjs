const {test}=require('node:test');const assert=require('node:assert/strict');
const {workViewRequest}=require('../static/view-agent.js');
test('view context includes all filtered matches beyond the rendered first page, with stable order',()=>{
  const items=Array.from({length:150},(_,i)=>({id:'item-'+i}));
  const filters={query:'bug',scope:'org/repo',githubView:'review',status:'active'};
  const request=workViewRequest([...items,items[0]],{source:'github',filters},'  What needs attention?  ');
  assert.equal(request.itemIds.length,150);assert.equal(request.itemIds[149],'item-149');assert.deepEqual(request.filters,filters);assert.equal(request.question,'What needs attention?');
});
test('empty and oversized work views require an explicit narrower selection',()=>{
  assert.throws(()=>workViewRequest([],{source:'linear',filters:{}},'Check'),/no items/);
  assert.throws(()=>workViewRequest(Array.from({length:4097},(_,i)=>({id:String(i)})),{source:'sentry',filters:{}},'Check'),/Narrow/);
});
