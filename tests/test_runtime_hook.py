"""The lifecycle bridge must report activity without retaining command content."""
import unittest
from runtime_hook import activity_snapshot

class ActivityTests(unittest.TestCase):
    def test_shells_monitors_and_wakeups_are_counted_without_contents(self):
        data = {'background_tasks':[
            {'id':'s','type':'shell','status':'running','command':'secret'},
            {'id':'s','type':'shell','status':'running'},
            {'id':'m','type':'monitor','status':'running','description':'private'},
            {'id':'done','type':'shell','status':'completed'},
            {'id':'future','type':'custom kind','status':'running'}],
            'session_crons':[{'prompt':'secret'}]}
        self.assertEqual(activity_snapshot(data), {'backgroundCounts':{'shell':1,'monitor':1,'custom kind':1},'scheduledWakeups':1})
    def test_missing_is_unknown_and_empty_is_authoritative_zero(self):
        self.assertEqual(activity_snapshot({}), {})
        self.assertEqual(activity_snapshot({'background_tasks':[],'session_crons':[]}), {'backgroundCounts':{},'scheduledWakeups':0})
    def test_malformed_and_oversized_metadata_is_bounded(self):
        self.assertEqual(activity_snapshot({'background_tasks':[None,{'type':[]}]}), {'backgroundCounts':{'other':1}})
        self.assertEqual(activity_snapshot({'session_crons':[{}]*1100})['scheduledWakeups'],1000)

if __name__ == '__main__': unittest.main()
