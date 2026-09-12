# apricorn

A behavioral recompilation of Pokémon HeartGold (US) in Rust.

apricorn loads a retail `hg_usa.nds` dump at runtime for all assets and
reimplements the game logic with behavioral equivalence: same RNG outcomes,
same formulas, same save format.

- The plan, with small work items and acceptance gates: [PLAN.md](PLAN.md)
- Planning review and sources: [research](docs/planning/review-2026-09-12.md);
  detailed next steps: [work cards](docs/planning/next-slices.md).
- Status: Phase 5 in progress — the new-game flow lands in a walkable
  bedroom; stairs and doors warp between the house floors and New Bark
  Town with movement, collision and camera ported from the original.
  NPC objects and lighting are present in the working implementation;
  live scripts/dialogue, menu integration and verified parity remain.
  The revised plan starts with reproducible timing/state/save checks (5A)
  and tracks renderer verification in 5B.
- Run: `cargo run -p apricorn-desktop -- --rom hg_usa.nds`
- Controls and remaining parity work: [game flow](docs/game-flow.md)

## Requirements

- Rust 1.92+ (stable)
- Your own dump of Pokémon HeartGold (US). Verify with:
  `Get-FileHash hg_usa.nds -Algorithm SHA1` (PowerShell) or
  `sha1sum hg_usa.nds`
  Expected SHA1: `4fcded0e2713dc03929845de631d0932ea2b5a37`
  (the retail US dump pret/pokeheartgold builds against).
  The ROM is never committed to this repo.

apricorn is a fan research project for personal use. It contains no game assets.
Pokémon and HeartGold are trademarks of Nintendo / Creatures Inc. / GAME FREAK inc.
