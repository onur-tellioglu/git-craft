# Vision

## Why this exists

Minecraft proved that a voxel sandbox can be a canvas for millions of imaginations. But the
game belongs to a single studio, and its direction — shaped by Mojang and Microsoft — no
longer matches what many of the people who love it actually want. The community has always
dreamed bigger than the official roadmap: mods, shaders, data packs, total conversions. Yet
the one thing the community could never touch was the **engine** itself.

git-craft takes the other road. It is a voxel sandbox that is **open all the way down** —
the renderer, the world generation, the physics, the lighting — and it grows by
contribution, not by a corporate roadmap.

## The agent-era thesis

Something changed recently: AI coding agents got good. Tools like Claude Code are now in the
hands of ordinary developers, and a well-scoped change to an unfamiliar codebase no longer
requires weeks of ramp-up. If the project provides clear documentation and firm guardrails,
a contributor — guided by their agent — can land real, correct work on day one.

That reframes what a community project can be. git-craft is an experiment in exactly this:

> **What can a community build on its own, in the open, with AI agents as
> force-multipliers?**

We want to find out — and to show it.

## Principles

- **Open by default.** The engine, the docs, the design history — all public, all permanent.
- **Documentation is first-class.** Specs and plans live in the repo forever. If it isn't
  written down, an agent can't build from it, and neither can a newcomer.
- **Built for agents and humans alike.** `AGENTS.md` and a guided `/contribute` flow mean a
  contributor's tooling doesn't have to match the maintainer's.
- **Small, reviewed steps.** Changes land through PRs with green CI and human review on a
  protected `main`. Trust the process, not heroics.
- **Validate with data, not vibes.** Performance and rendering claims are proven with the
  F3 HUD timestamps and benchmarks — never asserted by feel.
- **Performance is a feature.** The target is shader-pack-grade visuals at a 384-block
  render distance and 120 fps on Apple M4.

## Where we are

Early, and honest about it. The engine already does cascaded shadows, GTAO, TAA,
volumetrics, bloom, ACES tone mapping, a Hillaire atmosphere, and screen-space water
reflections. There is no persistence or texturing yet. The roadmap is in the open under
`docs/superpowers/`.

## Come build with us

If any of this resonates, you are exactly who this project is for. Read
[CONTRIBUTING.md](CONTRIBUTING.md), point your agent at [AGENTS.md](AGENTS.md), and open a
PR. Let's show what a community can do.
