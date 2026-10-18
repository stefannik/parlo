#!/usr/bin/env python3
"""Tiny local HTTP file server for the "Transcribe from URL" tests (stdlib only; binds 127.0.0.1 only).

  scripts/fixture-server.py DIR [--port 0] [--bps N] [--no-head]
Prints `PORT <n>` once it is listening, then serves until killed. Features the downloaders rely on:
  * Range requests (206), Content-Length, Content-Type from the extension (m4a → audio/mp4, mp3 → audio/mpeg, …)
  * /redirect/<file>  → 302 to /<file>          * /status/<code> → that status
  * /html/<anything>  → a text/html page         * --bps N throttles bodies to N bytes/s (makes progress observable)
  * --no-head         → HEAD answers 405 (some real podcast hosts do)
"""
import argparse
import mimetypes
import os
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

mimetypes.add_type("audio/mp4", ".m4a")
mimetypes.add_type("audio/webm", ".webm")
mimetypes.add_type("audio/mpeg", ".mp3")
mimetypes.add_type("audio/ogg", ".ogg")


class H(BaseHTTPRequestHandler):
    root: Path
    bps: int = 0
    no_head: bool = False
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # quiet
        pass

    def _send(self, code, headers, body=b""):
        self.send_response(code)
        for k, v in headers.items():
            self.send_header(k, v)
        self.end_headers()
        if body and self.command != "HEAD":
            self.wfile.write(body)

    def _handle(self):
        path = self.path.split("?", 1)[0]
        if path.startswith("/status/"):
            code = int(path.rsplit("/", 1)[1])
            return self._send(code, {"Content-Length": "0", "Connection": "close"})
        if path.startswith("/html/"):
            b = b"<html><head><title>Not media</title></head><body>hello</body></html>"
            return self._send(200, {"Content-Type": "text/html", "Content-Length": str(len(b))}, b)
        if path.startswith("/redirect/"):
            return self._send(302, {"Location": "/" + path[len("/redirect/"):], "Content-Length": "0"})
        rel = os.path.normpath(path.lstrip("/"))
        f = (self.root / rel).resolve()
        if not str(f).startswith(str(self.root.resolve())) or not f.is_file():
            return self._send(404, {"Content-Length": "0", "Connection": "close"})
        size = f.stat().st_size
        ctype = mimetypes.guess_type(str(f))[0] or "application/octet-stream"
        start, end, code = 0, size - 1, 200
        m = re.match(r"bytes=(\d*)-(\d*)", self.headers.get("Range", ""))
        if m and (m.group(1) or m.group(2)):
            if m.group(1):
                start = int(m.group(1))
                if m.group(2):
                    end = min(size - 1, int(m.group(2)))
            else:
                start = max(0, size - int(m.group(2)))
            code = 206
        if start >= size:
            return self._send(416, {"Content-Range": f"bytes */{size}", "Content-Length": "0"})
        n = end - start + 1
        h = {"Content-Type": ctype, "Content-Length": str(n), "Accept-Ranges": "bytes"}
        if code == 206:
            h["Content-Range"] = f"bytes {start}-{end}/{size}"
        if self.command == "HEAD":
            return self._send(code, h)
        self.send_response(code)
        for k, v in h.items():
            self.send_header(k, v)
        self.end_headers()
        try:
            with f.open("rb") as fh:
                fh.seek(start)
                left = n
                chunk = 16384 if not self.bps else max(1024, self.bps // 10)
                while left > 0:
                    b = fh.read(min(chunk, left))
                    if not b:
                        break
                    self.wfile.write(b)
                    left -= len(b)
                    if self.bps:
                        time.sleep(len(b) / self.bps)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        self._handle()

    def do_HEAD(self):
        if self.no_head:
            return self._send(405, {"Content-Length": "0"})
        self._handle()


class Quiet(ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, request, client_address):  # clients hang up mid-transfer on cancel: not an error here
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--bps", type=int, default=0)
    ap.add_argument("--no-head", action="store_true")
    a = ap.parse_args()
    H.root, H.bps, H.no_head = Path(a.dir), a.bps, a.no_head
    srv = Quiet(("127.0.0.1", a.port), H)
    print(f"PORT {srv.server_address[1]}", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
