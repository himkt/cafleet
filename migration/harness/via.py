"""Run cafleet as the child of a process whose argv carries a shared-server signature.

  via.py PIDFILE TOKEN... -- BINARY ARG...

The tokens (for example `codex app-server`) stay in this process's argv, so the cafleet ancestry
walk sees an ancestor that matches the signature. stdio is inherited; the exit code is relayed.
"""
import os
import subprocess
import sys

pidfile = sys.argv[1]
split = sys.argv.index("--")
binary, args = sys.argv[split + 1], sys.argv[split + 2:]
child = subprocess.Popen(["cafleet", *args], executable=binary)
with open(pidfile, "w") as f:
    f.write(str(child.pid))
code = child.wait()
sys.exit(code if code >= 0 else 128 - code)
