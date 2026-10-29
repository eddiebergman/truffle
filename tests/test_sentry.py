import os
from pathlib import Path
import sys
import unittest
import tempfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import server
import sentry_source as sentry
import work_links


def node(identity='12', status='unresolved'):
    return dict(id=identity, title='Example failure', permalink=f'https://example.sentry.io/issues/{identity}/',
                project={'slug':'api'}, status=status, shortId='API-X', count='42', userCount=3,
                lastSeen='2026-10-20T10:00:00Z')


class SentryTests(unittest.TestCase):
    def setUp(self):
        self.config = patch.object(sentry, 'settings', return_value=('example','private-token','https://sentry.io','90d'))
        self.config.start()
        self.addCleanup(self.config.stop)

    def test_pagination_deduplicates_and_keeps_fixed_api_host(self):
        links = '<https://untrusted.example/path?cursor=next>; rel="next"; results="true"'
        with patch.object(sentry, 'get', side_effect=[([node()], links),([node(), node('13')], '')]) as get:
            items = sentry.fetch(server)
        self.assertEqual([x['id'] for x in items], ['sentry:example:12','sentry:example:13'])
        self.assertEqual(get.call_args.args[0], '/api/0/organizations/example/issues/')
        self.assertEqual(get.call_args.args[1]['cursor'], 'next')
        self.assertEqual(get.call_args.args[1]['statsPeriod'], '90d')
        self.assertNotIn('private-token', str(items))
        self.assertEqual(sentry.next_cursor('<https://sentry.io/>; rel="next"; results="false"; cursor="a"'), None)

    def test_incomplete_or_missing_feed_raises_instead_of_replacing_cache(self):
        with patch.object(sentry, 'get', return_value=([], '<https://sentry.io/>; rel="next"; results="true"; cursor="a"')):
            with self.assertRaises(server.SyncError):
                sentry.fetch(server)
        with patch.object(sentry, 'get', return_value=(None,'')):
            with self.assertRaises(server.SyncError):
                sentry.fetch(server)

    def test_saved_issue_refresh_observes_resolution_and_missing_access(self):
        refs=[sentry.item(node(), 'example', server),sentry.item(node('13'), 'example', server)]
        with patch.object(sentry, 'get', side_effect=[(node(status='resolved'),''),(None,'')]):
            items=sentry.refresh(refs,server)
        self.assertEqual(items[0]['status'],'done')
        self.assertTrue(items[1]['sourceUnavailable'])
        with self.assertRaises(server.SyncError):
            sentry.refresh([{'id':'sentry:another:12'}],server)

    def test_url_import_and_provider_links_validate_identity(self):
        with patch.object(sentry, 'get', return_value=(node(),'')) as get:
            item=work_links.resolve('https://example.sentry.io/issues/12/',server)
            self.assertEqual(item['id'],'sentry:example:12')
            self.assertEqual(get.call_args.args[0],'/api/0/organizations/example/issues/12/')
            with self.assertRaises(ValueError):
                work_links.resolve('https://sentry.io/organizations/other/issues/12/',server)
        with self.assertRaises(server.SyncError):
            sentry.item({**node(),'permalink':'https://sentry.io.evil.example/issues/12/'},'example',server)

    def test_env_loader_accepts_provider_settings_only(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            path=Path(directory)/'.env'
            path.write_text('SENTRY_ORG=example\nSENTRY_STATS_PERIOD=30d\nSENTRY_AUTH_TOKEN=private-token\nHOME=/other\n')
            server.load_env(path)
            self.assertEqual(os.environ['SENTRY_ORG'],'example')
            self.assertEqual(os.environ['SENTRY_STATS_PERIOD'],'30d')
            self.assertEqual(os.environ['SENTRY_AUTH_TOKEN'],'private-token')
            self.assertNotIn('HOME',os.environ)

    def test_credentials_cannot_be_redirected(self):
        self.assertIsNone(sentry.NoRedirect().redirect_request(None,None,302,'',{},'https://other.example'))


if __name__ == '__main__':
    unittest.main()
