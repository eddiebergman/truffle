import json
from pathlib import Path
import shlex
import sys
import tempfile
import threading
import time
import shutil
import subprocess
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import agents
import agent_inbox
import server
import work_links
import work_review

SID = '11111111-1111-4111-8111-111111111111'
RID = '22222222-2222-4222-8222-222222222222'


class SessionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.store = server.Store(Path(self.temp.name) / 'state.json')
        self.store.edit(dict(action='save-note', title='OpenAPI', status='todo'))
        self.item = self.store.state['notes'][0]

    def tearDown(self):
        self.store.file_lock.close()
        self.temp.cleanup()

    def save_session(self, **updates):
        payload = dict(title='OpenAPI work', agent='claude', cwd=self.temp.name,
                       itemIds=[self.item['id']], sessionId=SID, destination='cmux')
        payload.update(updates)
        self.store.edit(dict(action='save-session', **payload))
        return self.store.state['sessions'][-1]

    def test_review_covers_all_work_without_reassigning_ticket_sessions(self):
        existing = self.save_session()
        item_settings = json.loads(json.dumps(self.store.state['itemSettings']))
        old_items = list(existing['items'])
        self.store.state['overrides'][self.item['id']] = dict(hidden=True, status='blocked')
        with patch.object(server, 'request_json') as remote:
            review = self.store.prepare_review(dict(agent='codex'))
            remote.assert_not_called()
        self.assertEqual(review['items'], [])
        self.assertEqual(review['workflow'], 'work-review')
        self.assertEqual(existing['items'], old_items)
        self.assertEqual(self.store.state['itemSettings'], item_settings)
        context = self.store.session_context(review['id'])
        data = context['reviewData']
        self.assertEqual(data['items'][0]['id'], self.item['id'])
        self.assertTrue(data['items'][0]['hidden'])
        self.assertEqual(data['items'][0]['dashboardStatus'], 'blocked')
        self.assertNotIn('reviewData', self.store.snapshot()['sessions'][-1])
        self.assertIn('Do not change tickets', review['instruction'])
        self.assertIn('Sentry follow-up', review['instruction'])
        self.assertIn(work_review.LINK_INSTRUCTION, review['instruction'])
        old_context={**context, 'instruction':'Older saved review instruction'}
        self.assertIn(work_review.LINK_INSTRUCTION, agents.context_packet(old_context))
        self.assertIn('three most useful next actions', review['instruction'])
        self.assertNotIn('wait for the user', agents.context_packet(context))

    def test_review_packet_is_private_and_preserves_full_work_links(self):
        review = self.store.prepare_review(dict(agent='codex'))
        gateway = self.store.gateway
        packet = gateway.write_packet(self.store.session_context(review['id']))
        data = packet.with_suffix('.json')
        self.assertEqual(data.stat().st_mode & 0o777, 0o600)
        self.assertIn(str(data), packet.read_text())
        self.assertEqual(json.loads(data.read_text())['items'][0]['id'], self.item['id'])
        self.assertIn(review['id'], packet.read_text())
        self.assertEqual(self.store.prepare_review(dict(agent='codex'))['id'], review['id'])
        self.store.save_session(dict(id=review['id'], sessionId=SID))
        self.assertEqual(self.store.state['sessions'][-1]['workflow'], 'work-review')
        with self.assertRaisesRegex(ValueError, '30 seconds'):
            self.store.prepare_review(dict(agent='codex'))

    def test_review_snapshot_retains_completed_linked_work_and_bounds_descriptions(self):
        old = dict(id='github:old', source='github', title='Merged PR', url='https://github.com/acme/api/pull/1',
                   status='done', detail='x'*10000, sentryLinks=[dict(url='https://acme.sentry.io/issues/1/')])
        self.store.state['items'] = [old]
        self.save_session(itemIds=[old['id']])
        self.store.state['items'] = []
        self.store.state['notes'] += [{**self.item, 'id': 'note:'+str(n), 'detail': 'x'*10000} for n in range(30)]
        data = work_review.snapshot(self.store.state)
        self.assertIn('github:old', [x['id'] for x in data['items']])
        self.assertLessEqual(sum(len(x.get('detail','')) for x in data['items']), 30000)
        linked = next(x for x in data['items'] if x['id']=='github:old')
        self.assertEqual(linked['status'], 'done')
        self.assertTrue(linked['sentryLinks'])

    def test_sentry_links_require_issue_integration_and_trusted_issue_url(self):
        links = work_links.sentry_links([
            dict(sourceType='sentry', url='https://acme.sentry.io/issues/123/', title='Error'),
            dict(sourceType='sentry', url='https://acme.sentry.io/issues/123/', title='Error'),
            dict(sourceType='api', url='https://acme.sentry.io/explore/traces/trace/abc/'),
            dict(sourceType='sentry', url='https://sentry.io.evil.example/issues/456/')], server)
        self.assertEqual(links, [dict(url='https://acme.sentry.io/issues/123/', title='Error')])

    def test_project_done_archive_and_session_survive_restart(self):
        session = self.save_session()
        self.store.edit(dict(action='save-note', id=self.item['id'], title='OpenAPI', status='done'))
        self.assertEqual(self.store.snapshot()['sessions'][0]['items'][0]['status'], 'done')
        self.store.edit(dict(action='archive-project', id=self.item['id']))
        self.store.edit(dict(action='archive-session', id=session['id']))
        self.store.file_lock.close()
        reloaded = server.Store(self.store.path)
        try:
            self.assertTrue(reloaded.state['sessions'][0]['archivedAt'])
            self.assertEqual(reloaded.state['sessions'][0]['sessionId'], SID)
            self.assertTrue(reloaded.state['notes'][0]['archivedAt'])
            reloaded.edit(dict(action='archive-session', id=session['id'], restore=True))
            self.assertIsNone(reloaded.state['sessions'][0]['archivedAt'])
        finally:
            reloaded.file_lock.close()

    def test_many_items_can_share_one_conversation_and_unknown_ids_fail(self):
        self.store.state['items'] = [dict(id='linear:one', source='linear', title='Issue', status='todo')]
        self.save_session(itemIds=[self.item['id'], 'linear:one'])
        self.assertEqual(len(self.store.snapshot()['sessions'][0]['items']), 2)
        before = self.store.snapshot()
        with self.assertRaises(ValueError):
            self.save_session(itemIds=['missing'], sessionId=RID)
        self.assertEqual(self.store.snapshot(), before)
        self.save_session()
        self.assertEqual(len(self.store.state['sessions']), 1)

    def test_agent_registration_updates_same_record_and_preserves_links(self):
        session = self.save_session()
        result = self.store.register_agent(dict(id=session['id'], agent='claude', sessionId=SID, cwd=self.temp.name))
        self.assertEqual(result['id'], session['id'])
        self.assertEqual(len(self.store.state['sessions']), 1)
        self.assertEqual(result['items'][0]['id'], self.item['id'])
        with self.assertRaises(ValueError):
            self.store.register_agent(dict(id=session['id'], agent='claude', sessionId=RID, cwd=self.temp.name))
        self.assertEqual(self.store.state['sessions'][0]['sessionId'], SID)

    def test_register_creates_local_project_without_remote_calls(self):
        result = self.store.register_agent(dict(agent='codex', sessionId=SID, title='Local work',
            cwd=self.temp.name, project='New local project'))
        self.assertEqual(result['items'][0]['source'], 'notes')
        self.assertFalse(result['launchNew'])

    def test_failed_import_does_not_leave_partial_project_or_session(self):
        before = self.store.snapshot()
        with self.assertRaises(ValueError):
            self.store.prepare_session(dict(agent='claude', title='Bad', cwd=self.temp.name,
                projectName='Would be orphaned', urls=['https://evil.example/task']))
        self.assertEqual(self.store.snapshot(), before)

    def test_closed_linked_work_keeps_authoritative_done_state(self):
        self.store.state['items'] = [dict(id='github:closed', source='github', title='PR', status='todo')]
        self.save_session(itemIds=['github:closed'])
        with patch.object(work_links, 'refresh', return_value=[dict(id='github:closed', source='github', title='PR', status='done')]):
            self.store.sync('github', lambda: [])
        self.assertEqual(self.store.snapshot()['sessions'][0]['items'][0]['status'], 'done')
        session = self.store.state['sessions'][0]
        self.store.edit(dict(action='archive-session', id=session['id']))
        with patch.object(work_links, 'refresh') as refresh:
            self.store.sync('github', lambda: [])
            refresh.assert_not_called()

    def test_demo_open_has_no_native_side_effects(self):
        gateway = agents.Gateway(Path(self.temp.name), demo=True)
        with patch.object(gateway, 'command') as command:
            result = gateway.open(dict(id=RID))
        self.assertTrue(result['preview'])
        command.assert_not_called()

    def test_claude_launch_quotes_arguments_and_passes_context_as_file(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, agent='claude', destination='cmux', sessionId=SID, cwd=self.temp.name,
            title='test $(touch /tmp/must-not-run)', instruction='untrusted `shell` content', items=[self.item])
        commands = []
        def cmux_json(*args):
            commands.append(args)
            return {'workspace_id': RID, 'surface_id': SID}
        with patch.object(gateway, 'lookup', return_value=None), patch.object(gateway, 'cmux_json', side_effect=cmux_json), patch.object(gateway, 'command'), patch('shutil.which', return_value='/bin/claude'):
            result = gateway.open(session, new=True)
        launch = commands[0]
        argv = shlex.split(launch[launch.index('--command') + 1])
        self.assertEqual(argv[argv.index('--name') + 1], session['title'])
        self.assertNotIn(session['instruction'], argv[-1])
        self.assertIn('untrusted `shell` content', (Path(self.temp.name) / 'contexts' / (RID + '.md')).read_text())
        self.assertEqual(result['workspaceId'], RID)
        with self.assertRaises(agents.AgentError):
            gateway.open(session, new=True)

    def test_running_session_focuses_explicit_surface_without_launch(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, agent='claude', sessionId=SID, cwd=self.temp.name)
        candidate = dict(sessionId=SID, agent='claude', running=True, active=True, surfaceId=SID, workspaceId=RID)
        with patch.object(gateway, 'lookup', return_value=candidate), patch.object(gateway, 'cmux_json', return_value={'panels': [{'id': SID}]} ) as cmux, patch.object(gateway, 'command') as command, patch('shutil.which', return_value='/bin/cmux'):
            gateway.open(session)
        self.assertNotIn('new-workspace', str(cmux.call_args_list))
        self.assertIn('focus-panel', str(command.call_args_list))
        self.assertIn(SID, str(command.call_args_list))

    def test_attaching_pending_codex_record_clears_launch_new(self):
        pending = self.save_session(agent='codex', sessionId='')
        self.assertTrue(pending['launchNew'])
        self.store.edit(dict(action='save-session', id=pending['id'], agent='codex', sessionId=SID,
                             title=pending['title'], cwd=self.temp.name, itemIds=[self.item['id']]))
        self.assertFalse(self.store.state['sessions'][0]['launchNew'])

    def test_explicit_registration_does_not_update_other_matching_conversation(self):
        first = self.save_session()
        pending = self.save_session(agent='claude', sessionId='', title='Other work')
        before = self.store.snapshot()
        with self.assertRaises(ValueError):
            self.store.register_agent(dict(id=pending['id'], agent='claude', sessionId=SID, cwd=self.temp.name))
        self.assertEqual(self.store.snapshot(), before)
        self.assertEqual(self.store.state['sessions'][0]['id'], first['id'])

    def test_project_override_done_updates_canonical_work_status(self):
        self.save_session()
        self.store.edit(dict(action='override', id=self.item['id'], value={'status': 'done'}))
        self.assertEqual(self.store.state['notes'][0]['status'], 'done')
        self.assertEqual(self.store.snapshot()['sessions'][0]['items'][0]['status'], 'done')
        self.assertFalse(self.store.state['overrides'][self.item['id']].get('status'))
        self.assertEqual(server.status_for('Done', 'completed', ['blocked']), 'done')

    def test_context_keeps_registration_and_every_link_with_large_descriptions(self):
        session = dict(id=RID, title='Large project', cwd=self.temp.name, agent='codex',
            items=[dict(id=str(i), source='linear', title='Issue', url=f'https://linear.app/acme/issue/ENG-{i}',
                        detail='x'*10000, status='todo') for i in range(20)])
        packet = agents.context_packet(session)
        self.assertIn('--record', packet)
        self.assertIn(RID, packet)
        self.assertIn('https://linear.app/acme/issue/ENG-19', packet)
        self.assertLess(len(packet), 40000)

    def test_archived_sessions_share_context_and_keep_poll_payload_small(self):
        self.item['detail'] = 'x'*10000
        session = self.save_session()
        for i in range(100):
            self.store.state['sessions'].append({**session, 'id': str(i), 'archivedAt': 1})
        snapshot = self.store.snapshot()
        self.assertNotIn('itemArchive', snapshot)
        self.assertLess(len(json.dumps(snapshot)), 150000)
        self.assertEqual(len(self.store.session_context(session['id'])['items'][0]['detail']), 10000)
        self.assertIn('itemArchive', self.store.snapshot(export=True))

    def test_exact_lookup_uses_identity_not_broad_inventory(self):
        gateway = agents.Gateway(Path(self.temp.name))
        with patch.object(gateway, 'cmux_json', return_value={'sessions': []}) as query:
            self.assertIsNone(gateway.lookup('claude', SID))
        self.assertIn('--session', query.call_args.args)
        self.assertIn(SID, query.call_args.args)

    def test_pending_desktop_codex_does_not_create_another_chat(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, agent='codex', destination='desktop', sessionId=None, cwd=self.temp.name)
        with patch.object(gateway, 'command') as command, self.assertRaises(agents.AgentError):
            gateway.open(session, new=False)
        command.assert_not_called()

    def test_import_timeout_has_no_late_commit(self):
        before = self.store.snapshot()
        with patch.object(server, 'wait', return_value=(set(), {object()})), patch.object(work_links, 'resolve', return_value={'id':'github:x'}):
            with self.assertRaisesRegex(ValueError, 'timed out'):
                self.store.prepare_session(dict(agent='claude', title='Timeout', cwd=self.temp.name,
                    projectName='Orphan', urls=['https://github.com/acme/repo/pull/1']))
        self.assertEqual(self.store.snapshot(), before)

    def test_unavailable_source_retains_prior_context(self):
        old = dict(id='github:closed', source='github', title='PR', status='todo', detail='Important cached context')
        self.store.state['items'] = [old]
        session = self.save_session(itemIds=[old['id']])
        def unavailable(source, refs, api):
            return [{**refs[0], 'sourceUnavailable': True}]
        with patch.object(work_links, 'refresh', side_effect=unavailable):
            self.store.sync('github', lambda: [])
        self.assertEqual(self.store.session_context(session['id'])['items'][0]['detail'], 'Important cached context')
        self.assertTrue(self.store.state['items'][0]['sourceUnavailable'])

    def test_linear_archive_and_restore_flow_through_linked_session(self):
        node = dict(id='ticket', identifier='ENG-1165', title='Snapshot', description='Keep context',
                    url='https://linear.app/acme/issue/ENG-1165', updatedAt='2026-10-19',
                    archivedAt='2026-10-19T16:58:25.531Z', state=dict(name='Todo', type='unstarted'),
                    team=dict(name='Engineering', key='ENG'), labels=dict(nodes=[]))
        archived = work_links.linear_item(node, server)
        self.assertEqual(archived['sourceStatus'], 'Archived')
        self.assertTrue(archived['archivedAt'])
        old = {**archived, 'archivedAt': None, 'sourceStatus': 'Todo'}
        self.store.state['items'] = [old]
        session = self.save_session(itemIds=[old['id']])
        with patch.object(work_links, 'refresh', return_value=[archived]):
            self.store.sync('linear', lambda: [])
        linked = self.store.snapshot()['sessions'][0]['items'][0]
        self.assertEqual(linked['archivedAt'], archived['archivedAt'])
        self.assertEqual(linked['sourceStatus'], 'Archived')
        node['archivedAt'] = None
        restored = work_links.linear_item(node, server)
        self.store.sync('linear', lambda: [restored])
        self.assertIsNone(self.store.snapshot()['sessions'][0]['items'][0]['archivedAt'])
        self.assertEqual(self.store.session_context(session['id'])['items'][0]['detail'], 'Keep context')

    def test_reattachment_restores_archived_record_and_preserves_its_metadata(self):
        session = self.save_session(agent='codex', destination='desktop', instruction='Keep this instruction')
        self.store.edit(dict(action='archive-session', id=session['id']))
        self.store.edit(dict(action='save-note', title='Another project', status='todo'))
        new_item = self.store.state['notes'][-1]
        self.save_session(agent='codex', destination='cmux', instruction='', itemIds=[new_item['id']])
        current = self.store.state['sessions'][0]
        self.assertIsNone(current['archivedAt'])
        self.assertEqual(current['destination'], 'desktop')
        self.assertEqual(current['instruction'], 'Keep this instruction')
        self.assertEqual(len(current['items']), 2)

    def test_legacy_cmux_create_response_resolves_stable_ids(self):
        gateway = agents.Gateway(Path(self.temp.name))
        outputs = ['OK workspace:30\n', json.dumps({'workspace_id':RID, 'surfaces':[{'id':SID}]})]
        with patch.object(gateway, 'command', side_effect=outputs) as command, patch('shutil.which', return_value='/bin/cmux'):
            result = gateway.cmux_json('new-workspace', '--name', 'A quoted name')
        self.assertEqual(result['workspace_id'], RID)
        self.assertEqual(result['surface_id'], SID)
        self.assertIn('both', command.call_args_list[0].args[0])
        self.assertEqual(command.call_count, 2)

    def test_signed_inbox_registers_without_a_network_request(self):
        inbox = agent_inbox.Inbox('test-token')
        stopped = threading.Event()
        def worker():
            while not stopped.is_set():
                inbox.process(self.store.register_agent)
                stopped.wait(0.01)
        thread = threading.Thread(target=worker)
        thread.start()
        try:
            result = agent_inbox.submit({'inbox':str(inbox.path),'token':'test-token'},
                dict(agent='codex', sessionId=SID, title='Sandboxed', cwd=self.temp.name, project='Sandbox work'), timeout=3)
            self.assertEqual(result['linkedItems'], 1)
            self.assertEqual(self.store.state['sessions'][0]['sessionId'], SID)
        finally:
            stopped.set()
            thread.join(timeout=3)
            shutil.rmtree(inbox.path)

    def test_inbox_rejects_unsigned_requests_and_nonregular_files(self):
        inbox = agent_inbox.Inbox('test-token')
        try:
            nonce = SID
            envelope = {'data':{'nonce':nonce,'time':time.time(),'payload':{}},'signature':'wrong'}
            agent_inbox.private_write(inbox.path / (nonce + '.request.json'), envelope)
            with patch.object(self.store,'register_agent') as callback:
                inbox.process(callback)
                callback.assert_not_called()
            self.assertFalse((inbox.path / (nonce + '.request.json')).exists())
        finally:
            shutil.rmtree(inbox.path)

    def test_known_terminal_without_lifecycle_hooks_focuses_instead_of_duplicate_resume(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, agent='codex', destination='cmux', sessionId=SID, cwd=self.temp.name, workspaceId=RID,surfaceId=SID)
        for candidate in [None, dict(running=False, lifecycle='closed'), dict(running=True, active=False, lifecycle='idle')]:
            with self.subTest(candidate=candidate), patch.object(gateway,'lookup',return_value=candidate), patch.object(gateway,'cmux_json',return_value={'surfaces':[{'id':SID}]}) as query, patch.object(gateway,'command') as command, patch('shutil.which',return_value='/bin/cmux'):
                gateway.opened_at.clear()
                gateway.open(session)
            self.assertNotIn('new-workspace', str(query.call_args_list))
            self.assertIn('focus-panel', str(command.call_args_list))

    def test_missing_saved_workspace_resumes_same_closed_chat(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, title='Saved chat', agent='codex', destination='cmux', sessionId=SID,
                       cwd=self.temp.name, workspaceId=RID, surfaceId=SID)
        for candidate in [None, dict(running=False, restorable=True)]:
            with self.subTest(candidate=candidate), patch.object(gateway, 'lookup', return_value=candidate), patch.object(gateway, 'cmux_json', side_effect=[agents.CmuxTargetMissing('gone'), {'workspace_id': RID}]) as cmux, patch.object(gateway, 'command'), patch('shutil.which', return_value='/bin/codex'):
                gateway.opened_at.clear()
                gateway.open(session)
                launch = cmux.call_args.args
                self.assertEqual(launch[0], 'new-workspace')
                argv = shlex.split(launch[launch.index('--command') + 1])
                self.assertEqual(argv[:3], ['/bin/codex', 'resume', SID])

    def test_recovery_preserves_original_and_context_with_fresh_chat_identity(self):
        original = self.save_session(instruction='Use the linked work as context.')
        before = json.loads(json.dumps(original))
        recovered = self.store.recover_session({'id': original['id']})
        self.assertNotEqual(recovered['id'], original['id'])
        self.assertNotEqual(recovered['sessionId'], original['sessionId'])
        self.assertTrue(recovered['launchNew'])
        self.assertEqual(recovered['items'], original['items'])
        self.assertEqual(recovered['instruction'], original['instruction'])
        self.assertIsNone(recovered['workspaceId'])
        self.assertEqual(self.store.session_context(original['id'])['sessionId'], SID)
        self.assertEqual(original, before)
        self.assertEqual(self.store.recover_session({'id': original['id']})['id'], recovered['id'])
        packet = agents.context_packet(self.store.session_context(recovered['id']))
        self.assertIn(recovered['id'], packet)
        self.assertIn('Use the linked work as context.', packet)
        with patch.object(self.store.gateway, 'open', side_effect=agents.AgentError('Cannot reach cmux')):
            with self.assertRaises(agents.AgentError):
                self.store.open_session(recovered['id'])
        self.assertEqual(self.store.recover_session({'id': original['id']})['id'], recovered['id'])

    def test_recovery_keeps_archived_reference_material_and_rejects_missing_folder(self):
        original = self.save_session(agent='codex')
        note = self.store.state['notes'].pop()
        note['detail'] = 'Reference material for recovery.'
        self.store.state['itemArchive'][note['id']] = note
        before = json.loads(json.dumps(self.store.state))
        with self.assertRaises(ValueError):
            self.store.recover_session({'id': original['id'], 'cwd': self.temp.name + '/missing'})
        self.assertEqual(before, self.store.state)
        recovered = self.store.recover_session({'id': original['id']})
        self.assertIsNone(recovered['sessionId'])
        self.assertIn('Reference material for recovery.', agents.context_packet(self.store.session_context(recovered['id'])))

    def test_missing_running_terminal_and_access_errors_never_duplicate_agent(self):
        gateway = agents.Gateway(Path(self.temp.name))
        session = dict(id=RID, agent='codex', destination='cmux', sessionId=SID,
                       cwd=self.temp.name, workspaceId=RID, surfaceId=SID)
        for failure, candidate in [(agents.AgentError('denied'), None),
                                   (agents.CmuxTargetMissing('gone'), dict(running=True, active=True, workspaceId=RID, surfaceId=SID))]:
            with self.subTest(failure=failure), patch.object(gateway, 'lookup', return_value=candidate), patch.object(gateway, 'cmux_json', side_effect=failure) as cmux, self.assertRaises(agents.AgentError):
                gateway.opened_at.clear()
                gateway.open(session)
            self.assertNotIn('new-workspace', str(cmux.call_args_list))

    def test_command_distinguishes_missing_target_from_access_denied(self):
        gateway = agents.Gateway(Path(self.temp.name))
        for stderr, expected in [('Error: not_found: Workspace not found', agents.CmuxTargetMissing),
                                 ('Error: Access denied - only processes started inside cmux can connect', agents.AgentError)]:
            result = subprocess.CompletedProcess([], 1, '', stderr)
            with patch('subprocess.run', return_value=result), self.assertRaises(expected) as raised:
                gateway.command(['/bin/cmux', 'list-panels'])
            if expected is agents.CmuxTargetMissing:
                self.assertNotIn('Restart', str(raised.exception))

    def test_inbox_cleanup_tolerates_concurrent_ack_removal(self):
        inbox = agent_inbox.Inbox('test-token')
        missing = inbox.path / 'already-consumed.result.json'
        try:
            with patch.object(Path, 'iterdir', return_value=iter([missing])):
                inbox.process(self.store.register_agent)
        finally:
            shutil.rmtree(inbox.path)

    def test_source_links_reject_other_hosts_and_credentials(self):
        for url in ['https://evil.example/issue/ENG-1', 'http://linear.app/acme/issue/ENG-1',
                    'https://secret@github.com/a/b/pull/1', 'https://github.com/a/b/tree/main']:
            with self.subTest(url=url), self.assertRaises(ValueError):
                work_links.resolve(url, server)
        with self.assertRaises(ValueError):
            agents.session_id('codex://threads/new?prompt=bad')
        self.assertEqual(agents.session_id('codex://threads/' + SID), SID)

    def test_quick_blank_session_registers_without_a_placeholder_project(self):
        folder=Path(self.temp.name)/'work';folder.mkdir()
        with patch.dict('os.environ', {'MYDASH_WORKDIR': self.temp.name}):
            session = self.store.quick_session({'agent': 'codex','folderPath':str(folder.resolve())})
        self.assertEqual(session['items'], [])
        self.assertEqual(session['destination'], 'cmux')
        self.assertEqual(session['cwd'], str(folder.resolve()))
        self.assertEqual(len(self.store.state['notes']), 1)
        registered = self.store.register_agent(dict(id=session['id'], agent='codex', sessionId=SID,
            cwd=self.temp.name, title='New Codex session'))
        self.assertFalse(registered['launchNew'])
        self.assertEqual(registered['items'], [])
        registered = self.store.register_agent(dict(id=session['id'], agent='codex', sessionId=SID,
            cwd=self.temp.name, title='OpenAPI', itemIds=[self.item['id']]))
        self.assertEqual([x['id'] for x in registered['items']], [self.item['id']])
        with self.assertRaises(agents.AgentError):
            self.store.quick_session({'agent': 'claude'})

    def test_quick_issue_launch_infers_repository_and_links_context(self):
        repo = Path(self.temp.name) / 'API'
        (repo / '.git').mkdir(parents=True)
        item = dict(id='github:one', title='Fix upload', url='https://github.com/acme/api/pull/1',
                    source='github', status='todo', reference='#1', detail='Source context')
        self.store.state['items'] = [item]
        with patch.dict('os.environ', {'MYDASH_WORKDIR': self.temp.name}):
            session = self.store.quick_session({'agent':'claude', 'itemId':item['id']})
        self.assertEqual(session['cwd'], str(repo.resolve()))
        self.assertEqual([x['id'] for x in session['items']], [item['id']])
        packet = agents.context_packet(self.store.session_context(session['id']))
        self.assertIn('Source context', packet)
        self.assertIn('Before your first reply', packet)
        self.assertIn('Do not wait for a request to use mydash', packet)
        self.assertIn('--session ' + session['sessionId'], packet)

    def test_quick_invalid_item_does_not_save_or_consume_launch_cooldown(self):
        with self.assertRaises(ValueError):
            self.store.quick_session({'agent':'claude','itemId':'missing'})
        folder=Path(self.temp.name)/'work';folder.mkdir()
        with patch.dict('os.environ', {'MYDASH_WORKDIR': self.temp.name}):
            record = self.store.quick_session({'agent':'claude','itemId':self.item['id'],'folderPath':str(folder.resolve())})
        self.assertEqual(len(self.store.state['sessions']), 1)
        self.assertEqual(record['items'][0]['id'], self.item['id'])

    def test_hidden_session_survives_registration_and_can_be_shown_again(self):
        record = self.save_session()
        self.store.edit(dict(action='hide-session', id=record['id'], hidden=True))
        self.store.register_agent(dict(id=record['id'], agent='claude', sessionId=SID, cwd=self.temp.name))
        self.store.file_lock.close()
        reloaded = server.Store(self.store.path)
        try:
            session = reloaded.snapshot()['sessions'][0]
            self.assertTrue(session['hidden'])
            self.assertEqual(session['sessionId'], SID)
            self.assertFalse(session.get('archivedAt'))
            reloaded.edit(dict(action='hide-session', id=record['id'], hidden=False))
            self.assertFalse(reloaded.snapshot()['sessions'][0]['hidden'])
            before = reloaded.snapshot()
            with self.assertRaises(ValueError):
                reloaded.edit(dict(action='hide-session', id=record['id'], hidden='false'))
            self.assertEqual(reloaded.snapshot(), before)
        finally:
            reloaded.file_lock.close()

    def test_hidden_remote_work_remains_reversible_after_leaving_active_search(self):
        item = dict(id='github:hidden', source='github', title='Work', status='todo', url='https://github.com/a/api/pull/1')
        self.store.state['items'] = [item]
        self.store.edit(dict(action='override', id=item['id'], value={'hidden':True, 'pinned':True}))
        with patch.object(work_links, 'refresh', return_value=[{**item,'status':'done'}]) as refresh:
            self.store.sync('github', lambda: [])
        refresh.assert_called_once()
        self.assertEqual(self.store.snapshot()['items'][0]['status'], 'done')
        self.assertTrue(self.store.snapshot()['overrides'][item['id']]['hidden'])
        self.store.edit(dict(action='override', id=item['id'], value={'hidden':False}))
        self.assertFalse(self.store.snapshot()['overrides'][item['id']]['hidden'])
        self.assertTrue(self.store.snapshot()['overrides'][item['id']]['pinned'])
        with self.assertRaises(ValueError):
            self.store.edit(dict(action='override', id=item['id'], value={'hidden':1}))

    def test_hide_during_sync_keeps_the_cached_row_available(self):
        item = dict(id='github:hidden', source='github', title='Work', status='todo')
        self.store.state['items'] = [item]
        def refresh(*args):
            self.store.edit(dict(action='override', id=item['id'], value={'hidden':True}))
            return []
        record = self.save_session(itemIds=[item['id']])
        with patch.object(work_links, 'refresh', side_effect=refresh):
            self.store.sync('github', lambda: [])
        self.assertEqual(self.store.snapshot()['items'][0]['id'], item['id'])


if __name__ == '__main__':
    unittest.main()
