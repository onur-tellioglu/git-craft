# git-craft — Claude Code notes

This project's canonical guidance lives in **[AGENTS.md](AGENTS.md)** — read it first. It
covers the architecture map, conventions, and guardrails.

## Quick reference (run from `git-craft/`)

```bash
cargo run --release                          # play
cargo test                                   # engine-core unit tests
cargo clippy --all-targets -- -D warnings    # lints
cargo fmt                                     # format
```

## Guardrails

- Never push to `main` — land changes via a PR from a feature branch.
- No proprietary assets. Validate rendering/perf with the F3 HUD and `--bench`, not by feel.
- Read the design spec at `docs/superpowers/specs/2026-06-11-dabcraft-design.md` first.

## Claude Code specifics

- Run `/contribute` (project command in `.claude/commands/`) for a guided contribution loop.
- Safe, common commands are pre-allowed in `.claude/settings.json`.
