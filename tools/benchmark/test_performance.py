from contextlib import redirect_stderr
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from performance import (InstructionMeter, STAGES, compare_instructions,
                         instruction_summary, parse_instructions, strip_time_report)


class InstructionTests(unittest.TestCase):
    def test_macos_time_stderr(self):
        stderr = '''application diagnostic: analysis complete
        0.12 real         0.09 user         0.02 sys
            16777216  maximum resident set size
                   0  page reclaims
           987654321  instructions retired
           123456789  cycles elapsed
'''
        self.assertEqual(parse_instructions(stderr), 987654321)
        self.assertEqual(strip_time_report(stderr), 'application diagnostic: analysis complete')
        self.assertEqual(parse_instructions('0 instructions retired\n'), 0)
        self.assertIsNone(parse_instructions('987 cycles elapsed\n'))
        self.assertIsNone(parse_instructions('diagnostic: 987 instructions retired\n'))

    def test_unavailable_warns_once(self):
        stderr = io.StringIO()
        with patch('performance.sys.platform', 'linux'), redirect_stderr(stderr):
            meter = InstructionMeter()
            for _ in range(3):
                self.assertIsNone(meter.record(''))
            self.assertEqual(meter.wrap(['process', 'a', 'm', 't']), ['process', 'a', 'm', 't'])
        self.assertEqual(stderr.getvalue().count('Retired instructions unavailable'), 1)

    def test_blocked_time_does_not_fail_children(self):
        from types import SimpleNamespace
        result = SimpleNamespace(returncode=1, stderr='time: sysctl: Operation not permitted')
        with patch('performance.sys.platform', 'darwin'), patch('performance.Path.is_file', return_value=True), \
                patch('performance.run_process', return_value=result), redirect_stderr(io.StringIO()):
            meter = InstructionMeter()
            self.assertFalse(meter.enabled)
            self.assertEqual(meter.wrap(['process']), ['process'])

    def test_failed_stage_keeps_count_but_not_resource_text_in_error(self):
        from types import SimpleNamespace
        from run import measured_command
        with patch('performance.sys.platform', 'darwin'), patch('performance.Path.is_file', return_value=True), \
                patch('performance.run_process', return_value=SimpleNamespace(returncode=0)):
            meter = InstructionMeter()
        counts = dict.fromkeys(STAGES)
        result = SimpleNamespace(returncode=1, stderr='bad mesh\n 0.12 real 0.09 user 0.02 sys\n 987 instructions retired\n')
        with patch('run.command', return_value=result) as command:
            with self.assertRaises(RuntimeError) as raised:
                measured_command(['specify-core', 'mesh'], 'mesh', counts, meter)
        self.assertEqual(counts['mesh'], 987)
        self.assertEqual(str(raised.exception), 'specify-core exited 1: bad mesh')
        self.assertEqual(command.call_args.args[0][:2], ['/usr/bin/time', '-l'])
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            meter.record('')
            meter.record('')
        self.assertEqual(stderr.getvalue().count('Retired instructions unavailable'), 1)

    def test_cargo_artifact_selects_executable_in_custom_target_directory(self):
        from types import SimpleNamespace
        from run import build_process
        executable = '/tmp/custom-cargo-target/release/examples/process'
        artifact = {'reason': 'compiler-artifact', 'target': {'name': 'process', 'kind': ['example']},
                    'executable': executable}
        result = SimpleNamespace(stdout=json.dumps({'reason': 'build-finished'}) + '\n' + json.dumps(artifact))
        with patch('run.command', return_value=result) as command:
            self.assertEqual(build_process(), Path(executable).resolve())
        self.assertIn('--message-format=json', command.call_args.args[0])

    def test_stage_totals_medians_and_missing(self):
        rows = {'a': dict(zip(STAGES, [10, 20, 40])), 'b': dict(zip(STAGES, [30, 40, None]))}
        summary = instruction_summary(rows)
        self.assertEqual(summary['analyse'], {'total': 40, 'median': 20, 'measured_parts': 2, 'parts': 2})
        self.assertIsNone(summary['process']['total'])
        self.assertEqual(summary['process']['median'], 40)
        self.assertEqual(summary['process']['measured_parts'], 1)
        self.assertIsNone(instruction_summary({'a': dict.fromkeys(STAGES)})['analyse']['median'])

    def test_strict_twenty_percent_both_directions(self):
        def report(values):
            parts = {p: dict(zip(STAGES, stages)) for p, stages in values.items()}
            return {'instructions': {'parts': parts, 'stages': instruction_summary(parts)}}
        before = report({p: [100, 100, 100] for p in ('up', 'down', 'boundary_up', 'boundary_down', 'missing')})
        after = report({'up': [100, 100, 161], 'down': [100, 100, 39],
                        'boundary_up': [100, 100, 160], 'boundary_down': [100, 100, 40],
                        'missing': [100, 100, None]})
        lines = []
        compare_instructions(before, after, lines.append)
        listed = [line for line in lines if line.startswith('Instructions >20%')]
        self.assertEqual(len(listed), 2)
        self.assertTrue(any(' up:' in line for line in listed))
        self.assertTrue(any(' down:' in line for line in listed))
        self.assertTrue(any(line.startswith('Instructions analyse: 500 -> 500 (delta +0)') for line in lines))

    def test_old_reports_and_zero_counts(self):
        lines = []
        compare_instructions({}, {}, lines.append)
        self.assertEqual(len(lines), 3)
        before = {'instructions': {'parts': {'p': dict.fromkeys(STAGES, 0)}}}
        after = {'instructions': {'parts': {'p': dict.fromkeys(STAGES, 1)}}}
        compare_instructions(before, after, lines.append)
        self.assertIn('Instructions >20% p: 0 -> 3 (from zero)', lines)


if __name__ == '__main__':
    unittest.main()
