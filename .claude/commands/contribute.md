---
description: Guided contribution flow for git-craft (read → branch → TDD → checks → PR)
---

You are contributing to git-craft, a community- and agent-driven voxel game engine.
Follow this loop. Never push to `main` — all changes land via PR.

1. Read `AGENTS.md` (the canonical project guide) and skim the design spec at
   `docs/superpowers/specs/2026-06-11-dabcraft-design.md`. If a milestone plan under
   `docs/superpowers/plans/` covers the work, read it too.
2. Confirm scope with the user: $ARGUMENTS. If it is unclear, ask one focused question.
3. Create a feature branch: `git checkout -b <type>/<short-kebab>` (type = feat/fix/refactor/chore/docs).
4. Implement test-first for engine-core changes (pure functions in `src/mesh`, `src/world`,
   `src/game`): write a failing `cargo test`, then make it pass. For rendering work,
   validate with the F3 HUD / `--bench`, not by feel.
5. From `git-craft/`, run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo test`. Fix anything red.
6. If behavior changed, add a line under `[Unreleased]` in `CHANGELOG.md`.
7. Commit atomically (`type: what and why`), then open a PR with `gh pr create` using the
   template. Describe how you verified the change.

Keep PRs small and focused. CI and maintainer review are the safety net.
