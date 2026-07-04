# SPEC.md / docs vs Implementation — Consistency Audit Findings

Source: multi-agent static audit (9 module finders + adversarial verification). 49 confirmed
findings de-duplicated into 32 contradictions: 9 high, 13 medium, 10 low. Paths relative to
repo root `/Users/himkt/work/himkt/cafleet`.

**Resolution policy from the user:** the implementation is basically correct — default to
aligning docs (SPEC.md + docs/) to the code. For the small set of items where the doc/SPEC is
actually correct and the *code* is the buggy side, the design doc MUST flag them explicitly and
propose a code fix (surfaced to the user for confirmation), NOT silently rewrite the doc to match
a bug.

---

## Cluster 1 — Persistence schema / `to_agent_id`

### 1.1 [HIGH] `tasks.to_agent_id` nullable-with-NULL vs NOT NULL + `0` sentinel  *(merges #1,#2,#6,#23,#38)*
- Doc: `SPEC.md:299`, `SPEC.md:371-379` (§5.5 "resolved"), `SPEC.md:662-663`, `SPEC.md:2612`; `docs/spec/data-model.md:80`; `docs/spec/message-envelope.md:87`
- Code: `cafleet/src/cafleet/db/models.py:73` (`nullable=False`); `cafleet/src/cafleet/db/alembic/versions/0001_initial_schema.py:64` (`nullable=False`); `cafleet/src/cafleet/broker/messaging.py:218` (`"to_agent_id": 0` on broadcast_summary rows)
- Docs say nullable, NULL on broadcast_summary, "no 0 sentinel". Code is NOT NULL and writes `0`. Observable in `--json`.
- Which is wrong: CONTESTED (verifiers 3 code / 2 doc). SPEC §5.5 labels NULL-no-sentinel "resolved". **Needs user decision.** Items 1.2 resolve with it.

### 1.2 [MEDIUM] Truthiness check vs `is None`/`IS NULL` for surfacing `to:`  *(merges #3,#8,#25,#45)*
- Doc: `SPEC.md:374-379`, `706-708`, `1525-1526`, `1573-1576`
- Code: `cafleet/src/cafleet/broker/queries.py:68-70` (`if to_id:`); `cafleet/src/cafleet/output/formatters.py:39-40` (`if task.get("to_agent_id"):`)
- SPEC forbids truthiness; code uses it. Mechanically coupled to 1.1. CONTESTED (2/2). Resolve together.

### 1.3 [MEDIUM] tasks indexes documented `status_timestamp DESC`; actual ascending  *(#4)*
- Doc: `docs/spec/data-model.md:92-93`
- Code: `0001_initial_schema.py:77-87`; `db/models.py:81-84`. `SPEC.md:2424-2427` matches code.
- Which is wrong: doc.

---

## Cluster 2 — Broker

### 2.1 [HIGH] Broadcast result missing `recipients`/`delivered` keys; text omits `delivered=`, mislabels count  *(merges #5,#14,#24)*
- Doc: `SPEC.md:670-674`, `1013-1022`; `docs/spec/message-envelope.md:74`; `docs/spec/cli-options.md:557-561` (also `:81`,`:131`)
- Code: `cafleet/src/cafleet/broker/messaging.py:253-255` (single `notifications_sent_count`); `cafleet/src/cafleet/cli/message.py:70-75` (prints `recipients={notifications_sent_count}`, no `delivered=`)
- Docs contract separate `recipients` (real N) / `delivered` (preview-success k), never conflated; code returns one key and CLI labels preview count as `recipients`. `--json` consumers break.
- Which is wrong: code (2 of 3; one unclear).

### 2.2 [HIGH] Root-Director deregistration guard raises `click.UsageError` (exit 2), not exit-1 app error  *(merges #7,#43)*
- Doc: `SPEC.md:596-603`, `804`, `2348`, `2352-2354`
- Code: `cafleet/src/cafleet/broker/agents.py:272-276` (`click.UsageError`); propagated by `cli/member.py:435-438`. Parallel CLI guard `cli/member.py:328-331` correctly uses `ClickException` (exit 1).
- Which is wrong: code.

### 2.3 [HIGH] §5.4 two-value kind projection; `get_agent` returns 4 values incl `"monitor"`, `list_agents` returns none  *(merges #10,#39)*
- Doc: `SPEC.md:362-366` (§5.4), `514-517`
- Code: `broker/agents.py:217-219` (`get_agent` → derive_agent_kind), `241-250` (`list_agents` no kind), `397-401` (`list_fleet_agents` collapses). SPEC per-function bullets `588-595`,`611-614` already match code.
- Which is wrong: doc (summary paragraphs are the drift).

### 2.4 [MEDIUM] Kind predicates crash on null/non-object `cafleet` card instead of returning false  *(#9)*
- Doc: `SPEC.md:511-514`
- Code: `broker/_shared.py:34-51` (only `except ValueError`) → `AttributeError` on `{"cafleet": null}` / `{"cafleet":"x"}`.
- Which is wrong: code.

### 2.5 [MEDIUM] fleet-isolation.md "always indistinguishable not found"; code distinguishes "not in fleet"  *(#11)*
- Doc: `docs/concepts/fleet-isolation.md:16-17`
- Code: `broker/messaging.py:150-153` (distinct "not found" vs "not in fleet"). `SPEC.md:810-811` documents both strings.
- Which is wrong: doc.

### 2.6 [LOW] api/broker.md layout omits `monitor.py`, `skill_installs.py`; re-export contract inaccurate; `list_roster` missing  *(#12)*
- Doc: `docs/api/broker.md:15-22`, `26-30`
- Code: `broker/__init__.py:27-41`; `broker/monitor.py`; `broker/skill_installs.py`; `broker/members.py:66`.
- Which is wrong: doc.

---

## Cluster 3 — CLI

### 3.1 [HIGH] `fleet show`/`fleet delete` take positional `FLEET_ID`, not `--fleet-id`  *(merges #13,#41,#42)*
- Doc: `SPEC.md:853-856`,`983-985`,`995`,`999`,`2536-2537`,`2565-2568`; `docs/spec/cli-options.md:87`,`295`,`376`,`396`; `docs/get-started/quickstart.md:134`; `docs/get-started/configure.md:110-111`
- Code: `cli/fleet.py:82-86` (show), `105-107` (delete) — `@click.argument("fleet_id", type=int)`; no `fleet_id_option`.
- Documented `cafleet fleet delete --fleet-id 1` fails ("No such option"). Which is wrong: code. ALSO doc-side: `configure.md:110-111` blanket "every fleet-scoped command takes --fleet-id" is wrong for `fleet create`/`fleet list` too — fix regardless.

### 3.2 [HIGH] Missing `--fleet-id`: cli-options.md documents Click exit-2 usage error; code raises exit-1 custom  *(#15)*
- Doc: `docs/spec/cli-options.md:1029`
- Code: `cli/_helpers.py:57-78` (`default=None` + callback `ClickException`, exit 1). `SPEC.md:845-847` matches code.
- Which is wrong: doc.

### 3.3 [HIGH] `member create` into soft-deleted fleet exits 2 (UsageError), not documented exit 1  *(#17)*
- Doc: `docs/spec/cli-options.md:1033`
- Code: `broker/agents.py:65-66` (`click.UsageError`), re-raised by `cli/member.py:242-245`.
- Which is wrong: doc. NOTE tension with 2.2 — maintainer may prefer normalizing both broker guards to `ClickException`.

### 3.4 [MEDIUM] Hidden `--quiet` exists on send/ack/ping; docs say no `--quiet` flag exists  *(#18)*
- Doc: `docs/spec/cli-options.md:146`,`899-901`; `SPEC.md:1007`
- Code: `cli/_helpers.py:48` (hidden `quiet_flag`) at `cli/message.py:34`,`110`, `cli/member.py:635`. `SPEC.md:1210-1212` documents ping flag.
- Which is wrong: doc.

### 3.5 [MEDIUM] `--full` hidden in code on many subcommands; docs call it "documented"; SPEC §10 claims no hidden flags  *(#19)*
- Doc: `docs/spec/cli-options.md:76`,`304`,`636`; `SPEC.md:989`,`1007`,`1434-1435`,`2517-2518`
- Code: `cli/_helpers.py:47` (`full_flag hidden=True`) at fleet/member/message; only `member show` visible `--full`. Also hidden: `--quiet`, `--activity`, `--ansi/--no-ansi`.
- Which is wrong: both (SPEC §10 wrong even vs §6.3). Prefer aligning docs to code (keep flags hidden) unless user wants them unhidden.

### 3.6 [LOW] `fleet list` widths: SPEC 40/20/8; code 4 cols 40/40/20/8; cli-options sample spacing impossible  *(#21)*
- Doc: `SPEC.md:992-994`; `docs/spec/cli-options.md:367-370`
- Code: `cli/fleet.py:71-79`.
- Which is wrong: doc.

---

## Cluster 4 — Output / formatters

### 4.1 [HIGH] Ping-age humanizer renders EM DASH `—`; docs mandate single ASCII `-` absent glyph  *(merges #20,#22,#44)*
- Doc: `SPEC.md:1495-1503` (explicit "no EM DASH"), `1633`,`1648-1651`,`2481-2483`,`2618`; `docs/spec/cli-options.md:982`,`985`
- Code: `output/formatters.py:216-220` (`_format_ping_age` returns U+2014), used at `:252`. Sibling helpers all use `-`.
- Which is wrong: code.

### 4.2 [MEDIUM] Compact envelope `status_state` "conditionally omitted"; `render_task` omits unconditionally  *(#26)*
- Doc: `docs/spec/message-envelope.md:31`
- Code: `output/render.py:77-89`. `SPEC.md:1457`,`1531` side with code.
- Which is wrong: doc.

### 4.3 [MEDIUM] token-reduction.md misdescribes default and `--full` agent renders  *(#46)*
- Doc: `docs/concepts/token-reduction.md:25`
- Code: `output/formatters.py:73-100`; `cli/member.py:481-484`. `SPEC.md:1578-1585`,`1163` match code.
- Which is wrong: doc.

### 4.4 [LOW] SPEC lists `coding_agent` among truthiness-gated conditional fields; always emitted  *(#27)*
- Doc: `SPEC.md:1652`
- Code: `broker/_shared.py:73-81` (always includes); required subscripts at `formatters.py:93,148,154,297,329`. SPEC field table `1550-1551` marks it required.
- Which is wrong: doc.

---

## Cluster 5 — Multiplexer

### 5.1 [HIGH] tmux-guard error string "tmux-pane commands…" vs code "member commands…"  *(merges #16,#28,#40)*
- Doc: `SPEC.md:1668-1670` (§6.5), `2379-2381` (§7.2 verbatim-error contract); `docs/spec/cli-options.md:420`
- Code: `multiplexer/tmux.py:128-129` (surfaced by `cli/_helpers.py:17-21`, `cli/doctor.py:32`). Five test files assert "member" wording.
- Which is wrong: doc (leftover from tmux-pane → member rename).

### 5.2 [MEDIUM] `send_bash_command` also rejects CR; SPEC specifies newline-only  *(#29)*
- Doc: `SPEC.md:1718-1721`
- Code: `multiplexer/tmux.py:313-314` (`"\n" in command or "\r" in command`). SPEC CLI phrasing `1200` says "newline/CR".
- Which is wrong: doc.

---

## Cluster 6 — Monitor

### 6.1 [MEDIUM] monitoring.md heartbeat as LAST tick step; code runs it FIRST  *(#30)*
- Doc: `docs/concepts/monitoring.md:180`,`194-196`
- Code: `monitor/loop.py:73-113` (heartbeat first, STOP on zero-row). `SPEC.md:1866-1871` matches code.
- Which is wrong: doc.

### 6.2 [LOW] §6.6 markers `Continue`/`Stop`; code exports `CONTINUE`/`STOP`  *(#31)*
- Doc: `SPEC.md:1829`,`1833-1834` (recurs 1870,1873,1899,1917,1924)
- Code: `monitor/loop.py:34-35`. `SPEC.md:2492` itself uses `STOP`.
- Which is wrong: doc.

### 6.3 [LOW] §6.6 "the four functions are public"; module has three  *(#32)*
- Doc: `SPEC.md:1841-1843`
- Code: `monitor/loop.py:38,60,134`. SPEC bullet list `1827-1832` enumerates three.
- Which is wrong: doc.

---

## Cluster 7 — WebUI API

### 7.1 [MEDIUM] webui-api.md timeline scoped by recipient (`context_id`); code scopes by sender (`from_agent_id`)  *(#34)*
- Doc: `docs/spec/webui-api.md:232`
- Code: `broker/queries.py:33-42` (join on `from_agent_id`) at `webui/api.py:192`. `SPEC.md:702-704` confirms sender join.
- Which is wrong: doc.

### 7.2 [MEDIUM] `GET /api/fleets` shape omits `director_agent_id`; says "all" but excludes soft-deleted  *(#35)*
- Doc: `docs/spec/webui-api.md:23-36`
- Code: `broker/fleets.py:126-164` (`deleted_at IS NULL`; five-key rows incl `director_agent_id`) at `webui/api.py:114-116`. `SPEC.md:546-549`,`2222-2223` match code.
- Which is wrong: doc.

### 7.3 [LOW] §6.8 `/api/timeline` "all messages"; hard-capped at 200  *(#36)*
- Doc: `SPEC.md:2254-2255`
- Code: `broker/queries.py:19,41` (`limit=200`). SPEC §6.2 + `webui-api.md:258` correct.
- Which is wrong: doc.

### 7.4 [LOW] §6.8 send checks "not in fleet"; code also requires `status=="active"`  *(#37)*
- Doc: `SPEC.md:2259-2263`
- Code: `webui/api.py:201-210` via `broker/agents.py:186-192`. `webui-api.md:293`,`310-311` correct.
- Which is wrong: doc.

---

## Cluster 8 — Get-started / reference

### 8.1 [MEDIUM] opencode.md `member capture` example missing mandatory `--fleet-id`  *(#33)*
- Doc: `docs/reference/coding-agents/opencode.md:84`
- Code: `cli/member.py:530-532` (`@fleet_id_option`); `cli/_helpers.py:57-67`.
- Which is wrong: doc.

### 8.2 [LOW] configure.md Codex ruleset allowlists nonexistent `cafleet agent` group; omits real `member show`  *(#47)*
- Doc: `docs/get-started/configure.md:59`,`76-80`
- Code: `cli/__init__.py:26-32`. `SPEC.md:2593` corroborates.
- Which is wrong: doc.

### 8.3 [LOW] install.md shows bare `gh skill install ./ --from-local`; task adds `--agent`,`--force`,`--scope user`  *(#48)*
- Doc: `docs/get-started/install.md:89`
- Code: `mise.toml:22-27`.
- Which is wrong: doc.

### 8.4 [LOW] contributing.md labels `cafleet:format` as "ruff format" only; task first runs `ruff check --fix`  *(#49)*
- Doc: `docs/get-started/contributing.md:36`
- Code: `cafleet/mise.toml:16-21`.
- Which is wrong: doc.

---

## Action summary

| Fix side | Items |
|---|---|
| Fix code (doc is correct) | 2.1, 2.2, 2.4, 3.1, 4.1 |
| Fix docs (code is correct) | 1.3, 2.3, 2.5, 2.6, 3.2, 3.3, 3.4, 3.6, 4.2, 4.3, 4.4, 5.1, 5.2, 6.1, 6.2, 6.3, 7.1, 7.2, 7.3, 7.4, 8.1, 8.2, 8.3, 8.4, plus configure.md blanket sentence in 3.1 |
| Fix both | 3.5 (hidden flags vs "documented" claims + SPEC §10) |
| Decision needed | 1.1 + 1.2 (`to_agent_id` NULL-vs-0 design; align schema + broker write + both truthiness checks + 4 doc surfaces together) |

Root causes worth noting: several doc drifts trace to the `tmux-pane` → `member` rename (5.1) and
the `fleet-id` positional-vs-option design (3.1, 3.2, 3.3, 8.1); fixing those roots clears multiple rows.
