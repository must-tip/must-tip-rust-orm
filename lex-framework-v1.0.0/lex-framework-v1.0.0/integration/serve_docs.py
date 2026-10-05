#!/usr/bin/env python3
"""Serve the built site under its GitHub Pages project prefix, without SPA fallback."""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', type=int, default=4187)
    args = parser.parse_args()
    site = Path(__file__).resolve().parents[1] / 'docs-site/dist'
    if not (site / 'index.html').is_file():
        parser.error('build docs-site with npm run build before starting browser tests')
    with TemporaryDirectory(prefix='lex-docs-preview-') as temporary:
        (Path(temporary) / 'must-tip-rust-orm').symlink_to(site, target_is_directory=True)
        handler = partial(SimpleHTTPRequestHandler, directory=temporary)
        with ThreadingHTTPServer(('127.0.0.1', args.port), handler) as server:
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass


if __name__ == '__main__':
    main()
