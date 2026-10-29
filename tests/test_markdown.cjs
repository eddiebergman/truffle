const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const context=vm.createContext({URL});
for(const filename of ['static/vendor/markdown-it.min.js','static/markdown.js'])vm.runInContext(fs.readFileSync(path.join(__dirname,'..',filename),'utf8'),context);
const render=text=>context.renderBriefingMarkdown(text);

test('renders briefing structure, nested lists and code without changing code contents',()=>{
  const html=render('# Work review\n\n## Next steps\n\n1. **Merge** the PR.\n   - Keep `head_sha` unchanged.\n\n> Verify production first.\n\n```python\nprint("<script>")\n```');
  assert.match(html,/<h1>Work review<\/h1>/);
  assert.match(html,/<h2>Next steps<\/h2>/);
  assert.match(html,/<ol>[\s\S]*<ul>/);
  assert.match(html,/<strong>Merge<\/strong>/);
  assert.match(html,/<code>head_sha<\/code>/);
  assert.match(html,/<blockquote>/);
  assert.match(html,/<pre><code class="language-python">print\(&quot;&lt;script&gt;&quot;\)/);
});

test('supports tables and CSP-compatible column alignment',()=>{
  const html=render('| PR | State |\n| :-- | --: |\n| #1 | **Approved** |');
  assert.match(html,/<div class="markdown-table"><table>/);
  assert.match(html,/<th class="markdown-align-right">State/);
  assert.doesNotMatch(html,/style=/);
});

test('supports read-only tasks without changing ordinary bracket text',()=>{
  const html=render('- [x] Complete\n- [ ] Check CI\n\n[x] This is a paragraph.');
  assert.match(html,/<input type="checkbox" disabled checked/);
  assert.match(html,/<input type="checkbox" disabled aria-label="Incomplete task">/);
  assert.match(html,/<p>\[x\] This is a paragraph\./);
});

test('links preserve human labels, support bare URLs, and isolate new tabs',()=>{
  const html=render('[The PR](https://github.com/acme/api/pull/1) and https://linear.app/acme/issue/ENG-1');
  assert.match(html,/<a href="https:\/\/github.com\/acme\/api\/pull\/1" target="_blank" rel="noopener noreferrer">The PR<\/a>/);
  assert.match(html,/<a href="https:\/\/linear.app\/acme\/issue\/ENG-1"/);
});

test('escapes source HTML and never renders remote images',()=>{
  const html=render('<script>alert(1)</script>\n<img src=x onerror="alert(1)">\n\n![Private diagram](https://tracker.example/pixel.png)');
  assert.doesNotMatch(html,/<script|<img|<iframe|<svg/);
  assert.match(html,/&lt;script&gt;/);
  assert.match(html,/Private diagram/);
  assert.doesNotMatch(html,/tracker\.example/);
});

test('rejects executable and local targets and does not expose URL credentials in links',()=>{
  for(const url of ['javascript:alert(1)','jav&#x61;script:alert(1)','data:text/html,evil','file:///etc/passwd','/api/export','//tracker.example/path']){
    assert.doesNotMatch(render(`[click](${url})`),/<a\b/,url);
  }
  assert.doesNotMatch(render('[private](https://user:secret@example.com)'),/href="[^"]*(?:secret|@)/);
});

test('links top actions from verified references elsewhere in the briefing',()=>{
  const text='1. **Reply on #1419 and #1416.** Decide between ENG-1156 and ENG-1211.\n2. **Merge Model#1327.**\n\nSources: [server PR](https://github.com/acme/api/pull/1419), [other PR](https://github.com/acme/api/pull/1416), [model PR](https://github.com/acme/Model/pull/1327), [ticket](https://linear.app/acme/issue/ENG-1211/deletion).';
  const html=render(text);
  assert.match(html,/<strong>Reply on <a href="https:\/\/github.com\/acme\/api\/pull\/1419"[^>]*>#1419<\/a>/);
  assert.match(html,/<a href="https:\/\/github.com\/acme\/api\/pull\/1416"[^>]*>#1416<\/a>/);
  assert.match(html,/<a href="https:\/\/github.com\/acme\/model\/pull\/1327"[^>]*>Model#1327<\/a>/);
  assert.match(html,/<a href="https:\/\/linear.app\/acme\/issue\/ENG-1211"[^>]*>ENG-1211<\/a>/);
  assert.doesNotMatch(html,/<a[^>]*>ENG-1156<\/a>/);
});

test('uses cached work, updates cached renders as references arrive, and skips code',()=>{
  const text='ENG-8 and api#7 and `#7`.\n\n```text\nENG-8 api#7\n```';
  assert.doesNotMatch(render(text),/<a\b/);
  const items=[{url:'https://linear.app/acme/issue/ENG-8/valid'},{url:'https://github.com/acme/api/pull/7'}];
  const html=context.renderBriefingMarkdown(text,items);
  assert.match(html,/<a href="https:\/\/linear.app\/acme\/issue\/ENG-8"/);
  assert.match(html,/<a href="https:\/\/github.com\/acme\/api\/pull\/7"/);
  assert.match(html,/<code>#7<\/code>/);
  assert.match(html,/<pre><code class="language-text">ENG-8 api#7/);
});

test('does not guess ambiguous PR numbers or nest links',()=>{
  const items=[{url:'https://github.com/acme/api/pull/7'},{url:'https://github.com/acme/web/pull/7'}];
  const html=context.renderBriefingMarkdown('Unqualified #7. Qualified api#7. [existing API #7](https://github.com/acme/api/pull/7) [existing web #7](https://github.com/acme/web/pull/7)',items);
  assert.match(html,/<p>Unqualified #7\./);
  assert.equal((html.match(/<a\b/g)||[]).length,3);
  assert.doesNotMatch(html,/<a[^>]*>[^<]*<a/);
});
