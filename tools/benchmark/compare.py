#!/usr/bin/env python3
"""Compare benchmark reports; only new errors or newly failed parts fail the gate."""
import argparse
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from common import normalise
from performance import compare_instructions


def compare(before, after, emit=print):
    a, b = before['data'], after['data']
    changed = False
    if a.get('tools_sha256') != b.get('tools_sha256'):
        emit('Tool library changed.')
        changed = True
    old, new = a['totals'], b['totals']
    for family in sorted(old['outcomes'].keys() | new['outcomes'].keys()):
        x, y = old['outcomes'].get(family, {}), new['outcomes'].get(family, {})
        changes = []
        for status in ('cut', 'left_out', 'covered', 'unaccounted'):
            v, w = x.get(status, 0), y.get(status, 0)
            changes.append(f'{status} {v} -> {w} ({w-v:+d})')
            if v != w:
                changed = True
            if status == 'cut' and w < v:
                emit(f'FEWER FEATURES CUT: {family}: {v} -> {w} ({w-v:+d}); review required')
        emit(f'{family}: ' + ', '.join(changes))
    for family in sorted(old['left_out_reasons'].keys() | new['left_out_reasons'].keys()):
        x = old['left_out_reasons'].get(family, {})
        y = new['left_out_reasons'].get(family, {})
        for reason in sorted(x.keys() | y.keys()):
            delta = y.get(reason, 0) - x.get(reason, 0)
            if delta:
                changed = True
                emit(f'Reason {family} ({delta:+d}): {reason}')
    prior = {p['part']: p for p in a['parts']}
    current = {p['part']: p for p in b['parts']}
    failed = False
    for part in sorted(prior.keys() | current.keys()):
        x, y = prior.get(part), current.get(part)
        if x != y:
            changed = True
            emit(f'Part {part}:\n  before: {json.dumps(x, sort_keys=True)}\n  after:  {json.dumps(y, sort_keys=True)}')
        if y and y.get('error') and (not x or not x.get('error') or
                                    normalise(x['error']) != normalise(y['error'])):
            failed = True
            emit(f'NEW ERROR {part}: {y["error"]}')
    if not changed:
        emit('No change in plan data (performance does not affect the gate).')
    compare_instructions(before, after, emit)
    return int(failed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('current', type=Path)
    args = parser.parse_args()
    return compare(json.loads(args.baseline.read_text()), json.loads(args.current.read_text()))


if __name__ == '__main__':
    sys.exit(main())
