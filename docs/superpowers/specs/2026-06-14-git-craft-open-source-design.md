# git-craft — Open-Source Release Design

**Date:** 2026-06-14
**Status:** Approved
**Topic:** Open-source the project on GitHub (rename, licensing, contribution infrastructure, publishing)

## 1. Overview

The project (currently named `dabcraft`) is a performance-focused, Minecraft-style
voxel engine written in Rust on wgpu, targeting modern shader-pack visuals (CSM
shadows, GTAO, TAA, volumetrics, bloom, ACES tone mapping, Hillaire atmosphere,
screen-space water reflections) at a 384-block render distance / 120 fps on Apple M4.

This effort opens the project as a public, contribution-ready repository. It does
four things:

1. **Renames** the project `dabcraft` → `git-craft` consistently across code, build
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
- A contributor can read CONTRIBUTING and know how to build, test, lint, and submit a PR.
- Licensing and trademark posture are unambiguous and legally clean.
- CI verifies formatting, lints, and tests on every push/PR.
- The project name is consistent everywhere a user or contributor will look.

**Non-Goals**
- No new gameplay/rendering features.
- No refactoring of engine code beyond rename-driven string/identifier changes.
- No per-file license headers (Rust dual-license convention relies on root LICENSE files).
- No rewriting of historical milestone plans/specs beyond the project-name token.
- No `crates.io` publication (binary crate; not a library release).

## 3. Rename: dabcraft → git-craft

Full, consistent rename. The token `dabcraft` appears ~954 times across the repo; the
bulk are in historical docs and `Cargo.lock`. Scope is split into "must rename" and
"preserve" to avoid breaking path references.

**Rename (identifiers, build, user-facing strings, directory):**
- `dabcraft/Cargo.toml` → `name = "git-craft"` (binary target name follows the package
  name automatically → produced binary is `git-craft`).
- Directory `dabcraft/` → `git-craft/` via `git mv` (preserves history). Update the
  workspace-relative paths in `CLAUDE.md` and the run/build instructions accordingly.
- `dabcraft/src/app.rs`: window title `"dabcraft"` → `"git-craft"`.
- `dabcraft/src/render/gpu.rs`: the device-capability error string mentioning `dabcraft`.
- `dabcraft/src/mesh/greedy.rs`: the header comment mentioning `dabcraft spec`.
- `.gitignore`: the `# dabcraft runtime data` comment.
- `Cargo.lock`: regenerated automatically by the first `cargo check`/build after the
  `Cargo.toml` change — do not hand-edit.
- `CLAUDE.md`: heading `# dabcraft` and all prose mentions of the project name.
- `docs/superpowers/specs|plans/*`: replace the bare project-name token in the **bodies**
  for consistency.

**Preserve (to keep links and dated artifacts intact):**
- The **filenames** of historical dated docs (e.g. `2026-06-11-dabcraft-design.md`,
  `…-dabcraft-m1-foundation.md`) are NOT renamed — they are dated artifacts.
- Any **path reference** that points at those files (notably the `Key documents` link
  in `CLAUDE.md`) keeps its `dabcraft` filename segment so the link stays valid.

**Verification:** after the rename, `grep -rI "dabcraft" --exclude-dir=target
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
   - `chore: rename dabcraft to git-craft` (dir, Cargo, code strings, docs token).
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

- `grep -rI dabcraft` (excluding `target/.git/.task-log`) returns only historical doc
  filenames and path references to them.
- `cargo build --release` produces a binary named `git-craft`; `cargo test`, `cargo
  clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass.
- Repo root contains `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `CONTRIBUTING.md`,
  `CODE_OF_CONDUCT.md`, and `.github/` with issue templates, a PR template, and `ci.yml`.
- CI runs green on the first push/PR.
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
