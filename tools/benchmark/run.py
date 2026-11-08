#!/usr/bin/env python3
"""Run the fixed MachinePlan sample. Invoke with python3 -I (untrusted dataset)."""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import zipfile

# -I deliberately omits the script directory; import only our sibling modules.
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from common import family, summarise_part, totals
from fetch import RemoteZip, extract
from performance import InstructionMeter, STAGES, instruction_summary, strip_time_report

REPO = HERE.parent.parent


def command(args, stdout=None, timeout=600, check=True):
    result = subprocess.run([str(a) for a in args], stdout=stdout or subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, timeout=timeout, cwd=REPO)
    if check and result.returncode:
        raise RuntimeError(f'{Path(str(args[0])).name} exited {result.returncode}: '
                           f'{result.stderr.strip()[-4000:]}')
    return result


def build_process():
    result = command(['cargo', 'build', '--release', '--manifest-path', REPO / 'rust/Cargo.toml',
                      '--example', 'process', '--message-format=json'], timeout=1800)
    for line in result.stdout.splitlines():
        artifact = json.loads(line)
        if (artifact.get('reason') == 'compiler-artifact' and
                artifact.get('target', {}).get('name') == 'process' and
                'example' in artifact['target']['kind'] and artifact.get('executable')):
            return Path(artifact['executable']).resolve()
    raise RuntimeError('Cargo did not report the process example executable')


def measured_command(args, stage, counts, meter, stdout=None):
    try:
        result = command(meter.wrap([str(a) for a in args]), stdout=stdout, check=False)
    except subprocess.TimeoutExpired:
        meter.unavailable('a stage timed out without an instruction count')
        raise
    counts[stage] = meter.record(result.stderr)
    if result.returncode:
        raise RuntimeError(f'{Path(str(args[0])).name} exited {result.returncode}: '
                           f'{strip_time_report(result.stderr)[-4000:]}')


def run_part(part, z, cache, tools, fetch_lock, process, meter):
    counts = dict.fromkeys(STAGES)
    row = {'part': part, 'labels': [], 'label_counts': {}, 'recognised_counts': {},
           'features': [], 'warnings': [], 'error': None, 'labels_complete': False,
           'recognition_complete': False}
    try:
        # ZipFile's seekable stream is shared. Release the lock before expensive CLI work.
        with fetch_lock:
            step = extract(z, f'{part}/{part}.stp', cache)
            ops_path = extract(z, f'{part}/{part}_operations.json', cache)
            operations = json.loads(ops_path.read_text(encoding='utf-8-sig'))['operations']
            groups = set()
            for op in operations:
                stem = f"{op['sequence_number'] + 1:03d}_{op['tool_name']}"
                text = extract(z, f'{part}/{stem}_details.txt', cache).read_text(encoding='utf-8-sig')
                match = re.search(r'^Geometry Group\s+(.+?)\s*$', text, re.M)
                if not match:
                    raise ValueError(f'missing Geometry Group: {stem}')
                groups.add(match.group(1))
        row['labels'] = [{'group': g, 'family': family(g)} for g in sorted(groups)]
        row['label_counts'] = dict(Counter(f['family'] for f in row['labels']))
        row['labels_complete'] = True
        # Derived outputs are always regenerated, even with a warm download cache.
        work = cache / 'runs' / part
        work.mkdir(parents=True, exist_ok=True)
        analysis_path, mesh_path = work / 'analysis.json', work / 'mesh.json'
        measured_command(['specify-core', 'analyse', '-o', analysis_path, step], 'analyse', counts, meter)
        analysis = json.loads(analysis_path.read_text())
        row['recognised_counts'] = dict(Counter(f['family'] for f in analysis['features']))
        row['recognition_complete'] = True
        row['features'] = [{'id': f['id'], 'family': f['family'], 'outcome': 'unaccounted'}
                           for f in sorted(analysis['features'], key=lambda f: f['id'])]
        measured_command(['specify-core', 'mesh', '-o', mesh_path, step], 'mesh', counts, meter)
        with (work / 'process.json').open('w') as out:
            measured_command([process, analysis_path, mesh_path, tools], 'process', counts, meter, stdout=out)
        output = json.loads((work / 'process.json').read_text())
        row['features'], row['warnings'] = summarise_part(analysis, output)
        if output.get('error') or output.get('post_error'):
            row['error'] = str(output.get('error') or output['post_error'])
    except Exception as exc:
        row['error'] = str(exc)
    return row, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--zip', type=Path, help='use a local dataset ZIP instead of HTTP ranges')
    parser.add_argument('--cache', type=Path, default=Path.home() / '.cache/step2cam-benchmark')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--tools', type=Path, default=REPO / 'demo/tools.json')
    parser.add_argument('--jobs', type=int, default=4)
    parser.add_argument('--limit', type=int, help='run the first N sample entries (remote smoke check)')
    args = parser.parse_args()
    if args.jobs < 1 or (args.limit is not None and args.limit < 1):
        parser.error('--jobs and --limit must be positive')
    parts = (HERE / 'sample.txt').read_text().splitlines()
    if args.limit:
        parts = parts[:args.limit]
    if any(not re.fullmatch(r'featured_part_\d{5}', p) for p in parts) or len(set(parts)) != len(parts):
        parser.error('sample must contain unique featured_part_NNNNN names')
    cache = args.cache.expanduser().resolve()
    if cache.is_relative_to(REPO):
        parser.error('--cache must be outside the repository')
    cache.mkdir(parents=True, exist_ok=True)
    tools = args.tools.resolve()
    # Build once before workers, avoiding concurrent initial Cargo builds.
    print("Building release process example...", file=sys.stderr)
    process = build_process()
    meter = InstructionMeter()
    import threading
    lock = threading.Lock()
    wall_start = time.monotonic()
    remote = None if args.zip else RemoteZip(cache=cache)
    source = args.zip or io.BufferedReader(remote, buffer_size=1 << 20)
    rows, instructions = [], {}
    with zipfile.ZipFile(source) as z, ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run_part, part, z, cache, tools, lock, process, meter) for part in parts]
        for future in as_completed(futures):
            row, counts = future.result()
            rows.append(row)
            instructions[row['part']] = counts
            print(f"{len(rows)}/{len(parts)} {row['part']}: {row['error'] or 'ok'}", file=sys.stderr)
    report = {'schema_version': 2, 'data': {
        'dataset': 'doi:10.5281/zenodo.21653081',
        'tools_sha256': hashlib.sha256(tools.read_bytes()).hexdigest(),
        'parts': sorted(rows, key=lambda p: p['part']), 'totals': totals(rows)},
        'instructions': {'parts': instructions, 'stages': instruction_summary(instructions)},
        'timing': {'wall_seconds': round(time.monotonic() - wall_start, 6)}}
    if remote:
        report['fetch'] = {'range_requests': remote.requests, 'bytes_received': remote.bytes}
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps(report['data']['totals'], indent=2, sort_keys=True))
    return int(any(p['error'] for p in rows))


if __name__ == '__main__':
    sys.exit(main())
