The fixtures referenced by COMPOSERS in ../../pane_state.rs are synthetic transport inputs, constructed from backend composer shapes with explicit SGR spans. Each synthetic placeholder/draft pair has identical visible text and layout. Tests rotate its text and preserve ambiguous/working/approval behavior independently of installed-host evidence.

The codex-herdr fixtures below are minimal actual composer/footer excerpts. They preserve source SGR bytes and internal spaces, omit preceding transcript lines, normalize CRLF to LF, remove a terminal whitespace-only line where present and end with one LF. Hashes identify both the original raw captures and the normalized excerpts.

The real matched pair has identical trimmed composer text, "› Ask Codex to do anything". Both use default foreground and RGB background 48;2;53;53;53. The placeholder text has dim SGR 2; the actual unsent user input has no dim. Both retain Context 10% used in the model footer. The placeholder has "? for shortcuts", which disappears in the actual draft. Tests require Idle and Draft from those respective roles and compare composer text rather than full-screen equality.

| Excerpt | Source SHA-256 | Excerpt SHA-256 |
|---|---|---|
| codex-herdr-placeholder.ansi | b6d9d24759fcaad9a4e38c821cc6e9d9cb7cd5d46cd1f8f904b20601a3c55878 | 20d260d99454ec0b8560fbea0a045f39230b03d9491a6af9e397ac77e802da66 |
| codex-herdr-draft.ansi | a489a8b7d2344dbd00a38b347c81acbcdae7016f5a75394c1e2f5d9bf2be9a9a | 502e6ecae8546b5e1b18b7b2dbaf07cd1848014f8579af3fb42df05c0be453a9 |
| codex-herdr-placeholder-warning.ansi | 50a0a9e94bb403ac988da31eec74d3f8acdb5d7ab61c5e811a64cbdfd399e948 | 4408885f9de30efa56dd7423f938a51c8aafea6a145b12c596b521b4d2d30848 |

The matched source captures are placeholder.stdout.ansi and draft.stdout.ansi under BASE/.execution/styled-captures/20261023T080502.681580Z-pair36-c5b263b0/, with pair-metadata.json and interpretation-facts.json beside them. Verifier's independent interpretation is BASE/.execution/styled-capture-pair-36-results.md.

| Pair provenance | Value |
|---|---|
| Target | Director-owned disposable member 36, herdr w2Y:p7 |
| Read command | herdr pane read w2Y:p7 --format ansi --source recent-unwrapped --lines 1080 |
| Placeholder interval | 2026-10-23T08:05:02.682026+00:00 through 2026-10-23T08:05:02.692830+00:00 |
| Draft interval | 2026-10-23T08:05:03.109074+00:00 through 2026-10-23T08:05:03.139927+00:00 |
| Native result | Both exit 0; placeholder 1905 bytes/74 ESC, draft 1791 bytes/63 ESC; empty stderr |
| Interaction | One literal unsent input, herdr pane send-text w2Y:p7 "Ask Codex to do anything"; no Enter; Director owns cleanup |
| Version evidence | PATH installation 0.162.1; effective fixture executable/version confirmation remains separate |
| Theme evidence | Inherited fixture configuration; default theme unconfirmed; collector changed no theme/config |
| Adapter boundary | Native transport before adapter tail windowing |

The warning-footer excerpt comes from BASE/.execution/styled-captures/20261023T075141.825533Z-e6901de1/stdout.ansi and its adjacent metadata.json. Its read-only herdr w2Y:p3 capture ran from 2026-10-23T07:51:41.825533+00:00 through 2026-10-23T07:51:41.836276+00:00 with exit 0, 2594 bytes/95 ESC and empty stderr. It has the same demonstrated dim/default-foreground/RGB-background placeholder role, Context 47% used and a shortcuts/warning footer. The observed PATH installation was 0.162.1; its live pane version and default theme were unconfirmed.

Optional-shortcuts, rotating-hint/context and work/approval/ambiguous-style variants constructed by the tests are derived inputs, rather than additional host observations. The original real excerpts retain their captured footer behavior.

The Codex captures establish the observed Codex/herdr profile. Default-theme support, other backend profiles, tmux transport and built-adapter behavior require their own evidence. The historical BASE/.execution/codex-idle-herdr-20261023T0610.json has no SGR bytes and supplies no placeholder/input-style distinction.

The opencode-herdr fixtures preserve complete actual OpenCode 1.19.1/herdr frames from the controlled unregistered pane w2Y:p8, terminal term_65e7e725eb0f28. They retain every screen row, SGR span, internal/trailing space and the complete composer/Cafleet/model/border/shortcuts/tip/wrapped cwd/branch/version layout. The only normalization is CRLF to LF plus one terminal LF: each original has 14,037 bytes/578 ESC/84 rows; each fixture has 13,955 bytes/578 ESC/84 rows. A direct byte comparison verifies that exact transformation.

| Full-frame fixture | Source SHA-256 | Normalized fixture SHA-256 |
|---|---|---|
| opencode-herdr-placeholder.ansi | f8f76d0d5f9ee3f771385bd6619c455c7b56c8eb83750c4d2693a0b6a6b6c53c | b0db10d647ac283cbd9108f45ec2971f63d53ce2bb32467b060f5d9404308eff |
| opencode-herdr-draft.ansi | 755fc0530edb6f1859f9b9e8369202ff052f7cbb31f6feb07c97b46cc8df2fd2 | 9c168b9f79f70822cbbf8fe31ba0d3f7bc714f43bba5a28e9888419e7eaee2bf |

The source files are placeholder.stdout.ansi and draft.stdout.ansi under BASE/.execution/host-matrix/opencode-herdr-default-20261023T0900Z/paired-captures/20261023T093322.261236Z-opencode-pair-6b0b5b88/. Their pair-metadata.json and independent-verifier-checks.json bind the actual before/after identity and the sole literal unsent input. Verifier's interpretation is the adjacent host-matrix profile's pair-results.md. The full collection ran from 2026-10-23T09:33:22.261236+00:00 through 2026-10-23T09:33:22.728428+00:00. Placeholder capture ran at 09:33:22.273172–09:33:22.283708; draft capture at 09:33:22.704405–09:33:22.716204, on the same UTC date. Both reads used herdr pane read w2Y:p8 --format ansi --source recent-unwrapped --lines 1080, exited 0 and had empty stderr. These are native transport captures before adapter tail windowing.

Both complete visible screens are identical. The actual placeholder text, Ask anything… "Fix broken tests", uses foreground RGB 128/128/128; the identical real unsent user input uses RGB 238/238/238. Both use composer background RGB 30/30/30 without dim. The only raw difference is that exact foreground run immediately before the composer text. The row Cafleet · Big Pickle OpenCode is metadata, and the last nonblank line is the wrapped branch continuation agent-multiplexer. Tests preserve this real trailing layout while requiring Idle for the placeholder and Draft for the input role.

Director exec 26 performed one herdr pane send-text with that exact harmless text and no Enter or Escape. Launch-start.json binds the absolute /Users/himkt/.local/share/mise/installs/aqua-anomalyco-opencode/v1.19.1/opencode executable, version 1.19.1, SHA-256 d4419d0e4ff251cdf50e99aa180c6fbe273ab647080296cf858dc3a339aab560 and --agent cafleet. The private TUI configuration explicitly requested theme=opencode; the observed palette is captured, while independent effective-theme selector and complete managed-layer readback remain unconfirmed. Provider authentication, model work, native execution/continuation and other mux/backend combinations retain separate verification gates. Director owns the live pane and cleanup.

Tests named as derived construct hint/agent/branch substitutions, an adapter-sized 80-row window, ambiguous styles, unstyled/truncated/footer-only inputs, quoted history and work/approval evidence. Those variants are not additional host observations. The actual two captured frames remain unchanged except for the declared line-ending normalization, and the synthetic COMPOSERS pairs remain independent.
