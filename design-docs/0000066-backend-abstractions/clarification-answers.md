# Clarification Answers — Design Doc 0000066 Backend Abstractions

User answers to the Drafter's 4-chunk clarification round. Each answer references the Drafter's category-letter / question-number scheme.

## A. Purpose & Timing

- **A1 Sequencing**: **Pure refactor first** — this doc lands `cafleet.coding_agent` and `cafleet.multiplexer` with claude+codex behind CodingAgent and tmux behind Multiplexer. **No new impls in this doc.** Cursor CLI and cmux ship in **separate follow-on design docs** that consume the abstraction. The Drafter's rationale (test surface is large, each new impl has its own integration questions, joint review is harder) was accepted.
- **A2 Module layout**: **Subpackages.** `cafleet/coding_agent/{__init__.py, base.py, claude.py, codex.py}` and `cafleet/multiplexer/{__init__.py, base.py, tmux.py}`. Each backend impl gets its own file so adding `cursor.py` / `cmux.py` later is a single new file.

## B. API / Interface

- **B1 CodingAgent base**: **Protocol** — duck-typed, no runtime overhead, easier to mock. The Drafter recommended ABC; the user explicitly chose Protocol. Implication: shared logic (e.g., the default `ensure_available()` that runs `shutil.which(self.binary_name)`) cannot live as a base-class method. It must live as a module-level helper that concrete classes call, or be re-implemented per impl. The Drafter picks the cleanest pattern (module-level helper recommended for the few shared bits).
- **B2 Multiplexer base**: **Protocol** — same reasoning as B1. Implication: helpers like `wait_for_pane_gone` (which composes `pane_exists`) and `_send_literal_then_enter` (which sequences `send-keys -l` + sleep + `send-keys Enter`) cannot live as base-class concrete methods. They live as module-level functions in `cafleet.multiplexer.base` (or `cafleet.multiplexer._common`) that each concrete multiplexer calls. Drafter picks the file split.
- **B3 display_name handling**: **Silently ignore in CodexAgent.** `ClaudeCodeAgent.build_spawn_argv` uses `display_name`; `CodexAgent.build_spawn_argv` accepts the kwarg and ignores it. CLI passes `display_name` unconditionally. No `supports_display_name` flag.
- **B4 Shared helper visibility** (implicit, resolved by B1/B2 Protocol choice): module-level functions, not protected methods. Drafter picks names.
- **B5 Context dataclass**: **Single shared `MultiplexerContext`** with fields `session`, `window_id`, `pane_id`. Today's `tmux.DirectorContext` is renamed and relocated. cmux maps its concepts onto the same slots. Broker imports stay trivial.
- **B6 Registry mechanism**: **Module-level dict.** `CODING_AGENTS: dict[str, CodingAgent] = {"claude": ClaudeCodeAgent(), "codex": CodexAgent()}` and the same shape for `MULTIPLEXERS`. Adding a backend = one dict entry. Simple, greppable.
- **B7 Multiplexer instance lifecycle**: **Module-level singleton.** One `TmuxMultiplexer()` instance lives in the registry dict and is reused. Multiplexer is stateless (each method shells out to tmux), so singleton is natural. Same pattern for CodingAgents.
- **B8 CLI `--coding-agent` Choice list**: **Dynamic from registry.** `click.Choice(list(CODING_AGENTS.keys()))`. Adding cursor in the follow-on design doc becomes one dict entry — no Click decorator edit. The Choice **values** stay `{"claude", "codex"}` in this design doc (since cursor is out of scope here).

## C. Tests

- **C1 Test seam**: **Function-level monkeypatch on the new path.** Tests that today do `monkeypatch.setattr(cafleet.tmux, "X", ...)` will instead do `monkeypatch.setattr(cafleet.multiplexer.tmux, "X", ...)`. Low churn — ~30 import-path updates, no test redesign. A cleaner DI seam (FakeMultiplexer + `set_multiplexer()` hook) is deferred to the cmux follow-on doc.
- **C2 Contract tests**: **Yes — add per-subclass contract tests.** Parametrize over `CODING_AGENTS.values()` and `MULTIPLEXERS.values()`. Each impl is asserted to satisfy the Protocol's method shapes — use `@runtime_checkable` on the Protocols and `isinstance(obj, ProtoClass)` at runtime, plus signature checks for the abstract methods. Pays off the moment cursor/cmux land.

## D. Edge cases & Out-of-scope

- **D1 Root Director placement validation at session-create**: **Stay metadata-only.** Per 0000046 §1: cafleet does not spawn the root Director, so the binary is already running in the calling pane by definition. No `shutil.which` gate added.
- **D2 Legacy `placement.coding_agent = "unknown"` rows**: **Let KeyError surface AND add an Alembic migration script** to backfill existing `"unknown"` rows. **This is a scope addition.** The migration should:
  - INSPECT existing `agent_placements` rows for `coding_agent = "unknown"`.
  - Backfill them to `"claude"` (the pre-0000046 hardcoded default — matches what those rows actually ran).
  - Be idempotent (only updates the `"unknown"` value; leaves `"claude"` / `"codex"` rows untouched).
  - Live as a new Alembic revision under `cafleet/src/cafleet/alembic/versions/` chained off the current head (`0009_drop_task_json_add_text.py`).
  - Drafter assigns the next revision number (`0010_backfill_unknown_coding_agent.py` or similar).
- **D3 Out-of-scope reconfirm**: **Confirmed.** Out of scope: cursor CLI / cmux impls themselves; `agent_placements.coding_agent` schema change (column type/constraints stay; D2's migration only backfills VALUES); CLI flag value surface (Choice **list values** stay `{"claude", "codex"}` until cursor lands — the Choice **computation** becomes dynamic but the values don't change); message broker / task lifecycle / WebUI changes.
- **D4 `broker.py` import pattern**: **Keep the local-import pattern.** Local `from cafleet.multiplexer.tmux import send_inline_preview` inside `_try_notify_recipient` — same pattern, new path. Monkeypatch target shifts; test redesign avoided.
- **D5 `cafleet doctor` scope**: **Multiplexer-aware.** `doctor` calls the registered Multiplexer's `context_discovery()` method and renders the result. When cmux lands, `doctor` automatically reports cmux IDs. Output schema stays `session_name` / `window_id` / `pane_id` (cmux maps onto the shared `MultiplexerContext` slots). The `--json` shape is preserved verbatim.

## Drafting directives

- Address the historical context explicitly in the **Background** section: cite 0000018 (abstraction shipped + superseded), 0000041 §B (abstraction deleted with rationale "no second backend"), 0000046 Background (explicit refusal to restore when codex re-added), 0000056 §P5 (skip on extracting build_*_command helpers). State what is different *this time*: cursor + cmux are concrete planned additions, so the abstraction ships with the path to a second concrete impl already in user-confirmed scope rather than speculative.
- Follow `Skill(design-doc)` template + guidelines.
- File inventory in the brief is a starting point; refine during drafting.
- After writing the initial draft, send `complete (doc)` to the Director — do **not** loop with the Reviewer directly.

Proceed with drafting.
