//! Story flags and script variables — the `SaveVarsFlags` block, save
//! block 4 (`SAVE_FLAGS`); pret `src/save_vars_flags.c` and
//! `include/save_vars_flags.h:9-12`:
//!
//! ```text
//! struct SaveVarsFlags {
//!     u16 vars[NUM_VARS];        // 0x000 .. 0x2E0   NUM_VARS  = 0x170  (vars.h:384)
//!     u8  flags[NUM_FLAGS / 8];  // 0x2E0 .. 0x44C   NUM_FLAGS = 2912   (flags.h:2223)
//! };                             // sizeof = 0x44C = BLOCK_RAW_SIZES[FLAGS]
//! ```
//!
//! Script-visible ids are *not* array indices: a variable id is
//! [`VAR_BASE`]-relative (`Save_VarsFlags_GetVarAddr`: `vars[varId -
//! VAR_BASE]`, asserted in range), and a flag id addresses bit `id % 8`
//! of `flags[id / 8]` (`Save_VarsFlags_GetFlagAddr`), with flag 0
//! reserved (it reads as unset and cannot be set) and ids at or above
//! [`TEMP_FLAG_BASE`] living outside the save in the static
//! `sTempFlags` array — [`TempFlags`] here. Ids at or above
//! [`SPECIAL_VAR_BASE`] are the script environment's `specialVars`,
//! not this block (`GetVarPointer`, `src/script_manager.c:354`).
//!
//! The new-game defaults (`save::new_game`) touch exactly two entries:
//! var `0x4035` ([`VAR_MAGIKARP_SIZE_RECORD`]) = 56150 at byte `0x6A`,
//! and flag `0x960` ([`FLAG_UNK_960`]) at byte `0x2E0 + 0x12C`, bit 0 —
//! which is how the layout above is pinned against the ROM
//! (`tests/vars_flags.rs`).

use std::fmt;

/// `VAR_BASE` — the first script variable id (`vars.h:4`).
pub const VAR_BASE: u16 = 0x4000;
/// `NUM_VARS` — how many saved variables there are (`vars.h:384`).
pub const NUM_VARS: usize = 0x170;
/// The last saved variable id (`VARS_END`, `vars.h:405`).
pub const VARS_END: u16 = VAR_BASE + NUM_VARS as u16 - 1;
/// `TEMP_VAR_BASE` — the temporary variables sit at the start of the
/// array (`vars.h:6`) and are cleared on every map load.
pub const TEMP_VAR_BASE: u16 = VAR_BASE;
/// `NUM_TEMP_VARS` (`vars.h:40`).
pub const NUM_TEMP_VARS: usize = 32;
/// `VAR_OBJ_GFX_BASE` — the object-graphics variables follow the
/// temporaries (`vars.h:42`).
pub const VAR_OBJ_GFX_BASE: u16 = TEMP_VAR_BASE + NUM_TEMP_VARS as u16;
/// `NUM_OBJ_GFX_VARS` (`vars.h:61`).
pub const NUM_OBJ_GFX_VARS: usize = 16;
/// `VAR_PLAYER_STARTER` (`vars.h:63`) — `Save_VarsFlags_SetStarter`.
pub const VAR_PLAYER_STARTER: u16 = 0x4030;
/// `VAR_MAGIKARP_SIZE_RECORD` (`vars.h:68`) — the fishing record the
/// new game seeds with 56150.
pub const VAR_MAGIKARP_SIZE_RECORD: u16 = 0x4035;
/// `VAR_LOTO_NUMBER_LO` (`vars.h:75`).
pub const VAR_LOTO_NUMBER_LO: u16 = 0x403C;
/// `VAR_LOTO_NUMBER_HI` (`vars.h:76`).
pub const VAR_LOTO_NUMBER_HI: u16 = 0x403D;

/// `SPECIAL_VAR_BASE` — the first environment-held variable
/// (`vars.h:386`).
pub const SPECIAL_VAR_BASE: u16 = 0x8000;
/// `NUM_SPECIAL_VARS` (`vars.h:387`): `0x8000..=0x800D`.
pub const NUM_SPECIAL_VARS: usize = 14;
/// `VAR_SPECIAL_RESULT` (`vars.h:401`).
pub const VAR_SPECIAL_RESULT: u16 = 0x800C;
/// `VAR_SPECIAL_LAST_TALKED` (`vars.h:402`).
pub const VAR_SPECIAL_LAST_TALKED: u16 = 0x800D;

/// `NUM_FLAGS` (`flags.h:2223`).
pub const NUM_FLAGS: usize = 2912;
/// `MAPTEMP_FLAG_BASE` / `NUM_MAPTEMP_FLAGS` (`flags.h:15-16`) — the
/// per-map temporaries `ClearTempFieldEventData` wipes.
pub const MAPTEMP_FLAG_BASE: u16 = 1;
/// `NUM_MAPTEMP_FLAGS` (`flags.h:16`).
pub const NUM_MAPTEMP_FLAGS: usize = 64;
/// `HIDDEN_ITEMS_FLAG_BASE` (`flags.h:825`).
pub const HIDDEN_ITEMS_FLAG_BASE: u16 = 800;
/// `TRAINER_FLAG_BASE` (`flags.h:1394`).
pub const TRAINER_FLAG_BASE: u16 = 0x550;
/// `FLAG_UNK_960` (`flags.h:1706`) — the one flag the new game sets.
pub const FLAG_UNK_960: u16 = 0x960;
/// `DAILY_FLAG_BASE` / `NUM_DAILY_FLAGS` (`flags.h:2027-2028`).
pub const DAILY_FLAG_BASE: u16 = 0xAA0;
/// `NUM_DAILY_FLAGS` (`flags.h:2028`).
pub const NUM_DAILY_FLAGS: usize = 192;
/// `TEMP_FLAG_BASE` (`flags.h:2226`) — from here up, flags are not
/// saved ([`TempFlags`]).
pub const TEMP_FLAG_BASE: u16 = 0x4000;
/// `NUM_TEMP_FLAGS` (`flags.h:2225`).
pub const NUM_TEMP_FLAGS: usize = 64;

/// Byte offset of `flags[]` inside the block (`sizeof(u16[NUM_VARS])`).
pub const FLAGS_OFFSET: usize = NUM_VARS * 2;
/// `sizeof(SaveVarsFlags)` — `Save_VarsFlags_sizeof`.
pub const SIZE: usize = FLAGS_OFFSET + NUM_FLAGS / 8;

/// Failures constructing a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarsFlagsError {
    /// The block is shorter than `sizeof(SaveVarsFlags)`.
    TooShort {
        /// The length offered.
        got: usize,
    },
}

impl fmt::Display for VarsFlagsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { got } => {
                write!(f, "SaveVarsFlags needs {SIZE:#x} bytes, got {got:#x}")
            }
        }
    }
}

impl std::error::Error for VarsFlagsError {}

/// Whether `flag` lives in [`TempFlags`] rather than the save block.
#[must_use]
pub const fn is_temp_flag(flag: u16) -> bool {
    flag >= TEMP_FLAG_BASE
}

/// Whether `var` is a saved variable id (`VAR_BASE..=VARS_END`).
#[must_use]
pub const fn is_saved_var(var: u16) -> bool {
    var >= VAR_BASE && var <= VARS_END
}

/// Whether `var` is an environment special variable id.
#[must_use]
pub const fn is_special_var(var: u16) -> bool {
    var >= SPECIAL_VAR_BASE && (var as usize) < SPECIAL_VAR_BASE as usize + NUM_SPECIAL_VARS
}

/// A typed view over the block's bytes — borrowed (`&[u8]` /
/// `&mut [u8]`, straight off a [`SaveData`](super::SaveData) block) or
/// owned (`Vec<u8>`), read-only or mutable by the backing type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarsFlags<B> {
    block: B,
}

impl<B: AsRef<[u8]>> VarsFlags<B> {
    /// Views `block` (at least [`SIZE`] bytes; a stored save block is
    /// longer by its padding and CRC, which the view never touches).
    ///
    /// # Errors
    /// [`VarsFlagsError::TooShort`] for anything shorter.
    pub fn new(block: B) -> Result<Self, VarsFlagsError> {
        let got = block.as_ref().len();
        if got < SIZE {
            return Err(VarsFlagsError::TooShort { got });
        }
        Ok(Self { block })
    }

    /// The backing bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.block.as_ref()[..SIZE]
    }

    /// Gives the backing store back.
    pub fn into_inner(self) -> B {
        self.block
    }

    /// `vars[index]` — raw array access.
    #[must_use]
    pub fn var_index(&self, index: usize) -> Option<u16> {
        if index >= NUM_VARS {
            return None;
        }
        let b = self.bytes();
        Some(u16::from_le_bytes([b[index * 2], b[index * 2 + 1]]))
    }

    /// `*Save_VarsFlags_GetVarAddr(var)` — the variable with script id
    /// `var`; `None` outside `VAR_BASE..=VARS_END` (where the original
    /// asserts).
    #[must_use]
    pub fn var(&self, var: u16) -> Option<u16> {
        if !is_saved_var(var) {
            return None;
        }
        self.var_index(usize::from(var - VAR_BASE))
    }

    /// `Save_VarsFlags_CheckFlagInArray` — whether saved flag `flag` is
    /// set. Flag 0 and every id outside the block (temporaries
    /// included) read as unset.
    #[must_use]
    pub fn flag(&self, flag: u16) -> bool {
        match flag_byte(flag) {
            Some(byte) => self.bytes()[byte] & (1 << (flag % 8)) != 0,
            None => false,
        }
    }
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> VarsFlags<B> {
    /// `vars[index] = value` — raw array access; `false` out of range.
    pub fn set_var_index(&mut self, index: usize, value: u16) -> bool {
        if index >= NUM_VARS {
            return false;
        }
        self.block.as_mut()[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        true
    }

    /// `*Save_VarsFlags_GetVarAddr(var) = value`; `false` (nothing
    /// written) outside `VAR_BASE..=VARS_END`.
    pub fn set_var(&mut self, var: u16, value: u16) -> bool {
        if !is_saved_var(var) {
            return false;
        }
        self.set_var_index(usize::from(var - VAR_BASE), value)
    }

    /// `Save_VarsFlags_SetFlagInArray`; `false` (nothing written) for
    /// flag 0 and ids outside the block.
    pub fn set_flag(&mut self, flag: u16) -> bool {
        match flag_byte(flag) {
            Some(byte) => {
                self.block.as_mut()[byte] |= 1 << (flag % 8);
                true
            }
            None => false,
        }
    }

    /// `Save_VarsFlags_ClearFlagInArray`; `false` for flag 0 and ids
    /// outside the block.
    pub fn clear_flag(&mut self, flag: u16) -> bool {
        match flag_byte(flag) {
            Some(byte) => {
                self.block.as_mut()[byte] &= 0xFF ^ (1 << (flag % 8));
                true
            }
            None => false,
        }
    }

    /// `ClearTempFieldEventData` (`src/script_manager.c:399`): zeroes the
    /// [`NUM_MAPTEMP_FLAGS`] map-temporary flags (the 8 bytes from
    /// `flags[MAPTEMP_FLAG_BASE / 8]`, so bit 0 — flag 0 — goes too)
    /// and the [`NUM_TEMP_VARS`] temporary variables.
    pub fn clear_temp_field_event_data(&mut self) {
        let b = self.block.as_mut();
        let flags = FLAGS_OFFSET + usize::from(MAPTEMP_FLAG_BASE / 8);
        b[flags..flags + NUM_MAPTEMP_FLAGS / 8].fill(0);
        let vars = usize::from(TEMP_VAR_BASE - VAR_BASE) * 2;
        b[vars..vars + NUM_TEMP_VARS * 2].fill(0);
    }

    /// `ClearDailyFlags` (`src/script_manager.c:410`): zeroes the
    /// [`NUM_DAILY_FLAGS`] daily flags.
    pub fn clear_daily_flags(&mut self) {
        let flags = FLAGS_OFFSET + usize::from(DAILY_FLAG_BASE / 8);
        self.block.as_mut()[flags..flags + NUM_DAILY_FLAGS / 8].fill(0);
    }
}

/// The block byte holding saved flag `flag`, or `None` for flag 0 and
/// ids outside the saved array (`Save_VarsFlags_GetFlagAddr`).
fn flag_byte(flag: u16) -> Option<usize> {
    if flag == 0 || is_temp_flag(flag) || usize::from(flag / 8) >= NUM_FLAGS / 8 {
        return None;
    }
    Some(FLAGS_OFFSET + usize::from(flag / 8))
}

/// The static, never-saved `sTempFlags[NUM_TEMP_FLAGS / 8]`
/// (`src/save_vars_flags.c:5`): flags `TEMP_FLAG_BASE..TEMP_FLAG_BASE +
/// NUM_TEMP_FLAGS`, zero at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TempFlags {
    bits: [u8; NUM_TEMP_FLAGS / 8],
}

impl TempFlags {
    /// All clear — the state at boot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bits: [0; NUM_TEMP_FLAGS / 8],
        }
    }

    /// The byte holding `flag`, if it is a temporary flag.
    fn byte(flag: u16) -> Option<usize> {
        let index = usize::from(flag.checked_sub(TEMP_FLAG_BASE)?);
        (index < NUM_TEMP_FLAGS).then_some(index / 8)
    }

    /// Whether temporary flag `flag` is set; `false` for any other id.
    #[must_use]
    pub fn flag(&self, flag: u16) -> bool {
        Self::byte(flag).is_some_and(|byte| self.bits[byte] & (1 << (flag % 8)) != 0)
    }

    /// Sets temporary flag `flag`; `false` for any other id.
    pub fn set_flag(&mut self, flag: u16) -> bool {
        match Self::byte(flag) {
            Some(byte) => {
                self.bits[byte] |= 1 << (flag % 8);
                true
            }
            None => false,
        }
    }

    /// Clears temporary flag `flag`; `false` for any other id.
    pub fn clear_flag(&mut self, flag: u16) -> bool {
        match Self::byte(flag) {
            Some(byte) => {
                self.bits[byte] &= 0xFF ^ (1 << (flag % 8));
                true
            }
            None => false,
        }
    }

    /// The raw bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{BLOCK_RAW_SIZES, block};

    #[test]
    fn layout_is_the_block_size() {
        assert_eq!(FLAGS_OFFSET, 0x2E0);
        assert_eq!(SIZE, 0x44C);
        assert_eq!(SIZE, BLOCK_RAW_SIZES[block::FLAGS] as usize);
        assert_eq!(VARS_END, 0x416F);
        assert!(is_saved_var(VAR_BASE) && is_saved_var(VARS_END));
        assert!(!is_saved_var(VAR_BASE - 1) && !is_saved_var(VARS_END + 1));
        assert!(is_special_var(0x8000) && is_special_var(0x800D) && !is_special_var(0x800E));
        assert!(is_temp_flag(0x4000) && !is_temp_flag(0x3FFF));
    }

    #[test]
    fn view_needs_a_whole_block() {
        assert_eq!(
            VarsFlags::new(vec![0u8; SIZE - 1]).err(),
            Some(VarsFlagsError::TooShort { got: SIZE - 1 })
        );
        // A stored block is longer (padding + CRC): the extra is ignored.
        let view = VarsFlags::new(vec![0u8; SIZE + 4]).unwrap();
        assert_eq!(view.bytes().len(), SIZE);
    }

    #[test]
    fn vars_are_base_relative_halfwords() {
        let mut view = VarsFlags::new(vec![0u8; SIZE]).unwrap();
        assert!(view.set_var(VAR_MAGIKARP_SIZE_RECORD, 56150));
        assert_eq!(view.var(VAR_MAGIKARP_SIZE_RECORD), Some(56150));
        assert_eq!(view.var_index(0x35), Some(56150));
        assert_eq!(&view.bytes()[0x6A..0x6C], &56150u16.to_le_bytes());
        // First and last ids; nothing outside.
        assert!(view.set_var(VAR_BASE, 1) && view.set_var(VARS_END, 2));
        assert_eq!(view.var_index(0), Some(1));
        assert_eq!(view.var_index(NUM_VARS - 1), Some(2));
        assert_eq!(view.var_index(NUM_VARS), None);
        assert!(!view.set_var(VAR_BASE - 1, 9) && !view.set_var(VARS_END + 1, 9));
        assert!(!view.set_var(0x8000, 9), "special vars are not in the block");
        assert_eq!(view.var(0x8000), None);
        // Nothing leaked past the vars array.
        assert!(view.bytes()[FLAGS_OFFSET..].iter().all(|&b| b == 0));
    }

    #[test]
    fn flags_are_bits_after_the_vars() {
        let mut view = VarsFlags::new(vec![0u8; SIZE]).unwrap();
        assert!(view.set_flag(FLAG_UNK_960));
        assert!(view.flag(FLAG_UNK_960));
        assert_eq!(view.bytes()[0x2E0 + 0x960 / 8], 1);
        assert!(!view.flag(FLAG_UNK_960 + 1));
        assert!(view.set_flag(FLAG_UNK_960 + 3));
        assert_eq!(view.bytes()[0x2E0 + 0x960 / 8], 0b1001);
        assert!(view.clear_flag(FLAG_UNK_960));
        assert_eq!(view.bytes()[0x2E0 + 0x960 / 8], 0b1000);
        // Flag 0 is reserved: unset, unsettable.
        assert!(!view.set_flag(0) && !view.flag(0));
        // The last flag exists; the next does not; temporaries are elsewhere.
        assert!(view.set_flag(NUM_FLAGS as u16 - 1));
        assert!(view.flag(NUM_FLAGS as u16 - 1));
        assert!(!view.set_flag(NUM_FLAGS as u16) && !view.flag(NUM_FLAGS as u16));
        assert!(!view.set_flag(TEMP_FLAG_BASE) && !view.flag(TEMP_FLAG_BASE));
        assert_eq!(view.bytes()[SIZE - 1], 0x80);
        // The var array is untouched.
        assert!(view.bytes()[..FLAGS_OFFSET].iter().all(|&b| b == 0));
    }

    #[test]
    fn temporaries_clear_as_the_field_does() {
        let mut view = VarsFlags::new(vec![0xFFu8; SIZE]).unwrap();
        view.clear_temp_field_event_data();
        // 32 temp vars (64 bytes) and 64 map-temp flags (8 bytes) go.
        assert!(view.bytes()[..NUM_TEMP_VARS * 2].iter().all(|&b| b == 0));
        assert_eq!(view.bytes()[NUM_TEMP_VARS * 2], 0xFF);
        assert!(view.bytes()[FLAGS_OFFSET..FLAGS_OFFSET + 8].iter().all(|&b| b == 0));
        assert_eq!(view.bytes()[FLAGS_OFFSET + 8], 0xFF);
        view.clear_daily_flags();
        let daily = FLAGS_OFFSET + usize::from(DAILY_FLAG_BASE / 8);
        assert!(view.bytes()[daily..daily + NUM_DAILY_FLAGS / 8].iter().all(|&b| b == 0));
        assert_eq!(view.bytes()[daily - 1], 0xFF);
        // The daily flags are the last 24 bytes of the block.
        assert_eq!(daily + NUM_DAILY_FLAGS / 8, SIZE);
    }

    #[test]
    fn temp_flags_live_off_the_save() {
        let mut temp = TempFlags::new();
        assert!(temp.set_flag(TEMP_FLAG_BASE));
        assert!(temp.set_flag(TEMP_FLAG_BASE + 63));
        assert!(temp.flag(TEMP_FLAG_BASE) && temp.flag(TEMP_FLAG_BASE + 63));
        assert!(!temp.flag(TEMP_FLAG_BASE + 1));
        assert!(!temp.set_flag(TEMP_FLAG_BASE + 64) && !temp.set_flag(1));
        assert!(temp.clear_flag(TEMP_FLAG_BASE));
        assert!(!temp.flag(TEMP_FLAG_BASE));
        assert_eq!(temp.bytes(), &[0, 0, 0, 0, 0, 0, 0, 0x80]);
    }

    #[test]
    fn borrowed_views_edit_in_place() {
        let mut raw = vec![0u8; SIZE];
        {
            let mut view = VarsFlags::new(raw.as_mut_slice()).unwrap();
            view.set_var(VAR_PLAYER_STARTER, 152);
            view.set_flag(TRAINER_FLAG_BASE);
        }
        let view = VarsFlags::new(raw.as_slice()).unwrap();
        assert_eq!(view.var(VAR_PLAYER_STARTER), Some(152));
        assert!(view.flag(TRAINER_FLAG_BASE));
    }
}
