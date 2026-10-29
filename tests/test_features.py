import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
from http.server import ThreadingHTTPServer

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import server
import dashboard_features as features


def issue(key='github:1', **values):
    return dict(id=key,source='github',title='fix: caching ENG-1',reference='#1',project='acme/api',
                url='https://github.com/acme/api/pull/1',status='needs-review',sourceStatus='Awaiting review',
                relations=['authored'],ci={'state':'pending'},**values)


class FeatureTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.store=server.Store(Path(self.temp.name)/'data/state.json')
        self.addCleanup(self.store.file_lock.close)
        self.addCleanup(self.temp.cleanup)
        self.item=issue()
        self.store.state['items']=[self.item]

    def test_snooze_wakes_on_source_changes_not_counters_or_local_overrides(self):
        self.store.edit(dict(action='snooze',id=self.item['id'],mode='change'))
        self.item.update(updatedAt='tomorrow',eventCount=90000)
        self.assertTrue(self.store.snapshot()['items'][0]['snoozed'])
        self.store.edit(dict(action='override',id=self.item['id'],value={'status':'blocked'}))
        self.assertTrue(self.store.snapshot()['items'][0]['snoozed'])
        self.item['ci']['state']='failed'
        self.assertFalse(self.store.snapshot()['items'][0]['snoozed'])
        self.store.edit(dict(action='snooze',id=self.item['id'],mode='until',until=time.time()+1000))
        self.assertTrue(features.snoozed(self.store.state,self.item))
        self.assertFalse(features.snoozed(self.store.state,self.item,now=time.time()+1001))
        self.store.edit(dict(action='snooze',id=self.item['id'],mode='clear'))
        self.assertNotIn(self.item['id'],self.store.state['snoozes'])

    def test_invalid_snooze_or_filter_rolls_back(self):
        before=self.store.snapshot()
        for payload in [dict(action='snooze',id=self.item['id'],mode='until',until=float('nan')),
                        dict(action='save-sentry-view',name='Unsafe',filters={'environment':'prod or secret'}),
                        dict(action='save-sentry-view',name='Invalid',filters={'minUsers':-1})]:
            with self.assertRaises(ValueError):self.store.edit(payload)
            self.assertEqual(before,self.store.snapshot())

    def test_change_baseline_is_quiet_and_bounded(self):
        features.observe(self.store.state,'github',[self.item],now=100)
        self.assertEqual(self.store.state['changes'],[])
        self.item.update(updatedAt='new time',title='New title')
        features.observe(self.store.state,'github',[self.item],now=200)
        self.assertEqual(self.store.state['changes'],[])
        self.item['ci']['state']='failed'
        features.observe(self.store.state,'github',[self.item],now=300)
        self.assertEqual(self.store.state['changes'][0]['message'],'CI failing')
        self.item['ci']['state']='passed'
        features.observe(self.store.state,'github',[self.item],now=400)
        self.assertEqual(self.store.state['changes'][0]['message'],'CI recovered')
        self.store.edit(dict(action='read-changes',all=True))
        self.assertTrue(all(x['read'] for x in self.store.state['changes']))
        self.assertNotIn('changeBaselines',self.store.snapshot())
        self.assertIn('changeBaselines',self.store.snapshot(export=True))

    def test_missing_active_pr_gets_completion_refresh_and_change(self):
        features.observe(self.store.state,'github',[self.item])
        done={**self.item,'status':'done','sourceStatus':'Merged'}
        with patch('work_links.refresh',return_value=[done]) as refresh:
            self.store.sync('github',lambda:[])
        self.assertEqual(refresh.call_args.args[1][0]['id'],self.item['id'])
        self.assertIn('Merged',self.store.state['changes'][0]['message'])
        self.assertEqual(self.store.state['items'][0]['status'],'done')

    def test_related_work_combines_ticket_references_errors_and_manual_links(self):
        linear=dict(id='linear:1',source='linear',title='Caching',reference='ENG-1',
                    url='https://linear.app/acme/issue/ENG-1',sentryLinks=[{'url':'https://acme.sentry.io/issues/99/'}])
        sentry=dict(id='sentry:acme:99',source='sentry',title='Error',url='https://sentry.io/organizations/acme/issues/99/')
        note=dict(id='note:1',source='notes',title='Investigation')
        self.store.state['items'] += [linear,sentry]
        self.store.state['notes']=[note]
        groups=features.relationships(self.store.state)
        self.assertEqual(set(groups[0]['itemIds']),{self.item['id'],linear['id'],sentry['id']})
        self.store.edit(dict(action='link-work',itemIds=[self.item['id'],note['id']]))
        self.assertEqual(len(features.relationships(self.store.state)[0]['itemIds']),4)
        self.store.edit(dict(action='unlink-work',itemIds=[note['id'],self.item['id']]))
        self.assertEqual(len(features.relationships(self.store.state)[0]['itemIds']),3)

    def test_bundle_launch_receives_all_context_and_does_not_create_project(self):
        self.store.state['notes']=[dict(id='note:1',source='notes',title='Local',status='todo')]
        self.store.edit(dict(action='link-work',itemIds=[self.item['id'],'note:1']))
        with patch('locations.choose',return_value=(self.temp.name,None)):
            record=self.store.quick_session(dict(agent='codex',itemId=self.item['id']))
        self.assertEqual({x['id'] for x in record['items']},{self.item['id'],'note:1'})
        self.assertEqual(len(self.store.state['notes']),1)

    def test_your_move_respects_ownership_and_native_state(self):
        self.item['ci']['state']='failed'
        self.assertIn('Fix failing CI',features.your_move(self.item))
        self.item['relations']=['reviews']
        self.assertEqual(features.your_move(self.item),['Your review requested'])
        self.item['status']='draft'
        self.assertEqual(features.your_move(self.item),[])
        self.item['status']='done'
        self.assertEqual(features.your_move(self.item),[])

    def test_checkpoint_survives_registration_and_appears_in_context(self):
        record=self.store.prepare_session(dict(agent='codex',title='Work',cwd=self.temp.name,itemIds=[self.item['id']]))
        self.store.register_agent(dict(operation='checkpoint',id=record['id'],done='Tests pass',blocked='',next='Review PR'))
        self.store.save_session(dict(id=record['id'],sessionId='11111111-1111-4111-8111-111111111111'))
        self.assertEqual(self.store.state['sessions'][0]['checkpoint']['next'],'Review PR')
        self.assertIn('Tests pass',server.agents.context_packet(self.store.session_context(record['id'])))

    def test_review_reports_keep_capture_times_and_do_not_change_status(self):
        self.store.state['sources']['github']['lastSuccess']=123
        record=self.store.prepare_review(dict(agent='codex'))
        self.store.state['sources']['github']['lastSuccess']=456
        self.store.register_agent(dict(operation='save-briefing',id=record['id'],body='Review https://github.com/acme/api/pull/1'))
        report=self.store.snapshot()['briefings'][0]
        self.assertEqual(report['sourceTimes']['github'],123)
        self.assertEqual(self.item['status'],'needs-review')
        self.assertEqual(record['items'],[])
        self.assertIn('briefing --record',server.agents.context_packet(self.store.session_context(record['id'])))

    def test_cli_saves_briefing_through_authenticated_local_endpoint(self):
        record=self.store.prepare_review(dict(agent='codex'))
        http=ThreadingHTTPServer(('127.0.0.1',0),server.Handler)
        http.authority=f'127.0.0.1:{http.server_port}'
        http.store=self.store;http.csrf='browser-csrf';http.agent_token='private-token'
        worker=threading.Thread(target=http.serve_forever,daemon=True);worker.start()
        try:
            (Path(self.temp.name)/'data/runtime.json').write_text(json.dumps(dict(url='http://'+http.authority,token=http.agent_token)))
            report=Path(self.temp.name)/'report.md';report.write_text('Three next actions.\nCheck CI.')
            result=subprocess.run([sys.executable,str(server.ROOT/'register.py'),'briefing','--record',record['id'],'--file',str(report)],env={**os.environ,'TRUFFLE_HOME':self.temp.name},capture_output=True,text=True,timeout=10)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertNotIn(http.agent_token,result.stdout)
            self.assertEqual(self.store.state['briefings'][0]['body'],report.read_text())
        finally:http.shutdown();http.server_close();worker.join()

    def test_new_agent_receives_related_session_handoff(self):
        first=self.store.prepare_session(dict(agent='codex',title='Original',cwd=self.temp.name,itemIds=[self.item['id']]))
        self.store.edit(dict(action='checkpoint',id=first['id'],done='Reproduced failure',next='Fix schema',blocked=''))
        other=self.store.prepare_session(dict(agent='claude',title='Follow-up',cwd=self.temp.name,itemIds=[self.item['id']]))
        context=self.store.session_context(other['id'])
        self.assertEqual(context['handoffs'][0]['checkpoint']['next'],'Fix schema')
        self.assertIn('Reproduced failure',server.agents.context_packet(context))

    def test_environment_lookup_is_single_flight_and_keeps_full_feed(self):
        entered=threading.Event();release=threading.Event()
        def fetch(*args,**kwargs):
            entered.set();release.wait(3);return []
        with patch('sentry_source.configured',return_value=True),patch('sentry_source.fetch',side_effect=fetch) as remote:
            self.store.sentry_environment(dict(environment='prod'))
            self.assertTrue(entered.wait(2))
            self.store.sentry_environment(dict(environment='staging'))
            self.assertEqual(remote.call_count,1)
            self.assertEqual(self.store.state['items'],[self.item])
            release.set()
            for _ in range(100):
                if not self.store.state['sentryEnvironments']['prod'].get('running'):break
                time.sleep(.01)
        self.assertEqual(self.store.state['sentryEnvironments']['prod']['itemIds'],[])
        self.assertTrue(self.store.state['sentryEnvironments']['prod']['lastSuccess'])
        self.assertEqual(self.store.state['items'],[self.item])

    def test_environment_lookup_cannot_bypass_source_backoff(self):
        self.store.state['sources']['sentry'].update(failures=2,nextAttempt=time.time()+3600)
        with patch('sentry_source.configured',return_value=True),patch('sentry_source.fetch') as remote:
            with self.assertRaisesRegex(ValueError,'retry window'):
                self.store.sentry_environment(dict(environment='prod'))
            remote.assert_not_called()

    def test_saved_filters_survive_restart(self):
        self.store.edit(dict(action='save-sentry-view',name='Production',filters=dict(minUsers=2,days=7,environment='prod',category='error')))
        self.store.file_lock.close()
        another=server.Store(self.store.path)
        try:self.assertEqual(another.snapshot()['sentryViews'][0]['filters']['environment'],'prod')
        finally:another.file_lock.close()


if __name__=='__main__':unittest.main()
