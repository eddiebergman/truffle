import json
import os
import subprocess
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import server


class DashboardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / 'state.json'
        self.store = server.Store(self.path)

    def tearDown(self):
        self.store.file_lock.close()
        self.temp.cleanup()

    def test_note_pin_override_and_restart(self):
        self.store.edit(dict(action='save-note', title='Persistent note', url='https://example.com/task', status='blocked'))
        note = self.store.state['notes'][0]
        self.store.edit(dict(action='override', id=note['id'], value=dict(pinned=True)))
        self.store.file_lock.close()
        reloaded = server.Store(self.path)
        try:
            self.assertEqual(reloaded.state['notes'][0]['title'], 'Persistent note')
            self.assertTrue(reloaded.state['overrides'][note['id']]['pinned'])
            self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        finally:
            reloaded.file_lock.close()

    def test_invalid_edit_does_not_change_state(self):
        before = self.store.snapshot()
        for url in ('javascript:alert(1)', 'file:///etc/passwd', 'https://user:secret@example.com'):
            with self.assertRaises(ValueError):
                self.store.edit(dict(action='save-note', title='Bad link', url=url))
        self.assertEqual(self.store.snapshot(), before)

    def test_failure_preserves_cache_and_applies_persistent_backoff(self):
        old = dict(id='github:1', source='github', title='Old item')
        self.store.state['items'] = [old]
        def fail():
            raise server.SyncError('Rate limited', 1200)
        self.store.sync('github', fail)
        self.assertEqual(self.store.state['items'], [old])
        info = self.store.state['sources']['github']
        self.assertGreater(info['nextAttempt'], time.time() + 1190)
        self.assertEqual(json.loads(self.path.read_text())['sources']['github']['failures'], 1)
        with patch.object(server, 'fetch_github') as fetch:
            self.store.state['sources']['linear']['nextAttempt'] = time.time() + 300
            self.store.schedule(manual=True)
            fetch.assert_not_called()

    def test_force_refresh_skips_interval_preserves_backoff_and_prevents_overlap(self):
        for info in self.store.state['sources'].values():
            info.update(nextAttempt=1300, failures=0)
        self.store.state['sources']['linear']['failures'] = 1
        with patch('server.time.time', return_value=1000), patch('server.threading.Thread') as thread:
            self.assertEqual(self.store.schedule(manual=True), [])
            self.assertEqual(self.store.schedule(manual=True, force=True), ['github'])
            self.assertEqual(self.store.state['sources']['linear']['nextAttempt'], 1300)
            self.assertEqual(thread.call_count, 1)
            with self.assertRaises(ValueError):
                self.store.schedule(manual=True, force=True)
        with patch('server.time.time', return_value=1016), patch('server.threading.Thread') as thread:
            self.assertEqual(self.store.schedule(manual=True, force=True), [])
            thread.assert_not_called()
        self.store.running.clear()
        with patch('server.time.time', return_value=1301), patch('server.threading.Thread'):
            self.assertEqual(self.store.schedule(manual=True, force=True), ['github', 'linear'])

    def test_sync_replaces_only_its_source_and_keeps_notes_and_overrides(self):
        self.store.state['items'] = [dict(id='github:old', source='github'), dict(id='linear:1', source='linear')]
        self.store.edit(dict(action='save-note', title='Local work'))
        self.store.state['overrides'] = {'github:new': {'status': 'blocked', 'pinned': True}}
        with patch('work_links.refresh', return_value=[]):
            self.store.sync('github', lambda: [dict(id='github:new', source='github')])
        self.assertEqual({x['id'] for x in self.store.state['items']}, {'github:new', 'linear:1'})
        self.assertEqual(len(self.store.state['notes']), 1)
        self.assertEqual(self.store.state['overrides']['github:new']['status'], 'blocked')

    def test_single_writer_lock(self):
        with self.assertRaisesRegex(ValueError, 'Another truffle'):
            server.Store(self.path)

    def test_workflow_mapping(self):
        self.assertEqual(server.status_for('In Review', 'started'), 'needs-review')
        self.assertEqual(server.status_for('In Progress', 'started', ['blocked']), 'blocked')
        self.assertEqual(server.status_for('Complete', 'completed'), 'done')
        self.assertEqual(server.status_for('Duplicate', 'duplicate'), 'done')

    def test_linear_related_scope_paginates_and_keeps_projectless_triage(self):
        def issue(key, **updates):
            node = dict(id=key, identifier='ENG-'+key, title=key, description='',
                url='https://linear.app/acme/issue/ENG-'+key, updatedAt='2026-10-20',
                team=dict(id='team', key='ENG', name='Engineering'), project=None,
                state=dict(name='Triage', type='triage'), labels=dict(nodes=[]),
                assignee=None, creator=None, subscribers=dict(nodes=[]), comments=dict(nodes=[]))
            return {**node, **updates}
        mine = issue('1', assignee=dict(id='me'), creator=dict(id='me'))
        subscribed = issue('2', subscribers=dict(nodes=[dict(id='me')]))
        commented = issue('3', comments=dict(nodes=[dict(id='comment')]), project=dict(id='p', name='Project'))
        replies = [dict(viewer=dict(id='me')),
                   dict(issues=dict(nodes=[mine, subscribed], pageInfo=dict(hasNextPage=True, endCursor='next'))),
                   dict(issues=dict(nodes=[subscribed, commented], pageInfo=dict(hasNextPage=False, endCursor=None)))]
        with patch.dict(os.environ, LINEAR_API_KEY='test'), patch.object(server, 'request_json', side_effect=replies) as request:
            items = server.fetch_linear()
        self.assertEqual(len(items), 3)
        self.assertEqual(items[0]['relations'], ['assigned', 'created'])
        self.assertEqual(items[1]['relations'], ['subscribed'])
        self.assertEqual(items[2]['relations'], ['commented'])
        self.assertEqual(items[0]['project'], 'No project')
        self.assertEqual(items[0]['sourceStatus'], 'Triage')
        self.assertEqual(items[0]['status'], 'todo')
        self.assertEqual(items[2]['projectId'], 'p')
        query = request.call_args.args[1]['query']
        self.assertIn('after: "next"', query)
        for field in ['assignee:', 'creator:', 'subscribers:', 'comments:']:
            self.assertIn(field, query)
        self.assertNotIn('project:', query)
        self.assertNotIn('"triage"', query)

    def test_linear_scope_upgrade_preserves_connection_backoff(self):
        self.store.state['sources']['linear'].update(failures=1, nextAttempt=9999999999)
        self.store.state['sources']['linear'].pop('relationshipScopeVersion', None)
        self.store.save();self.store.file_lock.close()
        self.store = server.Store(self.path)
        self.assertEqual(self.store.state['sources']['linear']['nextAttempt'], 9999999999)

    def test_github_query_pagination_and_deduplication(self):
        node = dict(id='P1', title='PR', url='https://github.com/test/repo/pull/1', number=1,
            updatedAt='2026-10-18', isDraft=True, reviewDecision=None,
            repository={'nameWithOwner': 'test/repo'}, labels={'nodes': []})
        calls = []
        def graphql(query):
            calls.append(query)
            if 'viewer' in query:
                return {'viewer': {'login': 'test'}}
            more = len(calls) == 2
            return {'search': {'nodes': [node], 'pageInfo': {'hasNextPage': more, 'endCursor': 'next'}}}
        with patch.object(server, 'github_graphql', side_effect=graphql):
            items = server.fetch_github()
        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]['status'], 'draft')
        self.assertIn('reviews', items[0]['relations'])
        self.assertIn('after: "next"', calls[2])

    def test_ready_to_merge_requires_approval_passing_ci_and_clean_merge(self):
        cases = [
            ('APPROVED', 'SUCCESS', 'MERGEABLE', 'CLEAN', False, [], 'ready-to-merge'),
            ('APPROVED', 'FAILURE', 'MERGEABLE', 'CLEAN', False, [], 'in-progress'),
            ('APPROVED', 'PENDING', 'MERGEABLE', 'CLEAN', False, [], 'in-progress'),
            ('APPROVED', 'SUCCESS', 'MERGEABLE', 'UNKNOWN', False, [], 'in-progress'),
            ('APPROVED', 'SUCCESS', 'CONFLICTING', 'DIRTY', False, [], 'in-progress'),
            ('REVIEW_REQUIRED', 'SUCCESS', 'MERGEABLE', 'CLEAN', False, [], 'needs-review'),
            ('APPROVED', 'SUCCESS', 'MERGEABLE', 'CLEAN', True, [], 'draft'),
            ('APPROVED', 'SUCCESS', 'MERGEABLE', 'CLEAN', False, ['blocked'], 'blocked'),
        ]
        for review, ci, mergeable, merge_state, draft, labels, expected in cases:
            with self.subTest(review=review, ci=ci, merge_state=merge_state, draft=draft, labels=labels):
                node = dict(id='P1', title='PR', url='https://github.com/test/repo/pull/1', number=1,
                    updatedAt='2026-10-18', isDraft=draft, reviewDecision=review,
                    repository={'nameWithOwner': 'test/repo'}, labels={'nodes': [{'name': x} for x in labels]},
                    mergeable=mergeable, mergeStateStatus=merge_state,
                    commits={'nodes': [{'commit': {'statusCheckRollup': {'state': ci,
                        'contexts': {'totalCount': 1, 'nodes': [{'name': 'tests', 'status': 'COMPLETED',
                            'conclusion': 'SUCCESS', 'detailsUrl': 'https://github.com/test/check'}]}}}}]})
                def graphql(query):
                    if 'viewer' in query:
                        return {'viewer': {'login': 'test'}}
                    return {'search': {'nodes': [node], 'pageInfo': {'hasNextPage': False}}}
                with patch.object(server, 'github_graphql', side_effect=graphql):
                    items = server.fetch_github()
                self.assertEqual(items[0]['status'], expected)
                self.assertEqual(items[0]['ci']['total'], 1)

    def test_ci_rollup_preserves_aggregate_and_individual_results(self):
        node = {'commits': {'nodes': [{'commit': {'statusCheckRollup': {
            'state': 'FAILURE', 'contexts': {'totalCount': 110, 'nodes': [
                {'name': 'lint', 'status': 'COMPLETED', 'conclusion': 'SKIPPED', 'detailsUrl': None},
                {'name': 'tests', 'status': 'IN_PROGRESS', 'conclusion': None, 'detailsUrl': None},
                {'context': 'legacy', 'state': 'FAILURE', 'targetUrl': 'https://example.com/check'},
            ]}}}}]}}
        result = server.parse_ci(node)
        self.assertEqual(result['state'], 'failed')
        self.assertEqual(result['total'], 110)
        self.assertEqual([x['state'] for x in result['checks']], ['skipped', 'pending', 'failed'])
        self.assertEqual(server.parse_ci({'commits': {'nodes': []}})['state'], 'none')

    def test_http_security_and_persistence(self):
        http = server.ThreadingHTTPServer(('127.0.0.1', 0), server.Handler)
        http.authority = f'127.0.0.1:{http.server_port}'
        http.csrf = 'test-csrf'
        http.agent_token = 'private-test-token'
        http.store = self.store
        thread = threading.Thread(target=http.serve_forever, daemon=True)
        thread.start()
        base = 'http://' + http.authority
        def post(origin=base, token='test-csrf', host=None):
            headers = {'Content-Type': 'application/json', 'Origin': origin, 'X-CSRF-Token': token}
            if host:
                headers['Host'] = host
            request = Request(base + '/api/edit', data=json.dumps(dict(action='save-note', title='<script>alert(1)</script>')).encode(), headers=headers)
            return urlopen(request)
        try:
            with urlopen(base + '/api/health') as response:
                self.assertEqual(json.load(response), {'service':'truffle','apiVersion':1,'storageVersion':1})
                self.assertIsNone(response.headers.get('Access-Control-Allow-Origin'))
            with urlopen(base + '/api/state') as response:
                self.assertEqual(json.load(response)['csrf'], 'test-csrf')
                self.assertIn("frame-ancestors 'none'", response.headers['Content-Security-Policy'])
                self.assertIsNone(response.headers.get('Access-Control-Allow-Origin'))
            for kwargs in ({'origin': 'https://evil.example'}, {'token': ''}, {'host': 'evil.example'}):
                with self.assertRaises(HTTPError) as error:
                    post(**kwargs)
                self.assertEqual(error.exception.code, 403)
                error.exception.close()
            with post() as response:
                self.assertEqual(response.status, 200)
                self.assertEqual(json.load(response)['notes'][0]['title'], '<script>alert(1)</script>')
            register_body = dict(agent='codex', sessionId='11111111-1111-4111-8111-111111111111',
                                 cwd=self.temp.name, project='Agent-linked project')
            for headers in ({'Authorization':'Bearer wrong'},
                            {'Authorization':'Bearer private-test-token', 'Origin':'https://evil.example'}):
                request = Request(base + '/api/agent/register', data=json.dumps(register_body).encode(),
                                  headers={**headers, 'Content-Type':'application/json'})
                with self.assertRaises(HTTPError) as error:
                    urlopen(request)
                self.assertEqual(error.exception.code, 403)
                error.exception.close()
            runtime_dir = Path(self.temp.name) / 'data'
            runtime_dir.mkdir()
            (runtime_dir / 'runtime.json').write_text(json.dumps({'url':base, 'token':'private-test-token'}))
            result = subprocess.run([sys.executable, str(Path(server.__file__).parent / 'register.py'), 'register',
                                     '--agent', 'codex', '--project', 'Agent-linked project', '--cwd', self.temp.name],
                                    env={**os.environ, 'TRUFFLE_HOME':self.temp.name,
                                         'CODEX_THREAD_ID':'11111111-1111-4111-8111-111111111111'},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('Registered 1 linked items.', result.stdout)
            self.assertNotIn('private-test-token', result.stdout + result.stderr)
            self.assertEqual(self.store.state['sessions'][0]['items'][0]['title'], 'Agent-linked project')
            with self.assertRaises(HTTPError) as error:
                urlopen(base + '/../.env')
            self.assertEqual(error.exception.code, 404)
            error.exception.close()
            with urlopen(base + '/') as response:
                self.assertEqual(response.status, 200)
        finally:
            http.shutdown()
            http.server_close()
            thread.join()


if __name__ == '__main__':
    unittest.main()
