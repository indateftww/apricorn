# apricorn

A behavioral recompilation of Pokémon HeartGold (US) in Rust.

apricorn loads a retail `hg_usa.nds` dump at runtime for all assets and
reimplements the game logic with behavioral equivalence: same RNG outcomes,
same formulas, same save format.

- The plan, phase by phase: [PLAN.md](PLAN.md)
- Status: Phase 0 (scoping & research foundation)

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