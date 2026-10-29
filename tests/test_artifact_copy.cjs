const {test}=require('node:test'),assert=require('node:assert/strict');
const {artifactCSV,artifactMarkdown}=require('../static/artifact-copy.js');
test('CSV clipboard preserves quotes, newlines, Unicode, empty cells and numeric zero',()=>{
  assert.equal(artifactCSV([['name','value'],['a,b','two\nlines'],['"quote"',0],[null,'α']]),'name,value\r\n"a,b","two\nlines"\r\n"""quote""",0\r\n,α\r\n');
});
test('Markdown clipboard escapes table separators and includes rows beyond the preview limit',()=>{
  const rows=[['key','value'],['a|b','<script>\nsecond line'],...Array.from({length:1005},(_,i)=>['row',i])];
  const result=artifactMarkdown(rows);assert.ok(result.includes('a\\|b'));assert.ok(result.includes('&lt;script&gt;<br>second line'));assert.ok(result.includes('| row | 1004 |'));assert.equal(result.split('\n').length,rows.length+2);
});
