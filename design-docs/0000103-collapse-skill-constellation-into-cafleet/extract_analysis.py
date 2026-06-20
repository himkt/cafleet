import json
import sys

src = "/home/himkt/.claude/projects/-home-himkt-work-himkt-cafleet/27c991f5-3ccc-4656-932b-2e19adcb678a/subagents/workflows/wf_ac2c7651-b12/agent-a49a78cfdf6e7a4a2.jsonl"
out = "/home/himkt/work/himkt/cafleet/design-docs/0000103-merge-monitoring-into-supervision/analysis.md"

best = None
with open(src, encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        msg = obj.get("message")
        if not isinstance(msg, dict):
            continue
        content = msg.get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use" and block.get("name") == "Write":
                inp = block.get("input", {})
                fp = inp.get("file_path", "")
                if fp.endswith("analysis.md"):
                    text = inp.get("content", "")
                    # keep the longest (most complete) attempt
                    if best is None or len(text) > len(best):
                        best = text

if best is None:
    print("NO_WRITE_FOUND", file=sys.stderr)
    sys.exit(1)

with open(out, "w", encoding="utf-8") as fh:
    fh.write(best)

print(f"WROTE {len(best)} chars to {out}")
