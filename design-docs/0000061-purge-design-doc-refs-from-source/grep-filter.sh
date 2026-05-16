#!/bin/bash
# Helper to enumerate non-exempt design-doc citations.
grep -rEn 'design-docs/[0-9]{7}-|design 0[0-9]{6}|per design [0-9]|see design [0-9]|added in design [0-9]|deprecated in design [0-9]' /home/himkt/work/himkt/cafleet \
  --include='*.md' --include='*.py' --include='*.ts' --include='*.tsx' --include='*.js' --include='*.json' --include='*.toml' --include='*.sh' --include='.gitignore' \
  --exclude-dir=design-docs --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=.venv --exclude-dir=dist \
  | grep -vE '(^/home/himkt/work/himkt/cafleet/(CLAUDE\.md|drafter\.md|reviewer\.md|programmer\.md|tester\.md|director-answers\.md|drafter-questions\.md):|/skills/design-doc[/-]|/\.claude/rules/design-doc-numbering\.md:|/cafleet/tests/test_base_dir|/cafleet/src/cafleet/base_dir\.py:)'
