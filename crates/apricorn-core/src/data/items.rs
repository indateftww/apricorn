//! Item data: the 34-byte rows of `item_data` (`a/0/1/7`), built by
//! pret's `csv2bin --pad 0xFF` from `files/itemtool/itemdata/
//! item_data.txt`.

use crate::nds::{NdsError, u16le};

use super::exact;

/// Field pocket ids — the raw `fieldPocket` nibble values of the item
/// bitfield word (names from pret's bag/save code; ids pinned against
/// the retail rows by tests/data_hg.rs).
pub mod pocket {
    /// Items that live in the generic "Items" pocket.
    pub const ITEMS: u8 = 0;
    /// Healing items pocket.
    pub const MEDICINE: u8 = 1;
    /// Poké Ball pocket.
    pub const BALLS: u8 = 2;
    /// TM/HM pocket.
    pub const TMHMS: u8 = 3;
    /// Berry pocket.
    pub const BERRIES: u8 = 4;
    /// Mail pocket.
    pub const MAIL: u8 = 5;
    /// Battle items pocket.
    pub const BATTLE_ITEMS: u8 = 6;
    /// Key items pocket.
    pub const KEY_ITEMS: u8 = 7;
}

/// The `naturalGiftType` value marking "this item grants no Natural
/// Gift" — a sentinel written into the data itself (31 is past the last
/// real type id, 17).
pub const NO_NATURAL_GIFT: u8 = 31;

/// One item row — the 34 bytes `csv2bin` emits per item (the 36-byte C
/// `ItemData` struct in `include/item.h` adds two bytes of tail padding
/// the archive does not carry).
///
/// The 20 `partyParam` bytes (offsets 0x0E–0x21, pret's `ItemPartyParam`
/// union — per-status healing power, EV/dispensary/friendship
/// modifiers) stay raw here; their meaning is per-field-use-function
/// and gets decoded with the bag/party code that consumes them
/// (Phase 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemEntry {
    /// Buy price (0 for unsellable/unbuyable).
    pub price: u16,
    /// Held-item effect id.
    pub hold_effect: u8,
    /// Parameter for the held effect (e.g. HP restored per turn).
    pub hold_effect_param: u8,
    /// Pluck effect id.
    pub pluck_effect: u8,
    /// Fling effect id.
    pub fling_effect: u8,
    /// Fling base power.
    pub fling_power: u8,
    /// Natural Gift base power.
    pub natural_gift_power: u8,
    /// Natural Gift type — a pret `TYPE_*` id or
    /// [`NO_NATURAL_GIFT`].
    pub natural_gift_type: u8,
    /// Whether the item cannot be tossed.
    pub prevent_toss: bool,
    /// Whether the item can be selected.
    pub selectable: bool,
    /// The bag pocket the item lives in — a [`pocket`] id.
    pub field_pocket: u8,
    /// The battle bag pocket the item lives in.
    pub battle_pocket: u8,
    /// Field use-function id (0 = unusable in the field).
    pub field_use_func: u8,
    /// Battle use-function id (0 = unusable in battle).
    pub battle_use_func: u8,
    /// Party-menu use function id.
    pub party_use: u8,
    /// The raw 20-byte `ItemPartyParam` union, meaning per use-function.
    pub party_param: [u8; 20],
}

impl ItemEntry {
    /// Bytes in one row.
    pub const SIZE: usize = 34;

    /// Parses one `item_data` member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is not exactly
    /// [`SIZE`](Self::SIZE) bytes or its 0x0D pad byte is not zero
    /// (the retail generator writes zero; `csv2bin`'s 0xFF padding
    /// lives *between* members, not inside them).
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        exact(data, Self::SIZE, "item entry")?;
        if data[0x0D] != 0 {
            return Err(NdsError::Invalid {
                what: "item entry's padding byte at 0x0D is not zero",
            });
        }
        let bits = u16le(data, 0x08)?;
        let mut party_param = [0u8; 20];
        party_param.copy_from_slice(&data[0x0E..0x22]);
        Ok(Self {
            price: u16le(data, 0)?,
            hold_effect: data[0x02],
            hold_effect_param: data[0x03],
            pluck_effect: data[0x04],
            fling_effect: data[0x05],
            fling_power: data[0x06],
            natural_gift_power: data[0x07],
            natural_gift_type: (bits & 0x1F) as u8,
            prevent_toss: bits >> 5 & 1 != 0,
            selectable: bits >> 6 & 1 != 0,
            field_pocket: (bits >> 7 & 0xF) as u8,
            battle_pocket: (bits >> 11 & 0x1F) as u8,
            field_use_func: data[0x0A],
            battle_use_func: data[0x0B],
            party_use: data[0x0C],
            party_param,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Potion-shaped fixture (item 17 in the retail table).
    fn potion_bytes() -> Vec<u8> {
        let mut b = vec![0u8; ItemEntry::SIZE];
        b[0..2].copy_from_slice(&300u16.to_le_bytes()); // price
        b[3] = 20; // holdEffectParam (Restore Held)
        b[6] = 30; // fling power
        b[0x07] = 0; // Natural Gift power
        let bits = 1u16 | (1 << 6) | (pocket::MEDICINE as u16) << 7 | (4u16) << 11; // selectable, field pocket 1, battle pocket 4
        b[0x08..0x0A].copy_from_slice(&bits.to_le_bytes());
        b[0x0A] = 1; // field use: heal HP
        b[0x0B] = 2; // battle use: heal HP
        b[0x0C] = 1; // party use: heal HP
        b[0x0E] = 20; // partyParam byte 0: a distinct probe value
        b
    }

    #[test]
    fn parses_every_item_field() {
        let item = ItemEntry::parse(&potion_bytes()).expect("fixture parses");
        assert_eq!(item.price, 300);
        assert_eq!(item.hold_effect, 0);
        assert_eq!(item.hold_effect_param, 20);
        assert_eq!(item.fling_power, 30);
        assert_eq!(item.natural_gift_power, 0);
        assert_eq!(item.natural_gift_type, 1); // Fighting, as encoded
        assert!(!item.prevent_toss);
        assert!(item.selectable);
        assert_eq!(item.field_pocket, pocket::MEDICINE);
        assert_eq!(item.battle_pocket, 4);
        assert_eq!(
            (item.field_use_func, item.battle_use_func, item.party_use),
            (1, 2, 1)
        );
        assert_eq!(item.party_param[0], 20);
        assert_eq!(item.party_param[1..], [0u8; 19]);
    }

    #[test]
    fn bitfield_decode_covers_both_pocket_nibbles() {
        // Master Ball's real bitfield: field pocket 2 (Balls),
        // battle pocket 1, unselectable, no Natural Gift (type 31
        // saturates the 5-bit field).
        let mut b = potion_bytes();
        let bits = (31u16 & 0x1F) | (pocket::BALLS as u16) << 7 | (1u16) << 11;
        b[0x08..0x0A].copy_from_slice(&bits.to_le_bytes());
        b[0x0A] = 0; // unusable in the field
        let item = ItemEntry::parse(&b).expect("fixture parses");
        assert_eq!(item.natural_gift_type, NO_NATURAL_GIFT);
        assert!(!item.selectable);
        assert_eq!(item.field_pocket, pocket::BALLS);
        assert_eq!(item.battle_pocket, 1);
    }

    #[test]
    fn rejects_wrong_sizes_and_pad() {
        let good = potion_bytes();
        assert!(ItemEntry::parse(&good[..33]).is_err(), "truncated");
        let mut long = good.clone();
        long.push(0);
        assert!(ItemEntry::parse(&long).is_err(), "trailing slack");

        let mut drift = good.clone();
        drift[0x0D] = 1;
        assert!(ItemEntry::parse(&drift).is_err(), "pad byte drifted");
    }
}
