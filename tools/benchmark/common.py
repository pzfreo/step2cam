"""Report accounting shared by the runner and comparison (standard library only)."""
import re
from collections import Counter, defaultdict


def family(group):
    """Geometry groups identify features, not individual machining operations."""
    if group.startswith(('STEP1HOLE', 'STEP1POCKET')):
        return 'hole'
    for prefix, name in [('FG_CHAMFER_SURFACE', 'chamfer'), ('POCKET_', 'pocket'),
                         ('SLOT_', 'slot'), ('CORNER_NOTCH_', 'notch')]:
        if group.startswith(prefix):
            return name
    return 'other'


def normalise(reason):
    # Keep signs/punctuation: only numeric values (including exponents) are grouped.
    return re.sub(r'\d+(?:\.\d*)?(?:[eE][+-]?\d+)?', '#', str(reason))


def summarise_part(analysis, output):
    plan = output['plan']
    cut = set()
    for setup in plan.get('setups', []):
        for op in setup.get('operations', []):
            cut.update(op.get('features', []))
            if op.get('feature'):
                cut.add(op['feature'])
    skipped = {s['feature']: normalise(s['reason']) for s in plan.get('skipped', [])}
    covered = set(plan.get('covered_by_roughing', []))
    features = []
    for f in sorted(analysis['features'], key=lambda f: f['id']):
        fid = f['id']
        status = ('cut' if fid in cut else 'covered' if fid in covered else
                  'left_out' if fid in skipped else 'unaccounted')
        row = {'id': fid, 'family': f['family'], 'outcome': status}
        if status == 'left_out':
            row['reason'] = skipped[fid]
        features.append(row)
    return features, sorted(normalise(w) for w in plan.get('warnings', []))


def totals(parts):
    labels, recognised, warnings, errors = Counter(), Counter(), Counter(), Counter()
    outcomes, reasons = defaultdict(Counter), defaultdict(Counter)
    matches = {name: {'labels': 0, 'recognised': 0, 'parts_equal': 0, 'parts_compared': 0}
               for name in ('hole', 'chamfer')}
    both_equal = 0
    for p in parts:
        labels.update(p['label_counts'])
        recognised.update(p['recognised_counts'])
        warnings.update(p['warnings'])
        if p['error']:
            errors[normalise(p['error'])] += 1
        for f in p['features']:
            outcomes[f['family']][f['outcome']] += 1
            if f['outcome'] == 'left_out':
                reasons[f['family']][f['reason']] += 1
        if p['recognition_complete'] and p['labels_complete']:
            equal = True
            for label, ours in [('hole', 'holes'), ('chamfer', 'chamfers')]:
                m = matches[label]
                a, b = p['label_counts'].get(label, 0), p['recognised_counts'].get(ours, 0)
                m['labels'] += a
                m['recognised'] += b
                m['parts_compared'] += 1
                m['parts_equal'] += a == b
                equal = equal and a == b
            both_equal += equal
    return {'labels': dict(labels), 'recognised': dict(recognised), 'recognition': matches,
            'holes_and_chamfers_parts_equal': both_equal,
            'outcomes': {k: dict(v) for k, v in outcomes.items()},
            'left_out_reasons': {k: dict(v) for k, v in reasons.items()},
            'warnings': dict(warnings), 'errors': dict(errors), 'parts': len(parts)}
