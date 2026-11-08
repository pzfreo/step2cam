"""Read selected ZIP members; never accept a server's full-download fallback."""
import io
import os
from pathlib import Path, PurePosixPath
import re
import tempfile
import urllib.request

URL = 'https://zenodo.org/api/records/21653081/files/MachinePlan-10K.zip/content'
MAX_MEMBER = 128 * 1024 * 1024


class RemoteZip(io.RawIOBase):
    def __init__(self, url=URL):
        super().__init__()
        self.url, self.pos = url, 0
        self.requests, self.bytes = 0, 0
        # A one-byte range discovers size without trusting HEAD support or redirects.
        data, self.size = self._range(0, 0)

    def _range(self, start, end):
        req = urllib.request.Request(self.url, headers={
            'Range': f'bytes={start}-{end}', 'Accept-Encoding': 'identity'})
        with urllib.request.urlopen(req, timeout=120) as response:
            expected = f'bytes {start}-{end}/'
            header = response.headers.get('Content-Range', '')
            if response.status != 206 or not header.startswith(expected):
                raise OSError(f'server did not honour range {start}-{end}: {response.status}, {header}')
            size = int(header[len(expected):])
            data = response.read(end - start + 2)
            if len(data) != end - start + 1:
                raise OSError('short or oversized HTTP range response')
        self.requests += 1
        self.bytes += len(data)
        return data, size

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, offset, whence=0):
        pos = offset if whence == 0 else self.pos + offset if whence == 1 else self.size + offset
        if whence not in (0, 1, 2) or pos < 0:
            raise ValueError('invalid seek')
        self.pos = pos
        return pos

    def read(self, n=-1):
        if self.pos >= self.size or n == 0:
            return b''
        end = self.size - 1 if n < 0 else min(self.pos + n, self.size) - 1
        data, size = self._range(self.pos, end)
        if size != self.size:
            raise OSError('remote ZIP size changed during read')
        self.pos += len(data)
        return data

    def readinto(self, b):
        data = self.read(len(b))
        b[:len(data)] = data
        return len(data)


def cache_path(cache, member):
    """Reject traversal, Windows paths, and existing symlinks outside the cache."""
    path = PurePosixPath(member.replace('\\', '/'))
    if path.is_absolute() or '..' in path.parts or re.match(r'^[A-Za-z]:', str(path)):
        raise ValueError(f'unsafe ZIP member: {member!r}')
    root = Path(cache).resolve()
    dest = root.joinpath(*path.parts).resolve()
    if not dest.is_relative_to(root) or dest == root:
        raise ValueError(f'unsafe ZIP member: {member!r}')
    return dest


def extract(z, member, cache):
    dest = cache_path(cache, member)
    info = z.getinfo(member)
    if info.file_size > MAX_MEMBER:
        raise ValueError(f'ZIP member too large: {member}')
    if dest.is_file() and dest.stat().st_size == info.file_size:
        # ZipFile verifies CRC when first fetched; cached data are immutable inputs.
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = None
    try:
        with tempfile.NamedTemporaryFile(dir=dest.parent, delete=False) as out:
            tmp = out.name
            with z.open(info) as src:
                remaining = MAX_MEMBER
                while chunk := src.read(min(1024 * 1024, remaining + 1)):
                    remaining -= len(chunk)
                    if remaining < 0:
                        raise ValueError('expanded ZIP member too large')
                    out.write(chunk)
        os.replace(tmp, dest)
    finally:
        if tmp and os.path.exists(tmp):
            os.unlink(tmp)
    return dest
