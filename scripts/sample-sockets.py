#!/usr/bin/env python3
"""Sample the sockets of a process (and its descendants) on Linux — the proof behind "Parlo itself never opens a socket"
(DECISIONS C-021). Used by scripts/check-offline.sh around the app self-test.

  scripts/sample-sockets.py --pid PID [--interval 0.03] [--out report.json]

Every `interval` seconds, until PID exits: map the `socket:[inode]` fds of PID and of each descendant to the kernel's
inet tables (/proc/net/{tcp,tcp6,udp,udp6,raw,raw6}). Unix-domain sockets (X11, D-Bus) are not "network" and are ignored.
Report: samples taken, inet sockets seen in PID itself (must be empty), and in descendants (the download child — a
positive control that the sampler can see sockets at all). Exit status 0 always; the caller asserts.
"""
import argparse
import json
import os
import sys
import time

TABLES = ["tcp", "tcp6", "udp", "udp6", "raw", "raw6"]


def inet_inodes():
    """inode -> (table, local, remote) for every inet socket in this network namespace."""
    out = {}
    for t in TABLES:
        try:
            with open(f"/proc/net/{t}") as f:
                next(f)
                for line in f:
                    p = line.split()
                    if len(p) > 9:
                        out[p[9]] = (t, p[1], p[2])
        except OSError:
            pass
    return out


def fd_inodes(pid):
    res = set()
    d = f"/proc/{pid}/fd"
    try:
        for fd in os.listdir(d):
            try:
                l = os.readlink(f"{d}/{fd}")
            except OSError:
                continue
            if l.startswith("socket:["):
                res.add(l[8:-1])
    except OSError:
        pass
    return res


def children_of(root):
    """All descendants of root (pid -> comm)."""
    ppid = {}
    comm = {}
    for e in os.listdir("/proc"):
        if not e.isdigit():
            continue
        try:
            with open(f"/proc/{e}/stat") as f:
                s = f.read()
            r = s.rindex(")")
            comm[int(e)] = s[s.index("(") + 1 : r]
            ppid[int(e)] = int(s[r + 2 :].split()[1])
        except (OSError, ValueError):
            continue
    found = {}
    todo = [root]
    while todo:
        p = todo.pop()
        for c, pp in ppid.items():
            if pp == p and c not in found:
                found[c] = comm.get(c, "?")
                todo.append(c)
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--interval", type=float, default=0.03)
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    own, child = {}, {}
    names = set()
    samples = 0
    while os.path.exists(f"/proc/{a.pid}"):
        table = inet_inodes()
        for ino in fd_inodes(a.pid):
            if ino in table:
                own[ino] = table[ino]
        for cpid, name in children_of(a.pid).items():
            names.add(name)
            for ino in fd_inodes(cpid):
                if ino in table:
                    child[f"{name}:{ino}"] = table[ino]
        samples += 1
        time.sleep(a.interval)
    rep = {
        "samples": samples,
        "own_inet": sorted(f"{t} {l} -> {r}" for t, l, r in own.values()),
        "child_inet": sorted(set(f"{k.split(':')[0]} {t} -> {r}" for k, (t, l, r) in child.items())),
        "descendants_seen": sorted(names),
    }
    text = json.dumps(rep, indent=2)
    if a.out:
        open(a.out, "w").write(text)
    print(text, file=sys.stderr)


if __name__ == "__main__":
    main()
