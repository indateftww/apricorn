//! Move data: the 16-byte rows of the `waza` table (`a/0/1/1`).

use crate::nds::{NdsError, u16le};

use super::exact;

/// A move's damage class — the `category` byte of each `waza` row
/// (verified against the retail table: physical moves carry 0, special
/// moves 1, everything else 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveCategory {
    /// Physical: contact damage from the attacker's Attack.
    Physical,
    /// Special: damage from the attacker's Special Attack.
    Special,
    /// Status: no damage — effect only.
    Status,
}

/// One move row — pret's 16-byte `MoveTbl` (`include/move.h`), exactly
/// as the retail `waza` NARC stores it (the C struct adds no padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntry {
    /// The `MoveAttr` effect id the battle engine runs (Phase 6).
    pub effect: u16,
    /// The damage class.
    pub category: MoveCategory,
    /// Base power (0 for status moves and fixed-damage effects).
    pub power: u8,
    /// Type, a pret `TYPE_*` id (0 = Normal … 17 = Dark).
    pub type_: u8,
    /// Accuracy percent (0 means the effect decides, e.g. Struggle).
    pub accuracy: u8,
    /// Base power points.
    pub pp: u8,
    /// The effect's secondary-procedure chance percent (0 = never).
    pub effect_chance: u8,
    /// The move's target `RANGE_*` selector (read by battle targeting).
    pub range: u16,
    /// Turn priority bracket (-7…+7).
    pub priority: i8,
    /// pret `MoveTbl` byte 0x0B — unconsumed by decompiled sources.
    pub unk_0b: u8,
    /// pret `MoveTbl` byte 0x0C — unconsumed by decompiled sources.
    pub unk_0c: u8,
    /// Contest appeal type.
    pub contest_type: u8,
    /// pret `MoveTbl` byte 0x0E — unconsumed by decompiled sources.
    pub unk_0e: u16,
}

impl MoveEntry {
    /// Bytes in one row.
    pub const SIZE: usize = 16;

    /// Parses one `waza` member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is not exactly
    /// [`SIZE`](Self::SIZE) bytes or the category byte is not 0–2.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        exact(data, Self::SIZE, "move entry")?;
        let category = match data[2] {
            0 => MoveCategory::Physical,
            1 => MoveCategory::Special,
            2 => MoveCategory::Status,
            _ => {
                return Err(NdsError::Invalid {
                    what: "move entry's damage category is not 0-2",
                });
            }
        };
        Ok(Self {
            effect: u16le(data, 0)?,
            category,
            power: data[3],
            type_: data[4],
            accuracy: data[5],
            pp: data[6],
            effect_chance: data[7],
            range: u16le(data, 8)?,
            priority: data[0x0A] as i8,
            unk_0b: data[0x0B],
            unk_0c: data[0x0C],
            contest_type: data[0x0D],
            unk_0e: u16le(data, 0x0E)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Quick Attack-shaped fixture: effect 103, physical, power 40,
    /// Normal, 100% accurate, 30 PP, priority 1.
    fn quick_attack_bytes() -> Vec<u8> {
        let mut b = vec![0u8; MoveEntry::SIZE];
        b[0..2].copy_from_slice(&103u16.to_le_bytes()); // effect
        b[2] = 0; // physical
        b[3] = 40; // power
        b[4] = 0; // Normal
        b[5] = 100; // accuracy
        b[6] = 30; // pp
        b[7] = 0; // effect chance
        b[8..0x0A].copy_from_slice(&0x0Au16.to_le_bytes()); // selected user
        b[0x0A] = 1; // priority +1
        b[0x0B] = 115;
        b[0x0C] = 5;
        b[0x0D] = 4; // cool contest
        b[0x0E..0x10].copy_from_slice(&0u16.to_le_bytes());
        b
    }

    #[test]
    fn parses_every_move_field() {
        let entry = MoveEntry::parse(&quick_attack_bytes()).expect("fixture parses");
        assert_eq!(entry.effect, 103);
        assert_eq!(entry.category, MoveCategory::Physical);
        assert_eq!(entry.power, 40);
        assert_eq!(entry.type_, 0);
        assert_eq!(entry.accuracy, 100);
        assert_eq!(entry.pp, 30);
        assert_eq!(entry.effect_chance, 0);
        assert_eq!(entry.range, 0x0A);
        assert_eq!(entry.priority, 1);
        assert_eq!(entry.unk_0b, 115);
        assert_eq!(entry.unk_0c, 5);
        assert_eq!(entry.contest_type, 4);
        assert_eq!(entry.unk_0e, 0);
    }

    #[test]
    fn categories_come_from_the_category_byte() {
        let mut b = quick_attack_bytes();
        b[2] = 1;
        assert_eq!(
            MoveEntry::parse(&b).expect("fixture parses").category,
            MoveCategory::Special
        );
        b[2] = 2;
        assert_eq!(
            MoveEntry::parse(&b).expect("fixture parses").category,
            MoveCategory::Status
        );
        b[2] = 3;
        assert!(MoveEntry::parse(&b).is_err(), "category 3 is not a class");
    }

    #[test]
    fn negative_priority_parses_as_signed() {
        // Struggle's tail — and every -priority move (e.g. Avalanche at
        // -1) — exercises the i8 cast.
        let mut b = quick_attack_bytes();
        b[0x0A] = 0xFF; // -1
        assert_eq!(MoveEntry::parse(&b).expect("fixture parses").priority, -1);
    }

    #[test]
    fn rejects_wrong_sizes() {
        let good = quick_attack_bytes();
        assert!(MoveEntry::parse(&good[..15]).is_err(), "truncated");
        let mut long = good.clone();
        long.push(0);
        assert!(MoveEntry::parse(&long).is_err(), "trailing slack");
    }
}
