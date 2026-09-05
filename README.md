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
  and compare against the hash in [pret/pokeheartgold's README](https://github.com/pret/pokeheartgold).
  The ROM is never committed to this repo.

apricorn is a fan research project for personal use. It contains no game assets.
Pokémon and HeartGold are trademarks of Nintendo / Creatures Inc. / GAME FREAK inc.