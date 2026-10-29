import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import locations
import server


class FolderTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name).resolve()
        self.env=patch.dict(os.environ,{'TRUFFLE_WORKDIR':str(self.root)});self.env.start()
        self.store=server.Store(self.root/'data/state.json')
        self.repo=self.root/'api';self.repo.mkdir()

    def tearDown(self):
        self.store.file_lock.close();self.env.stop();self.temp.cleanup()

    def issue(self,number='ENG-1',parent=None,project=True):
        return dict(id='linear:'+number,source='linear',title=number,reference=number,url='https://linear.app/acme/issue/'+number,
            project='Service' if project else 'Engineering',projectId='project-1' if project else None,parent=parent,status='todo')

    def quick(self,**payload):
        self.store.quick_started_at=-100
        return self.store.quick_session(dict(agent='claude',**payload))

    def test_unknown_folder_prompts_without_saving_a_session(self):
        before=self.store.snapshot()
        with self.assertRaises(locations.FolderRequired) as raised:self.quick()
        self.assertEqual(self.store.snapshot(),before)
        self.assertIn(str(self.repo),[x['path'] for x in raised.exception.location['folders']])

    def test_github_matches_folder_without_git_and_ignores_old_root_hint(self):
        item=dict(id='github:1',source='github',title='PR',url='https://github.com/acme/API/pull/1',project='acme/API',status='todo')
        self.store.state['items']=[item];self.store.state['itemSettings'][item['id']]={'cwd':str(self.root)}
        self.assertEqual(self.quick(itemId=item['id'])['cwd'],str(self.repo))
        self.assertEqual(self.quick(scopeSource='github',scope='acme/API')['cwd'],str(self.repo))

    def test_new_folder_is_confined_to_root_and_remembers_the_repo(self):
        item=dict(id='github:1',source='github',title='PR',url='https://github.com/acme/new-repo/pull/1',project='acme/new-repo',status='todo')
        self.store.state['items']=[item]
        for name in ['../escape','/tmp/escape','.hidden','a/b','a\\b']:
            with self.subTest(name=name),self.assertRaises(ValueError):self.quick(itemId=item['id'],newFolder=name)
        record=self.quick(itemId=item['id'],newFolder='new-repo')
        self.assertEqual(record['cwd'],str(self.root/'new-repo'))
        self.assertTrue((self.root/'new-repo').is_dir())
        self.assertEqual(self.store.state['folderMappings']['github:acme/new-repo']['path'],record['cwd'])
        self.assertEqual(self.quick(itemId=item['id'])['cwd'],record['cwd'])

    def test_linear_family_choice_propagates_to_siblings_not_other_families(self):
        parent=dict(id='linear:P1',reference='ENG-100',title='Parent',url='https://linear.app/acme/issue/ENG-100')
        first=self.issue(parent=parent);second=self.issue('ENG-2',parent=parent);other=self.issue('ENG-3')
        root={**parent,'source':'linear','project':'Service','projectId':'project-1','status':'todo','children':[{'id':first['id']}]}
        self.store.state['items']=[first,second,other,root]
        self.quick(itemId=first['id'],folderPath=str(self.repo))
        self.assertEqual(self.quick(itemId=second['id'])['cwd'],str(self.repo))
        self.assertEqual(self.quick(itemId=root['id'])['cwd'],str(self.repo))
        with self.assertRaises(locations.FolderRequired):self.quick(itemId=other['id'])

    def test_linear_project_mapping_survives_restart_and_item_override_wins(self):
        first=self.issue();second=self.issue('ENG-2');self.store.state['items']=[first,second]
        self.quick(itemId=first['id'],folderPath=str(self.repo),rememberKey='linear-project:acme:project-1')
        self.store.file_lock.close();self.store=server.Store(self.root/'data/state.json')
        self.assertEqual(self.quick(itemId=second['id'])['cwd'],str(self.repo))
        other=self.root/'other';other.mkdir()
        self.quick(itemId=second['id'],folderPath=str(other),rememberKey='item:'+second['id'])
        self.assertEqual(self.quick(itemId=second['id'])['cwd'],str(other))
        self.assertEqual(self.quick(itemId=first['id'])['cwd'],str(self.repo))

    def test_linear_team_is_not_treated_as_a_project_mapping(self):
        item=self.issue(project=False)
        self.assertEqual([x['key'] for x in locations.contexts(item)],['item:'+item['id']])

    def test_linear_link_resolution_is_cached_and_rejects_redirects_elsewhere(self):
        with patch.dict(os.environ,{'LINEAR_API_KEY':'test'}),patch.object(server,'request_json',return_value={'issue':{'url':'https://linear.app/acme/issue/ENG-1'}}) as request:
            self.assertEqual(self.store.linear_ticket_url('ENG-1'),'https://linear.app/acme/issue/ENG-1')
            self.store.linear_ticket_url('ENG-1');request.assert_called_once()
            with self.assertRaises(ValueError):self.store.linear_ticket_url('../invalid')
        with patch.dict(os.environ,{'LINEAR_API_KEY':'test'}),patch.object(server,'request_json',return_value={'issue':{'url':'https://example.com/escape'}}):
            with self.assertRaises(ValueError):self.store.linear_ticket_url('ENG-2')

    def pr(self, **updates):
        return dict(id='github:pr-42',source='github',kind='Pull request',title='Fix regression',reference='#42',
                    url='https://github.com/acme/api/pull/42',project='acme/api',status='needs-review',**updates)

    def test_immediate_pr_review_is_distinct_and_targets_only_this_pr(self):
        import agents
        item=self.pr();self.store.state['items']=[item]
        implementation=self.quick(itemId=item['id'])
        self.store.quick_started_at=-100
        review=self.store.quick_session(dict(agent='codex',itemId=item['id']),review=True)
        self.assertNotEqual(review['id'],implementation['id'])
        self.assertEqual(review['title'],'Review api#42');self.assertEqual(review['workflow'],'pr-review')
        self.assertEqual(review['cwd'],str(self.repo));self.assertEqual([x['id'] for x in review['items']],[item['id']])
        context=agents.context_packet(self.store.session_context(review['id']))
        self.assertIn('$eddie-review '+item['url'],context)
        self.assertIn('Do not wait for more instructions',context)
        self.assertIn('.worktrees/review-pr-',context)
        self.assertNotIn('If no work is linked yet, wait',context)
        self.assertIsNone(implementation['workflow'])
        self.store.quick_started_at=-100
        self.assertEqual(self.store.quick_session(dict(agent='codex',itemId=item['id']),review=True)['id'],review['id'])

    def test_review_folder_prompt_and_claude_invocation(self):
        import agents
        item=self.pr();item['url']='https://github.com/acme/missing/pull/42';self.store.state['items']=[item]
        before=self.store.snapshot()
        with self.assertRaises(locations.FolderRequired):self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)
        self.assertEqual(self.store.snapshot(),before)
        review=self.store.quick_session(dict(agent='claude',itemId=item['id'],folderPath=str(self.repo)),review=True)
        self.assertIn('/eddie-review '+item['url'],review['instruction'])
        self.assertEqual(self.store.state['folderMappings']['github:acme/missing']['path'],str(self.repo))
        self.store.register_agent(dict(id=review['id'],agent='claude',sessionId=review['sessionId'],cwd=str(self.repo)))
        current=self.store.state['sessions'][-1]
        self.assertEqual(current['workflow'],'pr-review');self.assertEqual(current['reviewItemId'],item['id'])
        self.assertEqual(self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)['id'],review['id'])
        self.store.edit(dict(action='archive-session',id=review['id']))
        self.store.quick_started_at=-100
        with self.assertRaises(agents.AgentError):self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)
        recovered=self.store.recover_session(dict(id=review['id']))
        self.assertEqual(recovered['reviewItemId'],item['id']);self.assertEqual(recovered['workflow'],'pr-review')

    def test_review_rejects_non_pr_and_invalid_targets_without_state_change(self):
        cases=[self.issue(),{**self.pr(),'kind':'Issue'}, {**self.pr(),'url':'https://evil.test/acme/api/pull/42'},
               {**self.pr(),'url':'https://github.com/acme/api/issues/42'}, {**self.pr(),'url':'https://github.com/acme/api/pull/42?redirect=other'}]
        for item in cases:
            self.store.state['items']=[item];before=self.store.snapshot()
            with self.subTest(item=item),self.assertRaises(ValueError):
                self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)
            self.assertEqual(self.store.snapshot(),before)

    def test_review_does_not_inherit_implementation_opinions(self):
        import agents
        item=self.pr();self.store.state['items']=[item]
        implementation=self.quick(itemId=item['id'])
        self.store.edit(dict(action='checkpoint',id=implementation['id'],done='Prior design opinions',blocked='',next=''))
        self.store.quick_started_at=-100
        review=self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)
        context=agents.context_packet(self.store.session_context(review['id']))
        self.assertNotIn('Prior design opinions',context)


    def test_started_review_is_reused_without_replacing_its_worktree(self):
        item=self.pr();self.store.state['items']=[item]
        review=self.store.quick_session(dict(agent='claude',itemId=item['id']),review=True)
        worktree=self.repo/'.worktrees'/'review-pr-42';worktree.mkdir(parents=True)
        self.store.register_agent(dict(id=review['id'],agent='claude',sessionId=review['sessionId'],cwd=str(worktree)))
        count=len(self.store.state['sessions'])
        reused=self.store.quick_session(dict(agent='claude',itemId=item['id'],folderPath=str(self.repo)),review=True)
        self.assertEqual(reused['id'],review['id']);self.assertEqual(reused['cwd'],str(worktree))
        self.assertFalse(reused['launchNew']);self.assertEqual(len(self.store.state['sessions']),count)
        self.store.quick_started_at=-100
        other=self.store.quick_session(dict(agent='codex',itemId=item['id']),review=True)
        self.assertNotEqual(other['id'],reused['id'])
