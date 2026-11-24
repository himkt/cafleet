"""Read-only client for a live runner's control socket (SPEC §6.9 Input control frames).

It sends the lifecycle `health` request, which carries no expiration and changes no state, so the
harness can wait for quiescence without influencing the run.
"""
import base64
import hashlib
import json
import os
import socket
import sqlite3
import struct


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def namespace(database_identity):
    digest = hashlib.sha256(canonical(database_identity).encode()).digest()
    return base64.b32encode(digest).decode().rstrip("=").lower()[:26]


def live_run(db_path, member_id):
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        row = conn.execute(
            "SELECT database_identity_json, fleet_id, member_id, director_member_id, run_id, owner_epoch, launch_nonce "
            "FROM member_runs WHERE member_id=? AND status NOT IN ('exited','failed','lost')", (member_id,)).fetchone()
    finally:
        conn.close()
    if row is None:
        return None
    database_identity = json.loads(row[0])
    identity = {"database_identity_json": database_identity, "fleet_id": row[1], "member_id": row[2],
                "director_member_id": row[3], "run_id": row[4], "owner_epoch": row[5], "launch_nonce": row[6]}
    directory = os.path.dirname(database_identity["canonical_path"])
    path = os.path.join(directory, ".cafleet-runs", namespace(database_identity), str(row[4]), "control.sock")
    return identity, path


def exchange(path, body, timeout=1.0):
    payload = json.dumps(body, separators=(",", ":")).encode()
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    try:
        sock.connect(path)
        sock.sendall(struct.pack(">I", len(payload)) + payload)
        header = recv_exact(sock, 4)
        return json.loads(recv_exact(sock, struct.unpack(">I", header)[0]))
    finally:
        sock.close()


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("runner closed the control socket")
        buf += chunk
    return buf


def health(db_path, member_id, timeout=1.0):
    """The runner's health result, or None when the member has no live run."""
    run = live_run(db_path, member_id)
    if run is None:
        return None
    identity, path = run
    reply = exchange(path, {"protocol_version": 1, "kind": "health", "identity": identity}, timeout)
    if "result" not in reply:
        raise RuntimeError(f"health failed: {reply}")
    return reply["result"]
