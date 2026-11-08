"""Optional retired-instruction measurements, separate from plan comparison data."""
from pathlib import Path
import re
import statistics
import subprocess
import sys
import threading

STAGES = ('analyse', 'mesh', 'process')


def parse_instructions(stderr):
    match = re.search(r'^\s*(\d+)\s+instructions retired\s*$', stderr, re.M)
    return int(match.group(1)) if match else None


class InstructionMeter:
    def __init__(self):
        self._lock = threading.Lock()
        self._warned = False
        self.enabled = sys.platform == 'darwin' and Path('/usr/bin/time').is_file()
        if self.enabled:
            # Some macOS sandboxes prohibit time's resource sysctls. Its own failure
            # must not turn a successful child into a benchmark error.
            try:
                probe = subprocess.run(['/usr/bin/time', '-l', '/usr/bin/true'],
                                       capture_output=True, text=True, timeout=10)
                self.enabled = probe.returncode == 0
                if not self.enabled:
                    self.unavailable(probe.stderr.strip())
            except (OSError, subprocess.TimeoutExpired) as exc:
                self.enabled = False
                self.unavailable(str(exc))
        else:
            self.unavailable('requires macOS /usr/bin/time -l')

    def unavailable(self, why):
        with self._lock:
            if not self._warned:
                print(f'Retired instructions unavailable; recording null: {why}', file=sys.stderr)
                self._warned = True

    def wrap(self, args):
        return ['/usr/bin/time', '-l', *args] if self.enabled else args

    def record(self, stderr):
        count = parse_instructions(stderr) if self.enabled else None
        if count is None:
            self.unavailable('instructions retired line missing from /usr/bin/time -l')
        return count


def instruction_summary(parts):
    result = {}
    for stage in STAGES:
        values = [p[stage] for p in parts.values() if p[stage] is not None]
        result[stage] = {'total': sum(values) if values and len(values) == len(parts) else None,
                         'median': statistics.median(values) if values else None,
                         'measured_parts': len(values), 'parts': len(parts)}
    return result


def part_total(stages):
    values = [stages.get(stage) for stage in STAGES]
    # Incomplete measurements cannot be compared as whole-part totals.
    return sum(values) if all(v is not None for v in values) else None


def compare_instructions(before, after, emit):
    a, b = before.get('instructions', {}), after.get('instructions', {})
    for stage in STAGES:
        x = a.get('stages', {}).get(stage, {}).get('total')
        y = b.get('stages', {}).get(stage, {}).get('total')
        delta = f'{y - x:+d}' if x is not None and y is not None else 'unavailable'
        emit(f'Instructions {stage}: {x} -> {y} (delta {delta})')
    prior, current = a.get('parts', {}), b.get('parts', {})
    for part in sorted(prior.keys() & current.keys()):
        x, y = part_total(prior[part]), part_total(current[part])
        if x is None or y is None:
            continue
        # Strictly greater than 20%, without rounding or floating-point boundaries.
        if abs(y - x) * 5 > x:
            percentage = f'{100 * (y - x) / x:+.1f}%' if x else 'from zero'
            emit(f'Instructions >20% {part}: {x} -> {y} ({percentage})')
