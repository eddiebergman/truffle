import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import newsletters
import server
import agents


class NewsletterTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve();self.data=self.root/'data';self.data.mkdir()
        self.repo=self.root/'project';self.repo.mkdir();(self.repo/'SKILL.md').write_text('Read-only daily report.')
        self.reports=self.repo/'reports';run=self.reports/'2026-10-20';run.mkdir(parents=True)
        self.report=run/'report.html';self.report.write_text('<h1>Daily</h1><script>document.title="report"</script>')
        self.config=[dict(id='daily',title='Daily',cwd=str(self.repo),skill='SKILL.md',reports='reports',agent='claude',command='/daily')]
        (self.data/'newsletters.json').write_text(json.dumps(self.config))
        self.presets=newsletters.presets(self.data)
        self.state=dict(artifacts=[],sessions=[dict(id='one',cwd=str(self.repo)),dict(id='two',cwd=str(self.repo))])

    def test_discovers_reports_without_raw_data_or_source_scan(self):
        (self.report.parent/'core.json').write_text('{}');(self.report.parent/'report.src.html').write_text('draft')
        rows=newsletters.catalog(self.state,self.presets)['artifacts']
        self.assertEqual(len(rows),1);self.assertEqual(rows[0]['sessionIds'],[])
        self.assertEqual(rows[0]['path'],str(self.report));self.assertTrue(rows[0]['available'])

    def test_link_multiple_sessions_merge_and_reversible_replacement(self):
        artifact=newsletters.catalog(self.state,self.presets)['artifacts'][0]
        newsletters.save(self.state,dict(id=artifact['id'],sessionIds=['one']),self.presets)
        newsletters.save(self.state,dict(path=str(self.report),sessionIds=['two']),self.presets)
        self.assertEqual(self.state['artifacts'][0]['sessionIds'],['one','two'])
        newsletters.save(self.state,dict(id=artifact['id'],sessionIds=[],replaceSessions=True),self.presets)
        self.assertEqual(self.state['artifacts'][0]['sessionIds'],[])
        self.assertTrue(self.report.exists())

    def test_unknown_session_and_unsafe_url_are_rejected(self):
        for payload in [dict(path=str(self.report),sessionIds=['unknown']),dict(url='javascript:alert(1)'),dict(url='https://user:secret@example.com/a')]:
            with self.assertRaises(ValueError):newsletters.save(self.state,payload,self.presets)
        self.assertFalse(self.state['artifacts'])

    def test_symlink_escape_hidden_raw_and_oversized_file_are_rejected(self):
        outside=self.root/'secret.html';outside.write_text('secret')
        link=self.report.parent/'escaped.html';link.symlink_to(outside)
        hidden=self.repo/'.private';hidden.mkdir();secret=hidden/'report.html';secret.write_text('secret')
        raw=self.repo/'raw.json';raw.write_text('{}')
        large=self.repo/'big.html'
        with large.open('wb') as output:output.truncate(newsletters.MAX_FILE+1)
        for path in [outside,link,secret,raw,large]:
            with self.assertRaises(ValueError):newsletters.file_path(path,[self.repo])

    def test_presets_reject_escape_and_duplicate_id(self):
        self.config[0]['skill']='../elsewhere/SKILL.md'
        (self.data/'newsletters.json').write_text(json.dumps(self.config))
        with self.assertRaises(ValueError):newsletters.presets(self.data)
        self.config[0]['skill']='SKILL.md';self.config.append(self.config[0])
        (self.data/'newsletters.json').write_text(json.dumps(self.config))
        with self.assertRaises(ValueError):newsletters.presets(self.data)

    def test_existing_remote_artifact_dedup_and_missing_local_file(self):
        first=newsletters.save(self.state,dict(url='https://claude.ai/artifacts/existing',sessionIds=['one']),self.presets)
        second=newsletters.save(self.state,dict(url=first['url'],sessionIds=['two']),self.presets)
        self.assertEqual(first['id'],second['id']);self.assertEqual(second['sessionIds'],['one','two'])
        saved=newsletters.save(self.state,dict(path=str(self.report),sessionIds=['one']),self.presets)
        self.report.unlink()
        row=next(x for x in newsletters.catalog(self.state,self.presets)['artifacts'] if x['id']==saved['id'])
        self.assertFalse(row['available'])

    def test_prepare_register_save_and_recovery_preserve_workflow(self):
        store=server.Store(self.data/'state.json',demo=True)
        self.addCleanup(store.file_lock.close)
        record=store.prepare_newsletter(dict(presetId='daily'))
        self.assertEqual(record['cwd'],str(self.repo));self.assertEqual(record['workflow'],'newsletter')
        self.assertEqual(store.prepare_newsletter(dict(presetId='daily'))['id'],record['id'])
        packet=agents.context_packet(store.session_context(record['id']))
        self.assertIn(str(self.repo/'SKILL.md'),packet);self.assertIn('artifact --record',packet)
        self.assertNotIn('If no work is linked yet, wait',packet)
        store.register_agent(dict(id=record['id'],agent='claude',sessionId=record['sessionId'],cwd=str(self.repo)))
        self.assertEqual(store.state['sessions'][-1]['newsletterId'],'daily')
        store.register_agent(dict(id=record['id'],operation='save-artifact',path=str(self.report),title='Finished'))
        self.assertEqual(store.state['artifacts'][0]['sessionIds'],[record['id']])
        self.assertIn(str(self.report),agents.context_packet(store.session_context(record['id'])))
        recovered=store.recover_session(dict(id=record['id']))
        self.assertEqual(recovered['newsletterId'],'daily')
        body,kind,_=store.artifact_file(store.state['artifacts'][0]['id'])
        self.assertIn(b'<h1>Daily',body);self.assertTrue(kind.startswith('text/html'))
        with self.assertRaises(ValueError):store.artifact_file('../../secret')

    def test_api_artifact_headers_isolate_scripts_and_host(self):
        import threading
        from urllib.request import Request,urlopen
        from urllib.error import HTTPError
        store=server.Store(self.data/'state.json',demo=True)
        self.addCleanup(store.file_lock.close)
        http=server.ThreadingHTTPServer(('127.0.0.1',0),server.Handler)
        http.authority=f'127.0.0.1:{http.server_port}';http.store=store;http.csrf='test'
        thread=threading.Thread(target=http.serve_forever,daemon=True);thread.start()
        self.addCleanup(http.server_close);self.addCleanup(http.shutdown)
        base='http://'+http.authority
        with urlopen(base+'/api/library') as response:artifact=json.load(response)['artifacts'][0]
        with urlopen(base+'/artifacts/'+artifact['id']) as response:
            csp=response.headers['Content-Security-Policy'];self.assertIn('sandbox allow-scripts',csp)
            self.assertIn("connect-src 'none'",csp);self.assertNotIn('allow-same-origin',csp)
            self.assertNotIn('X-Frame-Options',response.headers)
        with self.assertRaises(HTTPError) as caught:urlopen(Request(base+'/api/library',headers={'Host':'evil.test'}))
        self.assertEqual(caught.exception.code,403)

if __name__=='__main__':unittest.main()
