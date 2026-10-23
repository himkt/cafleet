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

These captures establish the observed Codex/herdr profile. Default-theme support, other backend profiles, tmux transport and built-adapter behavior require their own evidence. The historical BASE/.execution/codex-idle-herdr-20261023T0610.json has no SGR bytes and supplies no placeholder/input-style distinction.
