# Clarification answers — issue #143 (design 0000108)

User decisions in response to the Drafter's clarifying questions.

## A. Delivery surface
**Release-page asset.** A permanent, public asset attached to the GitHub Release page via `gh release upload` (or an equivalent action such as `softprops/action-gh-release`). NOT `actions/upload-artifact`, NOT an ephemeral workflow-run artifact.

## B. Artifact content & format
- **B1. Contents:** `skills/` only — the three skill directories (`cafleet`, `cafleet-design-doc`, `cafleet-research`). Do NOT include `.claude-plugin/plugin.json` or any other manifest files.
- **B2. Format:** `.zip` only.
- **B3. Naming:** `cafleet-skills-v<version>.zip` — embed the release tag/version in the filename.

## C. CI job structure & permissions
- **C1. Job structure:** Independent / parallel. The new job runs alongside the existing PyPI publish job; neither blocks the other (a PyPI failure must not block the skills upload, and vice versa).
- **C2. Permissions:** Grant the new job its own `permissions: contents: write`. Keep the existing publish job at `contents: read`.

## D. Consumption / docs
**No README/docs download-and-install flow needed.** The user states skill installation will be automatic, so do NOT add a download-and-install docs section. Keep the design scoped to the CI workflow change. (Only update SKILL.md/README if the CLI/architecture surface actually changes — it does not here.)

## E. Edge cases — which releases
Produce the skills asset for **all** `release: published` events, including pre-releases (matching the existing publish trigger).
