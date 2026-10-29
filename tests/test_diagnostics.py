import io
import json
from pathlib import Path
import socket
import sys
import tempfile
import os
import subprocess
import threading
from urllib.request import Request,urlopen
import unittest
from unittest.mock import patch,Mock
from urllib.error import HTTPError,URLError

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import server
import sentry_source as sentry
from sync_diagnostics import DiagnosticLog,failure_fields


class DiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.log=DiagnosticLog(self.temp.name,max_bytes=100)
        config=patch.object(sentry,'settings',return_value=('example','private-token','https://sentry.io','90d'))
        config.start();self.addCleanup(config.stop)

    def test_failure_retains_page_and_timeout_but_not_exception_secret(self):
        first=dict(id='12',title='Failure',permalink='https://example.sentry.io/issues/12/',project={'slug':'api'})
        initial=io.BytesIO(json.dumps([first]).encode());initial.status=200
        initial.headers={'Link':'<https://sentry.io/>; rel="next"; results="true"; cursor="private-cursor"'}
        opener=Mock();opener.open.side_effect=[initial,URLError(TimeoutError('private-token private-payload'))]
        with patch.object(sentry,'build_opener',return_value=opener):
            with self.assertRaises(server.SyncError) as caught:sentry.fetch(server)
        exc=caught.exception
        self.assertEqual(exc.diagnostic['kind'],'timeout')
        self.assertEqual(exc.diagnostic['page'],2)
        self.assertEqual(exc.diagnostic['receivedItems'],1)
        self.log.write(source='sentry',outcome='failure',**failure_fields(exc))
        result=self.log.path.read_text()
        self.assertNotIn('private-token',result)
        self.assertNotIn('private-payload',result)
        self.assertNotIn('private-cursor',result)
        self.assertIn('TimeoutError',result)
        self.assertIn('timed out',str(exc))

    def test_dns_and_http_failure_are_distinct(self):
        opener=Mock();opener.open.side_effect=URLError(socket.gaierror(-2,'private-token'))
        with patch.object(sentry,'build_opener',return_value=opener):
            with self.assertRaises(server.SyncError) as caught:sentry.get('/api/0/test/',{},server)
        self.assertEqual(caught.exception.diagnostic['kind'],'dns')
        self.assertEqual(caught.exception.diagnostic['errno'],-2)
        opener.open.side_effect=HTTPError('https://sentry.io/',429,'private-token',{'Retry-After':'120'},None)
        with patch.object(sentry,'build_opener',return_value=opener):
            with self.assertRaises(server.SyncError) as caught:sentry.get('/api/0/test/',{},server)
        self.assertEqual(caught.exception.retry_after,120)
        self.assertEqual(caught.exception.diagnostic['httpStatus'],429)
        self.assertNotIn('private-token',str(caught.exception))

    def test_invalid_response_body_is_not_logged(self):
        response=io.BytesIO(b'private-response-content');response.status=200;response.headers={}
        opener=Mock();opener.open.return_value=response
        with patch.object(sentry,'build_opener',return_value=opener):
            with self.assertRaises(server.SyncError) as caught:sentry.get('/api/0/test/',{},server)
        self.log.write(**failure_fields(caught.exception))
        self.assertIn('invalid-json',self.log.path.read_text())
        self.assertNotIn('private-response-content',self.log.path.read_text())

    def test_logs_rotate_and_remain_private(self):
        for _ in range(12):self.log.write(source='sentry',outcome='success',elapsedMs=10,Authorization='private-token')
        files=list(self.log.path.parent.iterdir())
        self.assertEqual(len(files),3)
        for path in files:
            self.assertEqual(path.stat().st_mode&0o777,0o600)
            self.assertNotIn('private-token',path.read_text())
            for line in path.read_text().splitlines():self.assertIn('time',json.loads(line))

    def test_reader_handles_rotation_and_malformed_lines_without_exposing_extra_fields(self):
        log=DiagnosticLog(self.temp.name,max_bytes=100)
        for count in range(8):log.write(source='github',operation='sync',outcome='failure',page=count)
        log.path.write_text(log.path.read_text()+'broken JSON\n'+json.dumps({'time':'2026-10-20T00:00:00+00:00','source':'github','Authorization':'private-token'})+'\n')
        entries=log.read()
        self.assertEqual(entries[1]['page'],7)
        self.assertEqual(len(log.read(limit=2)),2)
        self.assertNotIn('private-token',json.dumps(entries))

    def test_github_cli_timeout_retains_deadline_without_command_output(self):
        timeout=subprocess.TimeoutExpired(['gh','private-token'],35,output='private-response',stderr='private-token')
        with patch.dict(os.environ,{'GITHUB_TOKEN':''}),patch('server.subprocess.run',side_effect=timeout):
            with self.assertRaises(server.SyncError) as caught:server.github_graphql('private-query')
        fields=failure_fields(caught.exception)
        self.assertEqual(fields['kind'],'timeout')
        self.assertEqual(fields['transport'],'gh-cli')
        self.assertEqual(fields['timeoutSeconds'],35)
        self.assertEqual(fields['reasonType'],'TimeoutExpired')
        self.log.write(**fields)
        self.assertNotIn('private',self.log.path.read_text())

    def test_cli_stderr_is_classified_without_being_logged(self):
        result=Mock(returncode=1,stderr='gh: HTTP 429 rate limit exceeded private-token',stdout='private-response')
        with patch.dict(os.environ,{'GITHUB_TOKEN':''}),patch('server.subprocess.run',return_value=result):
            with self.assertRaises(server.SyncError) as caught:server.github_graphql('private-query')
        self.assertEqual(caught.exception.diagnostic['kind'],'rate-limit')
        self.assertEqual(caught.exception.diagnostic['httpStatus'],429)
        self.log.write(**failure_fields(caught.exception))
        self.assertNotIn('private',self.log.path.read_text())

    def test_http_token_transport_preserves_timeout_cause(self):
        with patch('server.urlopen',side_effect=URLError(TimeoutError('private-token'))):
            with self.assertRaises(server.SyncError) as caught:server.request_json('https://api.github.com/graphql',{},'private-token')
        self.assertEqual(caught.exception.diagnostic['transport'],'http')
        self.assertEqual(caught.exception.diagnostic['kind'],'timeout')
        self.assertNotIn('private-token',str(caught.exception.diagnostic))

    def test_local_log_endpoint_is_host_checked_and_does_not_return_workspace_state(self):
        store=server.Store(Path(self.temp.name)/'state.json')
        store.diagnostics.write(source='github',outcome='failure',kind='timeout',timeoutSeconds=35)
        http=server.ThreadingHTTPServer(('127.0.0.1',0),server.Handler)
        http.authority=f'127.0.0.1:{http.server_port}';http.store=store
        thread=threading.Thread(target=http.serve_forever,daemon=True);thread.start()
        try:
            with urlopen('http://'+http.authority+'/api/logs') as response:
                data=json.load(response)
                self.assertEqual(data['entries'][0]['timeoutSeconds'],35)
                self.assertNotIn('Access-Control-Allow-Origin',response.headers)
                self.assertNotIn('items',data)
            request=Request('http://'+http.authority+'/api/logs',headers={'Host':'foreign.example'})
            with self.assertRaises(HTTPError) as caught:urlopen(request)
            self.assertEqual(caught.exception.code,403)
            caught.exception.close()
        finally:http.shutdown();http.server_close();thread.join();store.file_lock.close()

    def test_sync_records_failure_and_keeps_cache(self):
        store=server.Store(Path(self.temp.name)/'state.json')
        try:
            store.state['items']=[{'id':'sentry:example:12','source':'sentry','title':'Cached'}]
            error=server.SyncError('Sentry request timed out.',diagnostic={'kind':'timeout','page':4})
            def fetch():raise error
            store.sync('sentry',fetch)
            self.assertEqual(store.state['items'][0]['title'],'Cached')
            record=json.loads(store.diagnostics.path.read_text().splitlines()[-1])
            self.assertEqual(record['page'],4)
            self.assertEqual(record['outcome'],'failure')
            self.assertEqual(record['retrySeconds'],300)
        finally:store.file_lock.close()


if __name__=='__main__':unittest.main()
