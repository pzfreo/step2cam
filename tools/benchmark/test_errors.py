import copy
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import totals
from compare import compare
from performance import STAGES
from run import command, measured_command, portable_error, run_part


class ErrorTests(unittest.TestCase):
    def error_for(self, user):
        home = Path('/Users') / user
        repo, cache = home / 'work/step2cam', home / 'cache/benchmark'
        error = (f'process exited 1: /usr/bin/time -l {repo}/bin/process: '
                 f'cannot read {cache}/mesh.json or {home}/tools.json')
        return portable_error(error, cache, repo, home)

    def test_all_roots_and_time_wrapper(self):
        expected = ('process exited 1: <repo>/bin/process: '
                    'cannot read <cache>/mesh.json or <home>/tools.json')
        self.assertEqual(self.error_for('alice'), expected)
        self.assertEqual(self.error_for('bob'), expected)
        self.assertEqual(portable_error("['/usr/bin/time', '-l', '/cache/part']",
                                        '/cache', '/repo', '/home/user'), "['<cache>/part']")

    def test_stored_exception_is_portable(self):
        cache = Path('/tmp/benchmark-cache')
        with patch('run.extract', side_effect=OSError(f'bad input {cache}/part.stp')):
            row, counts = run_part('p', None, cache, None, threading.Lock(), None, None)
        self.assertEqual(row['error'], 'bad input <cache>/part.stp')
        self.assertEqual(counts, dict.fromkeys(STAGES))

    def test_portable_errors_do_not_fail_comparison_on_another_machine(self):
        row = {'part': 'p', 'features': [], 'warnings': [], 'error': self.error_for('alice'),
               'label_counts': {}, 'recognised_counts': {},
               'labels_complete': False, 'recognition_complete': False}
        before = {'data': {'parts': [row], 'totals': totals([row])}}
        after = copy.deepcopy(before)
        after['data']['parts'][0]['error'] = self.error_for('bob')
        after['data']['totals'] = totals(after['data']['parts'])
        lines = []
        self.assertEqual(compare(before, after, lines.append), 0)
        self.assertFalse(any('NEW ERROR' in line for line in lines))

    def test_timeout_has_only_stage_and_duration(self):
        args = ['/usr/bin/time', '-l', '/Users/alice/cache/process', '/Users/alice/repo/mesh']
        meter = Mock()
        meter.wrap.return_value = args
        with patch('run.command', side_effect=subprocess.TimeoutExpired(args, 12)):
            with self.assertRaisesRegex(RuntimeError, '^mesh timed out after 12 s$'):
                measured_command(args[2:], 'mesh', dict.fromkeys(STAGES), meter)


class ProcessGroupTests(unittest.TestCase):
    @unittest.skipUnless(hasattr(os, 'killpg'), 'requires POSIX process groups')
    def test_timeout_kills_wrapper_and_its_stage(self):
        # A wrapper waits for a stage that holds the inherited output pipes open.
        # Killing only the wrapper leaves the stage running and communicate blocked.
        wrapper = ('import subprocess,sys; '
                   'child=subprocess.Popen([sys.executable,"-I","-c","import time; time.sleep(7)"]); '
                   'print(child.pid,flush=True); child.wait()')
        started = time.monotonic()
        with self.assertRaises(subprocess.TimeoutExpired) as raised:
            command([sys.executable, '-I', '-c', wrapper], timeout=0.5)
        elapsed = time.monotonic() - started
        child = int(raised.exception.output.strip())
        try:
            self.assertLess(elapsed, 3, 'timeout waited for the orphaned stage')
            deadline = time.monotonic() + 2
            while True:
                try:
                    os.kill(child, 0)
                except ProcessLookupError:
                    break
                if time.monotonic() >= deadline:
                    self.fail('wrapped stage still exists after timeout')
                time.sleep(0.02)
        finally:
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass


if __name__ == '__main__':
    unittest.main()
