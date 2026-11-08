import copy
import io
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import family, normalise, summarise_part, totals
from compare import compare
from fetch import RemoteZip, cache_path, extract


class AccountingTests(unittest.TestCase):
    def test_families(self):
        for group, expected in [('STEP1HOLE_1', 'hole'), ('STEP1POCKET_2', 'hole'),
                                ('FG_CHAMFER_SURFACE_1', 'chamfer'), ('POCKET_RECT_1', 'pocket'),
                                ('SLOT_1', 'slot'), ('CORNER_NOTCH_2', 'notch'),
                                ('STEP123', 'other'), ('MILL_GEOM', 'other')]:
            self.assertEqual(family(group), expected)

    def test_normalise(self):
        self.assertEqual(normalise('depth 12.5 below Z-3; 2e-05 mm; top-12-wall'),
                         'depth # below Z-#; # mm; top-#-wall')
        self.assertEqual(normalise('no roughing was run'), 'no roughing was run')

    def test_outcomes(self):
        analysis = {'features': [{'id': str(i), 'family': 'holes'} for i in range(5)]}
        output = {'plan': {'setups': [{'operations': [{'features': ['0'], 'feature': '1'}]}],
                           'covered_by_roughing': ['2'],
                           'skipped': [{'feature': '3', 'reason': 'depth 12'}]}}
        features, warnings = summarise_part(analysis, output)
        self.assertEqual([f['outcome'] for f in features],
                         ['cut', 'cut', 'covered', 'left_out', 'unaccounted'])
        self.assertEqual(features[3]['reason'], 'depth #')


class CompareTests(unittest.TestCase):
    def report(self):
        part = {'part': 'p', 'error': None, 'features': [{'id': 'h', 'family': 'holes', 'outcome': 'cut'}],
                'label_counts': {'hole': 1}, 'recognised_counts': {'holes': 1}, 'warnings': [],
                'recognition_complete': True, 'labels_complete': True}
        return {'data': {'parts': [part], 'totals': totals([part])}, 'timing': {'total_seconds': 1}}

    def test_identical_and_timing(self):
        a = self.report()
        b = copy.deepcopy(a)
        b['timing']['total_seconds'] = 20
        lines = []
        self.assertEqual(compare(a, b, lines.append), 0)
        self.assertIn('No change (timings ignored).', lines)

    def test_fewer_cut_is_review_only(self):
        a = self.report()
        b = copy.deepcopy(a)
        b['data']['parts'][0]['features'][0].update(outcome='left_out', reason='no tool')
        b['data']['totals'] = totals(b['data']['parts'])
        lines = []
        self.assertEqual(compare(a, b, lines.append), 0)
        self.assertTrue(any('FEWER FEATURES CUT' in line for line in lines))
        self.assertTrue(any('before:' in line and 'after:' in line for line in lines))

    def test_errors(self):
        a = self.report()
        b = copy.deepcopy(a)
        b['data']['parts'][0]['error'] = 'failed 12'
        self.assertEqual(compare(a, b, lambda _: None), 1)
        self.assertEqual(compare(b, b, lambda _: None), 0)
        c = copy.deepcopy(b)
        c['data']['parts'][0]['error'] = 'different failure'
        self.assertEqual(compare(b, c, lambda _: None), 1)
        self.assertEqual(compare(b, a, lambda _: None), 0)


class FetchTests(unittest.TestCase):
    def test_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            for member in ('../escape', '/absolute', r'..\escape', 'C:/escape', 'a/../../escape'):
                with self.assertRaises(ValueError):
                    cache_path(tmp, member)
            self.assertEqual(cache_path(tmp, 'a/./b'), Path(tmp).resolve() / 'a/b')
            (Path(tmp) / 'link').symlink_to(Path(tmp).parent, target_is_directory=True)
            with self.assertRaises(ValueError):
                cache_path(tmp, 'link/escape')

    def test_extract_only_requested_member(self):
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, 'w') as z:
            z.writestr('part/data', b'hello')
            z.writestr('../escape', b'bad')
        stream.seek(0)
        with tempfile.TemporaryDirectory() as tmp, zipfile.ZipFile(stream) as z:
            self.assertEqual(extract(z, 'part/data', tmp).read_bytes(), b'hello')
            with self.assertRaises(ValueError):
                extract(z, '../escape', tmp)

    def test_reject_server_ignoring_range(self):
        class Response:
            status = 200
            headers = {}
            def __enter__(self):
                return self
            def __exit__(self, *args):
                pass
            def read(self, n):
                raise AssertionError('must reject before reading a full ZIP')
        with patch('urllib.request.urlopen', return_value=Response()):
            with self.assertRaises(OSError):
                RemoteZip('https://example.invalid/data.zip')


if __name__ == '__main__':
    unittest.main()
