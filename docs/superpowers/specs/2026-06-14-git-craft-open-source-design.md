# git-craft — Open-Source Release Design

**Date:** 2026-06-14
**Status:** Approved
**Topic:** Open-source the project on GitHub (rename, licensing, contribution infrastructure, publishing)

## 1. Overview

The project (currently named `git-craft`) is a performance-focused, Minecraft-style
voxel engine written in Rust on wgpu, targeting modern shader-pack visuals (CSM
shadows, GTAO, TAA, volumetrics, bloom, ACES tone mapping, Hillaire atmosphere,
screen-space water reflections) at a 384-block render distance / 120 fps on Apple M4.

### Mission

git-craft is not just an open-source voxel engine — it is a deliberate experiment in
**community-and-agent-driven game development**. The premise:

- Minecraft's direction under Mojang/Microsoft no longer matches what many players and
  builders actually want; the modding/creative community keeps outrunning the official
  game but cannot touch the engine itself.
- AI coding agents have matured, and tools like Claude Code are now in many developers'
  hands. A meaningful contribution no longer requires deep prior knowledge of the codebase —
  an agent, pointed at good docs and guardrails, can carry real work.
- So the goal is to find out **what a community can build on its own, in the open, with
  agents as force-multipliers** — a sandbox game that grows by contribution rather than by
  a single studio's roadmap.

This mission shapes the design: the repository is structured so that a contributor (human
or their AI agent) can arrive cold, understand the project from its docs, and ship a
correct change. Documentation is a permanent, first-class part of the repo — not an
afterthought — and the contribution path is explicitly built for agent-assisted work.

This effort opens the project as a public, contribution-ready repository. It does
four things:

1. **Renames** the project `git-craft` → `git-craft` consistently across code, build
   metadata, directory layout, and documentation.
2. **Licenses** the project under the Rust-ecosystem standard dual `MIT OR Apache-2.0`.
3. **Adds standard contribution infrastructure** so external contributors can build,
   understand, and submit changes (README, license files, CONTRIBUTING, CODE_OF_CONDUCT,
   issue/PR templates, CI).
4. **Publishes** to GitHub as a private repository (the maintainer flips it to public
   after review).

No new engine features. No behavioral code changes beyond the rename. Existing internal
design docs under `docs/superpowers/` are kept as-is and linked from the README.

## 2. Goals & Non-Goals

**Goals**
- A stranger can clone the repo, read the README, and run the game with `cargo run --release`.
- The project's mission and motivation are clearly stated, so contributors understand
  *why* it exists and what it is trying to prove.
- A contributor — human or AI agent — can read the docs, build, test, lint, and submit a PR
  without prior context.
- The contribution workflow is explicitly designed for agent-assisted work; contributors
  who lack the maintainer's personal tooling still get useful, repo-local guidance.
- `main` is protected: changes land only through reviewed PRs with green CI.
- Releases are versioned (SemVer) and tracked in a changelog.
- Licensing and trademark posture are unambiguous and legally clean.
- CI verifies formatting, lints, and tests on every push/PR.
- The project name is consistent everywhere a user or contributor will look.

**Non-Goals**
- No new gameplay/rendering features.
- No refactoring of engine code beyond rename-driven string/identifier changes.
- No per-file license headers (Rust dual-license convention relies on root LICENSE files).
- No rewriting of historical milestone plans/specs beyond the project-name token.
- No `crates.io` publication (binary crate; not a library release).

## 3. Rename: git-craft → git-craft

Full, consistent rename. The token `git-craft` appears ~954 times across the repo; the
bulk are in historical docs and `Cargo.lock`. Scope is split into "must rename" and
"preserve" to avoid breaking path references.

**Rename (identifiers, build, user-facing strings, directory):**
- `git-craft/Cargo.toml` → `name = "git-craft"` (binary target name follows the package
  name automatically → produced binary is `git-craft`).
- Directory `git-craft/` → `git-craft/` via `git mv` (preserves history). Update the
  workspace-relative paths in `CLAUDE.md` and the run/build instructions accordingly.
- `git-craft/src/app.rs`: window title `"git-craft"` → `"git-craft"`.
- `git-craft/src/render/gpu.rs`: the device-capability error string mentioning `git-craft`.
- `git-craft/src/mesh/greedy.rs`: the header comment mentioning `git-craft spec`.
- `.gitignore`: the `# git-craft runtime data` comment.
- `Cargo.lock`: regenerated automatically by the first `cargo check`/build after the
  `Cargo.toml` change — do not hand-edit.
- `CLAUDE.md`: heading `# git-craft` and all prose mentions of the project name.
- `docs/superpowers/specs|plans/*`: replace the bare project-name token in the **bodies**
  for consistency.

**Preserve (to keep links and dated artifacts intact):**
- The **filenames** of historical dated docs (e.g. `2026-06-11-dabcraft-design.md`,
  `…-dabcraft-m1-foundation.md`) are NOT renamed — they are dated artifacts.
- Any **path reference** that points at those files (notably the `Key documents` link
  in `CLAUDE.md`) keeps its `git-craft` filename segment so the link stays valid.

**Verification:** after the rename, `grep -rI "git-craft" --exclude-dir=target
--exclude-dir=.git --exclude-dir=.task-log .` returns only (a) historical doc filenames
and (b) path references to them. `cargo build` succeeds and emits a `git-craft` binary.

## 4. Licensing

Dual-licensed **`MIT OR Apache-2.0`** — the de-facto Rust standard (Bevy, Veloren,
tokio), maximizing adoption while granting an explicit patent license via Apache-2.0.
The consumer chooses either license.

- `LICENSE-MIT` — standard MIT text, copyright line `Copyright (c) 2026 Onur Tellioğlu`.
- `LICENSE-APACHE` — standard Apache License 2.0 text (verbatim, unmodified).
- `Cargo.toml` metadata: `license = "MIT OR Apache-2.0"`, plus `description`,
  `repository`, `readme = "../README.md"` (README lives at repo root, above the crate
  dir), `keywords`, `categories`, `authors`.
- **No per-file SPDX/license headers.**
- **Inbound = outbound:** CONTRIBUTING states contributions are licensed under the same
  dual `MIT OR Apache-2.0` terms (Apache-2.0 §5 default; no CLA).

**Third-party posture (verified):** all dependencies are permissive (wgpu, winit, glam,
bytemuck, egui, rayon, fastnoise-lite — MIT/Apache). No third-party source was copied
into the tree, so no `NOTICE` file is required. Rendering techniques implemented from
published papers (ACES tone mapping, Hillaire atmosphere, GTAO) are credited in a README
"References" section as courtesy and provenance, not as a license obligation.

## 5. Contribution Infrastructure (Standard scope)

All new files at the repo root unless noted.

### 5.1 `README.md`
First impression and entry point. Sections:
- **Title + one-line pitch** + trademark disclaimer line (see §6).
- **Screenshots/GIF** — placeholder block with a `<!-- TODO -->` note (maintainer adds
  media before going public; flagged, not silently omitted).
- **Features** — CSM shadows, GTAO, TAA, volumetric in-scatter, bloom, ACES tone mapping,
  Hillaire sky/atmosphere LUTs, screen-space water reflections; 384-block render distance
  / 120 fps Apple M4 target.
- **Build & Run** — Rust toolchain requirement (edition 2024 → Rust ≥ 1.85), then
  `cargo run --release`. A note that debug builds are intentionally slow (documented in
  `Cargo.toml` profile overrides).
- **Controls** — a table extracted precisely from `src/app.rs` keybindings at
  implementation time. Known bindings to verify and document: WASD + Space movement,
  mouse-look (Escape to capture/release), left/right mouse = break/place, digits 1–9 =
  hotbar, F3/H = debug HUD, R = render-scale cycle (1.0/0.75/0.5), B = volumetric debug
  view, plus F/V/G toggles (confirm each against the code).
- **Architecture / tech stack** — wgpu 29, winit, glam, egui; forward TBDR-friendly
  pipeline; pure-function engine core.
- **Project status** — early/active; milestone summary (M1–M5 done, M6 = persistence/
  textures/perf next).
- **Documentation** — links to `docs/superpowers/specs/2026-06-11-dabcraft-design.md`
  (design spec) and the `docs/superpowers/plans/` milestone plans.
- **Contributing** — one-line pointer to `CONTRIBUTING.md`.
- **References** — ACES, Hillaire atmosphere, GTAO citations.
- **License** — dual MIT/Apache statement + pointer to the two files.

### 5.2 `CONTRIBUTING.md`
- **Prerequisites** — Rust ≥ 1.85 (edition 2024); macOS/Apple-Silicon is the primary
  tested target.
- **Build/run/check** — `cargo run --release`, `cargo test`, `cargo clippy --all-targets
  -- -D warnings`, `cargo fmt`.
- **Code conventions** (from `CLAUDE.md`): engine core is pure functions over plain data,
  unit-tested (TDD); rendering claims validated via the F3 HUD GPU timestamps and
  `--bench` percentiles, never "by feel"; Apple TBDR discipline (forward rendering, no
  Z-prepass, alpha-tested geometry in its own pipeline after opaque).
- **Git workflow** — feature branches, atomic commits, commit format `type: what and why`
  (`feat`/`fix`/`refactor`/`chore`), branch names `<type>/<short-kebab>`.
- **PR process** — fork/branch → ensure fmt+clippy+test pass → open PR with the template.
- **Asset/IP rule** — contributions must not include Minecraft (or other proprietary)
  assets; textures/sounds must be original or under a compatible permissive/CC license.
- **Licensing of contributions** — inbound = outbound (`MIT OR Apache-2.0`), no CLA.

### 5.3 `CODE_OF_CONDUCT.md`
Contributor Covenant 2.1, verbatim. **Reporting/enforcement contact = the project's
GitHub Issues tracker** (no personal email published). The contact line reads roughly:
"Report issues by opening a GitHub issue or contacting the maintainers via the repository."

### 5.4 `.github/ISSUE_TEMPLATE/`
- `bug_report.yml` — fields: description, steps to reproduce, expected vs. actual,
  environment (OS, GPU/Apple-Silicon model, Rust version, render-scale setting), F3 HUD
  readout if relevant, screenshots.
- `feature_request.yml` — problem, proposed solution, alternatives, scope.
- `config.yml` — `blank_issues_enabled: false` (optional), no external contact links.

### 5.5 `.github/PULL_REQUEST_TEMPLATE.md`
Summary; related issue; type of change; checklist (`cargo fmt --check`, `cargo clippy`,
`cargo test` pass; follows conventions; docs/design updated if behavior changed).

### 5.6 `.github/workflows/ci.yml`
- **Runner:** `macos-latest` — the project targets Apple-Silicon/Metal; this avoids
  Linux winit/wgpu system-dependency setup and is free for public repos. CI compiles on
  the actual target platform.
- **Trigger:** `push` and `pull_request`.
- **Toolchain:** `dtolnay/rust-toolchain@stable`; cache via `Swatinem/rust-cache`.
- **Steps:** `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` →
  `cargo test`. Working directory = the crate dir (`git-craft/`).
- **Note:** tests are pure-function unit tests (no GPU required); they run headless on the
  runner. GPU-dependent rendering is validated manually via the HUD, not in CI.

## 6. Name & Trademark

The name `git-craft` is kept. To avoid implying affiliation with Mojang/Microsoft, the
README carries a disclaimer:

> git-craft is an independent, fan-made project. It is not affiliated with, endorsed by,
> or associated with Mojang Studios or Microsoft. "Minecraft" is a trademark of Mojang
> Synergies AB.

Currently the repo ships **no** Minecraft assets (no textures yet). The CONTRIBUTING
asset/IP rule (§5.2) prevents future contributions from introducing proprietary assets —
relevant for the upcoming M6 texture work, which must use original or compatibly-licensed
art.

## 7. Publishing Flow

Executed during implementation (this is the maintainer-facing sequence; outward-facing
steps require the maintainer's go-ahead at execution time):

1. **Pre-flight:** confirm `git status` is clean of unrelated changes; confirm `gh auth
   status` is authenticated.
2. **Merge latest code to `main`:** `feat/m5-shaders` → `main` (expected fast-forward, as
   `main` is strictly behind). Public `main` then reflects the latest engine state.
3. **Branch:** create `chore/open-source-setup` off `main`.
4. **Apply changes in atomic commits** on the branch:
   - `chore: rename git-craft to git-craft` (dir, Cargo, code strings, docs token).
   - `chore: add MIT/Apache dual license` (LICENSE-MIT, LICENSE-APACHE, Cargo metadata).
   - `docs: add README` .
   - `docs: add CONTRIBUTING and CODE_OF_CONDUCT`.
   - `chore: add GitHub issue/PR templates and CI workflow`.
   - Run `cargo fmt`, `cargo clippy`, `cargo test`, and a `cargo build --release` to
     confirm the rename builds and the binary is `git-craft` before finalizing.
5. **Create the GitHub repo (private):** `gh repo create git-craft --private --source=.
   --remote=origin --push`. Push `main` and the `chore/open-source-setup` branch.
6. **Open a PR** (`chore/open-source-setup` → `main`) so `main` history stays reviewable;
   the maintainer self-merges.
7. **Hand-off:** report the repo URL. The maintainer reviews and flips the repo to
   **public** when satisfied.

## 8. Success Criteria

- `grep -rI git-craft` (excluding `target/.git/.task-log`) returns only historical doc
  filenames and path references to them.
- `cargo build --release` produces a binary named `git-craft`; `cargo test`, `cargo
  clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass.
- Repo root contains `README.md`, `VISION.md`, `CHANGELOG.md`, `AGENTS.md`, `CLAUDE.md`,
  `LICENSE-MIT`, `LICENSE-APACHE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  and `.github/` with issue templates, a PR template, `CODEOWNERS`, `ci.yml`, and
  `release.yml`; plus `.claude/commands/contribute.md` and `.claude/settings.json`.
- CI runs green on the first push/PR.
- README states the mission ("Why git-craft?") and links to `VISION.md`; the repo
  description/topics carry the positioning.
- `CHANGELOG.md` exists with `[Unreleased]` + a `[0.1.0]` baseline; the release workflow is
  in place.
- `main` is protected (PR required, CI must pass, review required) once public; `CODEOWNERS`
  routes review to the maintainer.
- `AGENTS.md` lets a cold AI agent build, locate code, follow conventions, and open a
  correct PR without the maintainer's personal setup; `/contribute` guides Claude Code users.
- README lets a newcomer build, run, and understand the project; CONTRIBUTING lets a
  contributor submit a correct PR; license and trademark posture are unambiguous.

## 9. Risks & Mitigations

- **Rename breaks a path reference / link.** Mitigation: the explicit preserve-list in §3
  (historical filenames and references to them) plus the grep verification.
- **`Cargo.lock` churn.** Mitigation: regenerate via `cargo check`, never hand-edit; the
  only expected change is the package-name entry.
- **CI flakiness on macOS runners (cold builds slow).** Mitigation: `rust-cache`; the
  test set is small and pure-function.
- **Publishing prematurely public.** Mitigation: create as **private**; the maintainer
  performs the public flip after review (per the chosen publishing option).
- **Personal info exposure.** Mitigation: CoC uses GitHub Issues, not a personal email;
  the only personal data published is the copyright name `Onur Tellioğlu` (intended).
- **Branch protection not enforceable while private on a free plan.** Mitigation: classic
  protected branches / rulesets are free for public repos; configure the ruleset when (or
  right after) flipping to public. The bootstrap PR may merge before protection is live —
  acceptable for the very first commit.
- **Vendored `.claude/` tooling drifting from the maintainer's setup.** Mitigation: keep
  the repo-local command small and self-contained (one guided contribute flow), documented
  as tool-optional; `AGENTS.md` is the canonical, tool-agnostic source of truth.

## 10. Vision Artifacts (mission baked in)

The mission (see §1) is expressed in three places so it is unavoidable:

- **`VISION.md`** (repo root) — the manifesto. Long-form statement of: the post-Mojang gap,
  the agent-era thesis ("anyone with an agent can contribute"), the experiment ("what can a
  community build on its own, in the open"), the principles (open by default, docs are
  permanent and first-class, small reviewed PRs, validate with data), and an explicit
  invitation to contribute. Written to inspire without overclaiming — honest about the
  project being early.
- **README "Why git-craft?" section** — a tight 3–5 sentence condensation at the top of the
  README (right after the title/disclaimer), linking to `VISION.md`.
- **GitHub repo description + topics** — the one-line description carries the positioning;
  topics include `voxel`, `game-engine`, `rust`, `wgpu`, `sandbox`, `open-source`,
  `ai-agents`, `community-driven`.

## 11. Versioning & Releases

- **SemVer.** `Cargo.toml` `version` is the single source of truth (starts at `0.1.0`; the
  `0.x` line signals "early, expect breaking changes").
- **`CHANGELOG.md`** (repo root) — [Keep a Changelog](https://keepachangelog.com) format:
  an `[Unreleased]` section at the top that contributors append to, plus a baseline
  `[0.1.0]` entry summarizing the current M1–M5 state. PRs that change behavior add a line
  under `[Unreleased]`.
- **Git tags** — releases are tagged `vX.Y.Z`. Cutting a release = move `[Unreleased]`
  entries under a new `[X.Y.Z] - YYYY-MM-DD` heading, bump `Cargo.toml`, tag, push the tag.
  This process is documented in CONTRIBUTING (a short "Releases" subsection).
- **`.github/workflows/release.yml`** — on a pushed tag matching `v*`, create a GitHub
  Release whose body is the matching `CHANGELOG.md` section. Low-maintenance automation that
  keeps releases consistent.

## 12. Repository Governance

The repo is expected to receive PRs authored by *other people's agents*, so `main` must be
defended and review must be routed to the maintainer.

- **Branch protection / ruleset on `main`** (applied at/after the public flip): require a PR
  before merging; require the `CI / fmt + clippy + test` status check to pass; require the
  branch to be up to date; block force-pushes and deletion; require at least **one approving
  review** (so an agent cannot self-merge to `main`). Configured via `gh api` (ruleset or
  classic protection endpoint).
- **`.github/CODEOWNERS`** — the maintainer (`@<gh-login>`) owns the whole tree, so every PR
  auto-requests their review. This pairs with the "require review" rule above.
- **`SECURITY.md`** — short policy: how to report a vulnerability (privately, via GitHub
  security advisories / a private report), and that the project is early and best-effort.

## 13. Agent Enablement (chosen: docs + lightweight `.claude/`)

Contributors will often work through an AI agent that does **not** have the maintainer's
global Claude setup (skills, commands, global `CLAUDE.md`). The repo therefore ships its own
guidance and a small, tool-optional helper:

- **`AGENTS.md`** (repo root) — the **canonical, tool-agnostic** agent guide, following the
  emerging `AGENTS.md` convention many agentic tools read. Contents: project one-liner +
  link to VISION; how to build/run/test/lint (exact commands); architecture map (where the
  engine core, render passes, shaders, worldgen live); the hard conventions (pure-function
  TDD core, validate rendering via the F3 HUD/`--bench` not by feel, Apple TBDR discipline);
  guardrails (don't commit proprietary assets, don't push to `main`, keep PRs small); and
  "read the design spec first" pointers.
- **`CLAUDE.md`** (repo root) — kept (Claude Code auto-loads it), but trimmed to a short
  pointer: "This project's agent guidance lives in `AGENTS.md` — read it first," plus any
  Claude-Code-specific notes. `AGENTS.md` stays canonical to avoid drift.
- **CONTRIBUTING "Building with an AI agent" section** — human-facing: this project is
  designed to be built with agents; point your agent at `AGENTS.md` + the design spec +
  milestone plans, work in a small branch, let CI gate you, open a PR.
- **`.claude/commands/contribute.md`** — a project-scoped slash command (`/contribute`) that
  Claude Code contributors get automatically when they open the repo. It encodes the loop:
  read `AGENTS.md` and the relevant spec/plan → pick or confirm scope → create a feature
  branch → implement test-first → run `cargo fmt`/`clippy`/`test` → open a PR with the
  template. Self-contained; does not depend on the maintainer's personal skills.
- **`.claude/settings.json`** — minimal project settings: an allowlist for the safe, common
  commands a contributing agent runs (`cargo build`/`test`/`clippy`/`fmt`, `git status`/
  `diff`/`add`/`commit`, `gh pr`...), to reduce permission friction without granting
  anything dangerous. No secrets, no destructive permissions.
