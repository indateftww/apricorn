//! Map objects and their movement-command step machine.
//!
//! The state subset of pret's `LocalMapObject` (`include/map_object.h:48`,
//! 0x12C bytes) that movement touches, the C accessors from
//! `src/map_object.c`, and the per-frame step machine that pret only has
//! as asm:
//!
//! * `asm/unk_data_020FCBD8.s:623` — `gMovementCmdTable`, 113 entries,
//!   each pointing at a step-function array (`gMovementCmdSteps_NNN`,
//!   `asm/unk_data_020FD978.s:45` onward, continued in `unk_data_020FDB44.s`).
//! * `asm/unk_02062108.s:471` — `sub_02062400`, the runner: step
//!   functions are called in order while they return 1.
//! * `asm/unk_02062108.s:595` — `sub_020624CC`, the linear-move init;
//!   `:629` — `MapObjectMovementCmd090_Step1`, the per-frame advance;
//!   `:1044` — `sub_020627B0`, the walk-on-spot init; `:1069` —
//!   `MapObjectMovementCmd040_Step1`; `:534` —
//!   `MapObjectMovementCmd098_Step2`, the shared terminal step.
//! * `asm/unk_0205FD20.s:2395` — `sub_0206101C` (position advance),
//!   `:2284` — `sub_02060F24` (tile step), `:2319` — `sub_02060F78`
//!   (previous := current), `:2264` — the direction delta tables.
//! * `asm/unk_0205FD20.s:28` — `sub_0205FD30`, the object's per-frame
//!   SysTask body (housekeeping around the runner), `:2516` —
//!   `sub_02061108`, the standing-tile behaviour refresh.
//! * `asm/unk_02062108.s:170` — `EventObjectMovementMan_Create` and
//!   the `MovementScriptMachine` states (`:241`–`:352`) that apply a
//!   scripted movement list.
//!
//! Everything here is integer arithmetic on `fx32` (1/4096) values;
//! the machine is a pure function of its state, so a frame replays.

/// The fixed-point unit: `FX32_ONE`.
pub const FX32_ONE: i32 = 4096;
/// One tile in world units: 16 units, `16 * FX32_ONE`.
pub const TILE_FX32: i32 = 16 * FX32_ONE;
/// Half a tile: objects stand on tile centres (`src/map_object.c:1992`).
pub const HALF_TILE_FX32: i32 = 8 * FX32_ONE;
/// One elevation level: 8 world units (`src/map_object.c:1993`).
pub const Y_UNIT_FX32: i32 = 8 * FX32_ONE;

/// The attribute word the collision trait reports for a tile outside
/// the loaded map: behaviour `TILE_BEHAVIOR_NONE` (0xFF) in the low
/// byte, passable bit clear — exactly what the ROM's two failed
/// lookups combine to (`GetMetatileBehavior` → 0xFF, `sub_020548C0`
/// → 0; `asm/unk_02054648.s:376,:431`).
pub const ATTR_NONE: u16 = 0x00FF;

/// `TILE_BEHAVIOR_NONE` (`include/constants/metatile_behavior.h:245`).
pub const BEHAVIOR_NONE: u8 = 0xFF;

/// Terrain attributes as the field collision code reads them: one
/// `u16` per tile whose low byte is the metatile behaviour and whose
/// bit 15 is the impassable flag (`asm/unk_02054648.s:376`,
/// `sub_020548C0`; `:431`, `GetMetatileBehavior`). Tiles outside the
/// loaded map must report [`ATTR_NONE`]. Implemented by the real
/// terrain tables and by test stubs.
pub trait Collision {
    /// The attribute word of tile `(x, z)` (tile units, z south).
    fn attr(&self, x: i32, z: i32) -> u16;

    /// `sub_020548C0`: bit 15 of the attribute word.
    fn impassable(&self, x: i32, z: i32) -> bool {
        self.attr(x, z) & 0x8000 != 0
    }

    /// `GetMetatileBehavior`: the low byte, 0xFF outside the map.
    fn behavior(&self, x: i32, z: i32) -> u8 {
        (self.attr(x, z) & 0xFF) as u8
    }

    /// `MetatileBehavior_IsSurfableWater` (`src/metatile_behavior.c:314`):
    /// bit 0 of the ROM's `sMetatileBehaviorFlags[behavior]` — a
    /// 256-entry table that lives with the terrain data, not here.
    /// Defaults to "never water"; the real terrain overrides it.
    fn surfable(&self, _x: i32, _z: i32) -> bool {
        false
    }
}

/// A map with no tiles at all: every lookup fails (out-of-map
/// semantics), which blocks every step — useful as the terrain of an
/// object whose position is scripted rather than collided.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoTerrain;

impl Collision for NoTerrain {
    fn attr(&self, _x: i32, _z: i32) -> u16 {
        ATTR_NONE
    }
}

/// A facing / travel direction (`include/constants/global_fieldmap.h:5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Direction {
    /// `DIR_NORTH` (z decreasing).
    North = 0,
    /// `DIR_SOUTH` (z increasing).
    South = 1,
    /// `DIR_WEST` (x decreasing).
    West = 2,
    /// `DIR_EAST` (x increasing).
    East = 3,
}

impl Direction {
    /// All four, in `DIR_*` order.
    pub const ALL: [Direction; 4] = [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];

    /// The direction with `DIR_*` value `index`, if 0–3.
    #[must_use]
    pub fn from_index(index: u32) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    /// The `DIR_*` value.
    #[must_use]
    pub fn index(self) -> u32 {
        self as u32
    }

    /// `GetDeltaXByFacingDirection` — `_020FD4AC`: {0, 0, -1, 1}.
    #[must_use]
    pub fn delta_x(self) -> i32 {
        match self {
            Direction::North | Direction::South => 0,
            Direction::West => -1,
            Direction::East => 1,
        }
    }

    /// `GetDeltaYByFacingDirection` (a z delta) — `_020FD49C`:
    /// {-1, 1, 0, 0}.
    #[must_use]
    pub fn delta_z(self) -> i32 {
        match self {
            Direction::North => -1,
            Direction::South => 1,
            Direction::West | Direction::East => 0,
        }
    }

    /// `sub_020611F4` — `_020FD4DC`: the opposite direction.
    #[must_use]
    pub fn reverse(self) -> Self {
        match self {
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }
}

/// `VecFx32` — three fixed-point components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VecFx32 {
    /// X (east positive).
    pub x: i32,
    /// Y (up positive).
    pub y: i32,
    /// Z (south positive).
    pub z: i32,
}

impl VecFx32 {
    /// The world position of tile `(x, z)` at elevation `y`
    /// (`MapObject_SetPositionFromXYZAndDirection`,
    /// `src/map_object.c:1989`): tile centre, elevation in 8-unit steps.
    #[must_use]
    pub fn from_tile(x: i32, y: i32, z: i32) -> Self {
        Self {
            x: x * TILE_FX32 + HALF_TILE_FX32,
            y: y * Y_UNIT_FX32,
            z: z * TILE_FX32 + HALF_TILE_FX32,
        }
    }
}

/// `MapObjectFlagBits` (`include/map_object.h:116`) — the bits movement
/// reads and writes. Named where the asm fixes the meaning; pret's
/// `UNKn` names are kept in the docs so cross-referencing stays easy.
pub mod flag {
    /// `MAPOBJECTFLAG_ACTIVE`.
    pub const ACTIVE: u32 = 1 << 0;
    /// `MAPOBJECTFLAG_SINGLE_MOVEMENT`.
    pub const SINGLE_MOVEMENT: u32 = 1 << 1;
    /// `MAPOBJECTFLAG_START_MOVEMENT` — set by a movement's first step,
    /// consumed by the effect hooks at the end of the same tick.
    pub const START_MOVEMENT: u32 = 1 << 2;
    /// `MAPOBJECTFLAG_END_MOVEMENT` — set by a movement's completing
    /// step, consumed at the end of the same tick.
    pub const END_MOVEMENT: u32 = 1 << 3;
    /// `MAPOBJECTFLAG_UNK4`: a held movement command is loaded
    /// (`MapObject_SetHeldMovement`, `asm/unk_02062108.s:62`).
    pub const HELD_MOVEMENT: u32 = 1 << 4;
    /// `MAPOBJECTFLAG_UNK5`: the held movement has finished
    /// (`MapObjectMovementCmd098_Step2`, `asm/unk_02062108.s:534`).
    pub const HELD_MOVEMENT_DONE: u32 = 1 << 5;
    /// `MAPOBJECTFLAG_MOVEMENT_PAUSED`.
    pub const MOVEMENT_PAUSED: u32 = 1 << 6;
    /// `MAPOBJECTFLAG_UNK7`: `MapObject_SetFacingDirection` is a no-op
    /// while set (`src/map_object.c:1103`).
    pub const FACING_LOCKED: u32 = 1 << 7;
    /// `MAPOBJECTFLAG_UNK8`.
    pub const UNK8: u32 = 1 << 8;
    /// `MAPOBJECTFLAG_VISIBLE`.
    pub const VISIBLE: u32 = 1 << 9;
    /// `MAPOBJECTFLAG_KEEP`.
    pub const KEEP: u32 = 1 << 10;
    /// `MAPOBJECTFLAG_UNK11`: the standing-tile behaviour is stale and
    /// must be refreshed by the next tick (`sub_0205FE24`).
    pub const BEHAVIOR_STALE: u32 = 1 << 11;
    /// `MAPOBJECTFLAG_UNK12`: the height must be re-fetched by the next
    /// tick (`sub_0205FE0C`).
    pub const HEIGHT_STALE: u32 = 1 << 12;
    /// `MAPOBJECTFLAG_UNK13`.
    pub const UNK13: u32 = 1 << 13;
    /// `MAPOBJECTFLAG_UNK14`: the sprite is attached.
    pub const UNK14: u32 = 1 << 14;
    /// `MAPOBJECTFLAG_UNK16`: start-effect variant, consumed with
    /// [`START_MOVEMENT`].
    pub const UNK16: u32 = 1 << 16;
    /// `MAPOBJECTFLAG_UNK17`: end-effect variant, consumed with
    /// [`END_MOVEMENT`].
    pub const UNK17: u32 = 1 << 17;
    /// `MAPOBJECTFLAG_IGNORE_HEIGHTS`.
    pub const IGNORE_HEIGHTS: u32 = 1 << 23;
    /// `MAPOBJECTFLAG_UNK29`: the player's height-lookup mode.
    pub const UNK29: u32 = 1 << 29;
    /// `MAPOBJECTFLAG_UNK30`: also pauses movement
    /// (`MapObject_CheckMovementPaused`).
    pub const UNK30: u32 = 1 << 30;
}

/// The command families the direction tables `_020FD198`
/// (`asm/unk_data_020FCBD8.s:540`) know: a family is the command for
/// `DIR_NORTH`; the other three directions follow in `DIR_*` order.
pub mod family {
    /// Face without moving (commands 000–003).
    pub const FACE: u8 = 0;
    /// Walk one tile in 32 frames, speed 0x800 (004–007).
    pub const WALK_SLOWEST: u8 = 4;
    /// Walk one tile in 16 frames, speed 0x1000 (008–011).
    pub const WALK_SLOWER: u8 = 8;
    /// Walk one tile in 8 frames, speed 0x2000 (012–015) — the
    /// player's normal step (`MOVEMENT_STEP_*`).
    pub const WALK_NORMAL: u8 = 12;
    /// Walk one tile in 4 frames, speed 0x4000, anim group 4 (016–019).
    pub const WALK_FASTER: u8 = 16;
    /// Walk one tile in 2 frames, speed 0x8000 (020–023).
    pub const WALK_FASTEST: u8 = 20;
    /// Walk on the spot for 32 frames, anim group 1 (024–027).
    pub const WALK_ON_SPOT_SLOWEST: u8 = 24;
    /// Walk on the spot for 16 frames, anim group 2 (028–031) — the
    /// player's blocked-step "bump" (`asm/unk_0205CB48.s`, `sub_0205D4B4`).
    pub const WALK_ON_SPOT_SLOWER: u8 = 28;
    /// Walk on the spot for 8 frames, anim group 3 (032–035).
    pub const WALK_ON_SPOT_NORMAL: u8 = 32;
    /// Walk on the spot for 4 frames, anim group 4 (036–039).
    pub const WALK_ON_SPOT_FASTER: u8 = 36;
    /// Walk on the spot for 2 frames, anim group 5 (040–043) — the
    /// player's turn in place (`sub_0205D610`).
    pub const WALK_ON_SPOT_FASTEST: u8 = 40;
    /// Jump on the spot, 16 frames (044–047).
    pub const JUMP_ON_SPOT: u8 = 44;
    /// Jump one tile, 8 frames (048–051).
    pub const JUMP_1: u8 = 48;
    /// Jump one tile, 8 frames, variant (052–055).
    pub const JUMP_1_ALT: u8 = 52;
    /// Jump two tiles, 16 frames (056–059) — the ledge hop
    /// (`sub_0205D4B4` picks 0x38 when the target is a `JUMP_*` tile).
    pub const JUMP_2: u8 = 56;
    /// Directional animation family 076–079.
    pub const CMD_076: u8 = 76;
    /// Directional animation family 080–083.
    pub const CMD_080: u8 = 80;
    /// Move one tile in 1 frame, speed 0x10000, anim group 0 (084–087).
    pub const WALK_INSTANT: u8 = 84;
    /// Run one tile in 4 frames, speed 0x4000, anim group 9 (088–091)
    /// — what the player's running shoes select (0x58, `sub_0205D4B4`).
    pub const RUN: u8 = 88;
    /// Jump family 092–095.
    pub const CMD_092: u8 = 92;
    /// Directional animation family 096–099.
    pub const CMD_096: u8 = 96;
}

/// `MOVEMENT_STEP_END` (`include/constants/movements.h:22`).
pub const MOVEMENT_STEP_END: u8 = 254;
/// `MOVEMENT_NONE` — no held movement (`include/constants/movements.h:23`).
pub const MOVEMENT_NONE: u8 = 255;
/// The number of entries in `gMovementCmdTable`.
pub const MOVEMENT_CMD_COUNT: u8 = 113;

/// The rows of `_020FD198` (`asm/unk_data_020FCBD8.s:540`): each row
/// lists the commands for `DIR_NORTH..DIR_EAST`. `sub_0206234C` finds
/// the row that contains the requested family and returns its entry
/// for the direction; `sub_02062390` finds a command and returns its
/// column.
const DIRECTION_ROWS: [[u8; 4]; 22] = [
    [0x00, 0x01, 0x02, 0x03],
    [0x04, 0x05, 0x06, 0x07],
    [0x08, 0x09, 0x0A, 0x0B],
    [0x0C, 0x0D, 0x0E, 0x0F],
    [0x10, 0x11, 0x12, 0x13],
    [0x14, 0x15, 0x16, 0x17],
    [0x18, 0x19, 0x1A, 0x1B],
    [0x1C, 0x1D, 0x1E, 0x1F],
    [0x20, 0x21, 0x22, 0x23],
    [0x24, 0x25, 0x26, 0x27],
    [0x28, 0x29, 0x2A, 0x2B],
    [0x2C, 0x2D, 0x2E, 0x2F],
    [0x30, 0x31, 0x32, 0x33],
    [0x34, 0x35, 0x36, 0x37],
    [0x38, 0x39, 0x3A, 0x3B],
    [0x4C, 0x4D, 0x4E, 0x4F],
    [0x50, 0x51, 0x52, 0x53],
    [0x54, 0x55, 0x56, 0x57],
    [0x58, 0x59, 0x5A, 0x5B],
    [0x5C, 0x5D, 0x5C, 0x5D],
    [0x5E, 0x5F, 0x5E, 0x5F],
    [0x60, 0x61, 0x62, 0x63],
];

/// A movement command — an index into `gMovementCmdTable`
/// (`asm/unk_data_020FCBD8.s:623`), plus the two out-of-table markers.
///
/// Names follow the step-function families read off the asm (each
/// variant's doc names its `Step0` initialiser). Families the machine
/// implements: 000–043 (face, linear walks, walk-on-spot) and 084–091
/// (instant step, run). Every other command is `Unimplemented` at
/// run time — reported, not silently skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum MovementCmd {
    /// `MOVEMENT_FACE_UP` — `sub_0206247C`.
    FaceNorth = 0,
    FaceSouth = 1,
    FaceWest = 2,
    FaceEast = 3,
    /// 32-frame walk — `sub_020624CC(dir, 0x800, 32, 1)`.
    WalkSlowestNorth = 4,
    WalkSlowestSouth = 5,
    WalkSlowestWest = 6,
    WalkSlowestEast = 7,
    /// 16-frame walk — `sub_020624CC(dir, 0x1000, 16, 2)`.
    WalkSlowerNorth = 8,
    WalkSlowerSouth = 9,
    WalkSlowerWest = 10,
    WalkSlowerEast = 11,
    /// `MOVEMENT_STEP_UP` — `sub_020624CC(dir, 0x2000, 8, 3)`.
    WalkNormalNorth = 12,
    WalkNormalSouth = 13,
    WalkNormalWest = 14,
    WalkNormalEast = 15,
    /// `MOVEMENT_RUN_UP` in pret's naming — `sub_020624CC(dir, 0x4000, 4, 4)`.
    WalkFasterNorth = 16,
    WalkFasterSouth = 17,
    WalkFasterWest = 18,
    WalkFasterEast = 19,
    /// 2-frame walk — `sub_020624CC(dir, 0x8000, 2, 5)`.
    WalkFastestNorth = 20,
    WalkFastestSouth = 21,
    WalkFastestWest = 22,
    WalkFastestEast = 23,
    /// `sub_020627B0(dir, 32, 1)`.
    WalkOnSpotSlowestNorth = 24,
    WalkOnSpotSlowestSouth = 25,
    WalkOnSpotSlowestWest = 26,
    WalkOnSpotSlowestEast = 27,
    /// `sub_020627B0(dir, 16, 2)` — the blocked-walk bump.
    WalkOnSpotSlowerNorth = 28,
    WalkOnSpotSlowerSouth = 29,
    WalkOnSpotSlowerWest = 30,
    WalkOnSpotSlowerEast = 31,
    /// `sub_020627B0(dir, 8, 3)`.
    WalkOnSpotNormalNorth = 32,
    WalkOnSpotNormalSouth = 33,
    WalkOnSpotNormalWest = 34,
    WalkOnSpotNormalEast = 35,
    /// `sub_020627B0(dir, 4, 4)` (`MOVEMENT_WALK_IN_PLACE_FACE_DOWN` = 37).
    WalkOnSpotFasterNorth = 36,
    WalkOnSpotFasterSouth = 37,
    WalkOnSpotFasterWest = 38,
    WalkOnSpotFasterEast = 39,
    /// `sub_020627B0(dir, 2, 5)` — the player's turn in place.
    WalkOnSpotFastestNorth = 40,
    WalkOnSpotFastestSouth = 41,
    WalkOnSpotFastestWest = 42,
    WalkOnSpotFastestEast = 43,
    /// Jump family (`sub_02062958`, 0 tiles, 16 frames) — unimplemented.
    JumpOnSpotNorth = 44,
    JumpOnSpotSouth = 45,
    JumpOnSpotWest = 46,
    JumpOnSpotEast = 47,
    /// `MOVEMENT_WALK_UNK_48` — jump family, 8 frames — unimplemented.
    Jump1North = 48,
    Jump1South = 49,
    Jump1West = 50,
    Jump1East = 51,
    /// Jump family, 8 frames, variant — unimplemented.
    Jump1AltNorth = 52,
    Jump1AltSouth = 53,
    Jump1AltWest = 54,
    Jump1AltEast = 55,
    /// Jump family, 2 tiles, 16 frames (the ledge hop) — unimplemented.
    Jump2North = 56,
    Jump2South = 57,
    Jump2West = 58,
    Jump2East = 59,
    /// `sub_02062D54(1)` — unimplemented.
    Cmd060 = 60,
    /// `sub_02062D54(2)` — unimplemented.
    Cmd061 = 61,
    /// `sub_02062D54(4)` — unimplemented.
    Cmd062 = 62,
    /// `sub_02062D54(8)` — unimplemented.
    Cmd063 = 63,
    /// `sub_02062D54(0xF)` — unimplemented.
    Cmd064 = 64,
    /// `sub_02062D54(0x10)` — unimplemented.
    Cmd065 = 65,
    /// `sub_02062D54(0x20)` — unimplemented.
    Cmd066 = 66,
    /// Timed wait, anim group 1 — unimplemented.
    Cmd067 = 67,
    /// Timed wait, anim group 5 — unimplemented.
    Cmd068 = 68,
    /// Sets `SINGLE_MOVEMENT` — unimplemented.
    Cmd069 = 69,
    /// Clears `SINGLE_MOVEMENT` — unimplemented.
    Cmd070 = 70,
    /// `MOVEMENT_UNK_71`: sets `FACING_LOCKED` — unimplemented.
    LockFacing = 71,
    /// `MOVEMENT_UNK_72`: clears `FACING_LOCKED` — unimplemented.
    UnlockFacing = 72,
    /// Sets `ACTIVE` — unimplemented.
    Cmd073 = 73,
    /// Clears `ACTIVE` — unimplemented.
    Cmd074 = 74,
    /// `MOVEMENT_EMOTE_EXCLAMATION` — `sub_02062F48(0)`, unimplemented.
    EmoteExclamation = 75,
    /// `sub_02062FAC(6, dir)` — unimplemented.
    Cmd076 = 76,
    Cmd077 = 77,
    Cmd078 = 78,
    Cmd079 = 79,
    /// `sub_02062FAC(dir, 3, 7)` — unimplemented.
    Cmd080 = 80,
    Cmd081 = 81,
    Cmd082 = 82,
    Cmd083 = 83,
    /// 1-frame step — `sub_020624CC(dir, 0x10000, 1, 0)`.
    WalkInstantNorth = 84,
    WalkInstantSouth = 85,
    WalkInstantWest = 86,
    WalkInstantEast = 87,
    /// Run — `sub_020624CC(dir, 0x4000, 4, 9)`.
    RunNorth = 88,
    RunSouth = 89,
    RunWest = 90,
    RunEast = 91,
    /// Jump family 092–095 — unimplemented.
    Cmd092 = 92,
    Cmd093 = 93,
    Cmd094 = 94,
    Cmd095 = 95,
    /// `sub_02062FAC(dir, 7, 8)` — unimplemented.
    Cmd096 = 96,
    Cmd097 = 97,
    Cmd098 = 98,
    Cmd099 = 99,
    /// Timed wait, anim group 9 — unimplemented.
    Cmd100 = 100,
    /// Facing-vector animation — unimplemented.
    Cmd101 = 101,
    /// Timed wait — unimplemented.
    Cmd102 = 102,
    /// `sub_02062F48(1)` — the question-mark emote, unimplemented.
    EmoteQuestion = 103,
    /// Timed wait — unimplemented.
    Cmd104 = 104,
    /// Multi-step `sub_020632B0` sequences — unimplemented.
    Cmd105 = 105,
    Cmd106 = 106,
    Cmd107 = 107,
    Cmd108 = 108,
    Cmd109 = 109,
    Cmd110 = 110,
    Cmd111 = 111,
    Cmd112 = 112,
    /// `MOVEMENT_STEP_END` — terminates a movement list; never held.
    End = 254,
    /// `MOVEMENT_NONE` — no held movement.
    None = 255,
}

impl MovementCmd {
    /// The command with table index `value`, if it is one.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        if value < MOVEMENT_CMD_COUNT {
            // SAFETY-free: every value below the count is a variant
            // (the enum is dense over 0..113), so a lookup table of
            // the variants keeps this a plain match.
            Some(ALL_COMMANDS[value as usize])
        } else if value == MOVEMENT_STEP_END {
            Some(MovementCmd::End)
        } else if value == MOVEMENT_NONE {
            Some(MovementCmd::None)
        } else {
            Option::None
        }
    }

    /// The table index.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// `sub_0206234C(direction, family)` (`asm/unk_02062108.s:358`):
    /// the row of `_020FD198` containing `family`, indexed by
    /// `direction`. `None` where pret asserts (`GF_AssertFail`) and
    /// returns the family unchanged.
    #[must_use]
    pub fn for_direction(family: u8, direction: Direction) -> Option<Self> {
        DIRECTION_ROWS
            .iter()
            .find(|row| row.contains(&family))
            .and_then(|row| Self::from_u8(row[direction as usize]))
    }

    /// `sub_02062390(cmd)` (`asm/unk_02062108.s:399`): the column of
    /// the first row containing this command — its direction — or
    /// `None` (pret returns -1) for commands outside the table.
    #[must_use]
    pub fn direction(self) -> Option<Direction> {
        let value = self.as_u8();
        DIRECTION_ROWS.iter().find_map(|row| {
            row.iter()
                .position(|&c| c == value)
                .and_then(|i| Direction::from_index(i as u32))
        })
    }

    /// `sub_0205DE64` (`asm/unk_0205CB48.s:2476`): whether this is a
    /// blocked-walk bump (family 28, commands 0x1C–0x1F).
    #[must_use]
    pub fn is_bump(self) -> bool {
        (0x1C..=0x1F).contains(&self.as_u8())
    }

    /// `sub_0205DE98`'s test: whether this is a run command (0x58–0x5B).
    #[must_use]
    pub fn is_run(self) -> bool {
        (0x58..=0x5B).contains(&self.as_u8())
    }

    /// The linear-move parameters `(direction, speed, frames, anim
    /// group)` this command's `Step0` passes to `sub_020624CC`, if it
    /// is a linear move.
    fn linear(self) -> Option<(Direction, i32, i16, u16)> {
        let value = self.as_u8();
        let dir = Direction::from_index(u32::from(value & 3))?;
        let (speed, frames, anim) = match value & !3 {
            4 => (0x800, 32, 1),
            8 => (0x1000, 16, 2),
            12 => (0x2000, 8, 3),
            16 => (0x4000, 4, 4),
            20 => (0x8000, 2, 5),
            84 => (0x10000, 1, 0),
            88 => (0x4000, 4, 9),
            _ => return Option::None,
        };
        Some((dir, speed, frames, anim))
    }

    /// The walk-on-spot parameters `(direction, frames, anim group)`
    /// this command's `Step0` passes to `sub_020627B0`, if it is one.
    fn on_spot(self) -> Option<(Direction, i16, u16)> {
        let value = self.as_u8();
        let dir = Direction::from_index(u32::from(value & 3))?;
        let (frames, anim) = match value & !3 {
            24 => (32, 1),
            28 => (16, 2),
            32 => (8, 3),
            36 => (4, 4),
            40 => (2, 5),
            _ => return Option::None,
        };
        Some((dir, frames, anim))
    }
}

/// Every table command in index order — the `from_u8` lookup.
const ALL_COMMANDS: [MovementCmd; MOVEMENT_CMD_COUNT as usize] = {
    use MovementCmd::*;
    [
        FaceNorth,
        FaceSouth,
        FaceWest,
        FaceEast,
        WalkSlowestNorth,
        WalkSlowestSouth,
        WalkSlowestWest,
        WalkSlowestEast,
        WalkSlowerNorth,
        WalkSlowerSouth,
        WalkSlowerWest,
        WalkSlowerEast,
        WalkNormalNorth,
        WalkNormalSouth,
        WalkNormalWest,
        WalkNormalEast,
        WalkFasterNorth,
        WalkFasterSouth,
        WalkFasterWest,
        WalkFasterEast,
        WalkFastestNorth,
        WalkFastestSouth,
        WalkFastestWest,
        WalkFastestEast,
        WalkOnSpotSlowestNorth,
        WalkOnSpotSlowestSouth,
        WalkOnSpotSlowestWest,
        WalkOnSpotSlowestEast,
        WalkOnSpotSlowerNorth,
        WalkOnSpotSlowerSouth,
        WalkOnSpotSlowerWest,
        WalkOnSpotSlowerEast,
        WalkOnSpotNormalNorth,
        WalkOnSpotNormalSouth,
        WalkOnSpotNormalWest,
        WalkOnSpotNormalEast,
        WalkOnSpotFasterNorth,
        WalkOnSpotFasterSouth,
        WalkOnSpotFasterWest,
        WalkOnSpotFasterEast,
        WalkOnSpotFastestNorth,
        WalkOnSpotFastestSouth,
        WalkOnSpotFastestWest,
        WalkOnSpotFastestEast,
        JumpOnSpotNorth,
        JumpOnSpotSouth,
        JumpOnSpotWest,
        JumpOnSpotEast,
        Jump1North,
        Jump1South,
        Jump1West,
        Jump1East,
        Jump1AltNorth,
        Jump1AltSouth,
        Jump1AltWest,
        Jump1AltEast,
        Jump2North,
        Jump2South,
        Jump2West,
        Jump2East,
        Cmd060,
        Cmd061,
        Cmd062,
        Cmd063,
        Cmd064,
        Cmd065,
        Cmd066,
        Cmd067,
        Cmd068,
        Cmd069,
        Cmd070,
        LockFacing,
        UnlockFacing,
        Cmd073,
        Cmd074,
        EmoteExclamation,
        Cmd076,
        Cmd077,
        Cmd078,
        Cmd079,
        Cmd080,
        Cmd081,
        Cmd082,
        Cmd083,
        WalkInstantNorth,
        WalkInstantSouth,
        WalkInstantWest,
        WalkInstantEast,
        RunNorth,
        RunSouth,
        RunWest,
        RunEast,
        Cmd092,
        Cmd093,
        Cmd094,
        Cmd095,
        Cmd096,
        Cmd097,
        Cmd098,
        Cmd099,
        Cmd100,
        Cmd101,
        Cmd102,
        EmoteQuestion,
        Cmd104,
        Cmd105,
        Cmd106,
        Cmd107,
        Cmd108,
        Cmd109,
        Cmd110,
        Cmd111,
        Cmd112,
    ]
};

/// What one step function returned: pret's `1` (run the next step in
/// the same tick) or `0` (yield until the next tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// The runner calls the next step now.
    Continue,
    /// The runner stops for this tick.
    Yield,
}

/// An attempt to hold a command the machine cannot run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementError {
    /// `MapObject_SetHeldMovement` asserts `command < 0x71`
    /// (`asm/unk_02062108.s:62`): `End` and `None` are not holdable.
    NotHoldable(MovementCmd),
    /// The command's step family is not ported yet.
    Unimplemented(MovementCmd),
}

impl core::fmt::Display for MovementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MovementError::NotHoldable(cmd) => write!(f, "movement {cmd:?} cannot be held"),
            MovementError::Unimplemented(cmd) => write!(f, "movement {cmd:?} is not implemented"),
        }
    }
}

impl std::error::Error for MovementError {}

/// What one object tick observed — the edge flags the ROM's effect
/// hooks consume before clearing them (`sub_0205FE6C`, `sub_0205FEA4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickReport {
    /// `START_MOVEMENT` was set during the tick (a movement began, or
    /// the standing tile became valid).
    pub started: bool,
    /// `END_MOVEMENT` was set during the tick (a movement completed).
    pub ended: bool,
    /// The runner met a command it cannot execute; the object was
    /// marked finished so the machine stays live.
    pub unimplemented: Option<MovementCmd>,
}

/// The movement-relevant state of a `LocalMapObject`
/// (`include/map_object.h:48`). Field names keep pret's, with the
/// offsets in the docs; the scratch buffer is the 16-byte `unkF8`
/// window the step functions use (`sub_0205F3C0` / `sub_0205F3E4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapObject {
    /// `flags` (+0x00): [`flag`] bits.
    pub flags: u32,
    /// `flags2` (+0x04).
    pub flags2: u32,
    /// `id` (+0x08); `obj_player` = 0xFF for the player.
    pub id: u32,
    /// `spriteId` (+0x10).
    pub sprite_id: u32,
    /// `movement` (+0x14): the autonomous movement type; 1 = player.
    pub movement: u32,
    /// `initialFacing` (+0x24).
    pub initial_facing: Direction,
    /// `currentFacing` (+0x28).
    pub current_facing: Direction,
    /// `nextFacing` (+0x2C).
    pub next_facing: Direction,
    /// `previousFacing` (+0x30).
    pub previous_facing: Direction,
    /// `nextFacingBackup` (+0x34).
    pub next_facing_backup: Direction,
    /// `xRange` (+0x44); -1 = unbounded.
    pub x_range: i32,
    /// `yRange` (+0x48); -1 = unbounded.
    pub y_range: i32,
    /// `initialX/Y/Z` (+0x4C).
    pub initial: [i32; 3],
    /// `previousX/Y/Z` (+0x58): the tile the object is leaving.
    pub previous: [i32; 3],
    /// `currentX/Y/Z` (+0x64): the tile the object occupies or is
    /// entering — advanced at a move's first step.
    pub current: [i32; 3],
    /// `positionVector` (+0x70): the interpolated world position.
    pub position: VecFx32,
    /// `facingVector` (+0x7C).
    pub facing_vector: VecFx32,
    /// `unkA0`: the animation group (`sub_0205F328`).
    pub anim_group: u32,
    /// `movementCmd` (+0xA4).
    pub movement_cmd: MovementCmd,
    /// `movementStep` (+0xA8).
    pub movement_step: u32,
    /// `unkAC`: the behaviour of the current tile.
    pub current_behavior: u16,
    /// `unkAE`: the behaviour of the previous tile.
    pub previous_behavior: u16,
    /// `unk128`: the previous-tile behaviour latched at a move's end.
    pub end_behavior: u16,
    /// `unkF8`: the step functions' work buffer.
    pub scratch: [u8; 16],
}

impl MapObject {
    /// `MapObject_Create` for a plain object (`src/map_object.c:225`
    /// via `MapObject_CreateFromObjectEvent`, `MapObject_InitFromObjectEvent`,
    /// `MapObject_SetPositionVectorFromObjectEvent` and `sub_0205EC90`):
    /// at tile `(x, z)`, elevation 0, facing `direction`, with a fresh
    /// flag word (`ACTIVE | BEHAVIOR_STALE | HEIGHT_STALE | START_MOVEMENT`)
    /// and no held movement.
    #[must_use]
    pub fn create(x: i32, z: i32, direction: Direction, sprite_id: u32, movement: u32) -> Self {
        let mut object = Self {
            flags: 0,
            flags2: 0,
            id: 0,
            sprite_id,
            movement,
            initial_facing: direction,
            current_facing: direction,
            next_facing: direction,
            previous_facing: direction,
            next_facing_backup: direction,
            x_range: 0,
            y_range: 0,
            initial: [x, 0, z],
            previous: [x, 0, z],
            current: [x, 0, z],
            position: VecFx32::from_tile(x, 0, z),
            facing_vector: VecFx32::default(),
            anim_group: 0,
            movement_cmd: MovementCmd::None,
            movement_step: 0,
            current_behavior: 0,
            previous_behavior: 0,
            end_behavior: 0,
            scratch: [0; 16],
        };
        // sub_0205EC90: ACTIVE | UNK11 | UNK12, facing from the event,
        // then MapObject_ClearHeldMovement.
        object.flags |= flag::ACTIVE | flag::BEHAVIOR_STALE | flag::HEIGHT_STALE;
        object.set_facing_direction_direct(direction);
        object.set_next_facing_direction(direction);
        object.clear_held_movement();
        // MapObject_CreateFromObjectEvent: START_MOVEMENT after setup.
        object.flags |= flag::START_MOVEMENT;
        object
    }

    // ----- flag helpers (src/map_object.c:1006-1024) --------------------

    /// `MapObject_TestFlagsBits`: any of `bits` set.
    #[must_use]
    pub fn test_flags(&self, bits: u32) -> bool {
        self.flags & bits != 0
    }

    /// `MapObject_SetFlagsBits`.
    pub fn set_flags(&mut self, bits: u32) {
        self.flags |= bits;
    }

    /// `MapObject_ClearFlagsBits`.
    pub fn clear_flags(&mut self, bits: u32) {
        self.flags &= !bits;
    }

    // ----- facing (src/map_object.c:1099-1131) --------------------------

    /// `MapObject_SetFacingDirectionDirect`.
    pub fn set_facing_direction_direct(&mut self, direction: Direction) {
        self.current_facing = direction;
    }

    /// `MapObject_SetFacingDirection`: a no-op while `FACING_LOCKED`.
    pub fn set_facing_direction(&mut self, direction: Direction) {
        if !self.test_flags(flag::FACING_LOCKED) {
            self.previous_facing = self.current_facing;
            self.current_facing = direction;
        }
    }

    /// `MapObject_SetNextFacingDirection`.
    pub fn set_next_facing_direction(&mut self, direction: Direction) {
        self.next_facing_backup = self.next_facing;
        self.next_facing = direction;
    }

    /// `MapObject_SetOrQueueFacing`: both of the above.
    pub fn set_or_queue_facing(&mut self, direction: Direction) {
        self.set_facing_direction(direction);
        self.set_next_facing_direction(direction);
    }

    // ----- held movement (asm/unk_02062108.s:25-169) --------------------

    /// `MapObject_AreBitsSetForMovementScriptInit` (`:25`): active,
    /// not in single-movement mode, and no *unfinished* held movement.
    #[must_use]
    pub fn ready_for_movement(&self) -> bool {
        if !self.test_flags(flag::ACTIVE) || self.test_flags(flag::SINGLE_MOVEMENT) {
            return false;
        }
        !self.test_flags(flag::HELD_MOVEMENT) || self.test_flags(flag::HELD_MOVEMENT_DONE)
    }

    /// `MapObject_SetHeldMovement` (`:62`): load `command` at step 0,
    /// mark it held and unfinished.
    ///
    /// # Errors
    /// [`MovementError::NotHoldable`] for `End` / `None` (pret asserts
    /// `command < 0x71`).
    pub fn set_held_movement(&mut self, command: MovementCmd) -> Result<(), MovementError> {
        if command.as_u8() >= MOVEMENT_CMD_COUNT {
            return Err(MovementError::NotHoldable(command));
        }
        self.movement_cmd = command;
        self.movement_step = 0;
        self.set_flags(flag::HELD_MOVEMENT);
        self.clear_flags(flag::HELD_MOVEMENT_DONE);
        Ok(())
    }

    /// `MapObject_IsMovementPaused` (`:101`) — despite the name: "no
    /// held movement is in progress" (none loaded, or it finished).
    #[must_use]
    pub fn is_movement_idle(&self) -> bool {
        !self.test_flags(flag::HELD_MOVEMENT) || self.test_flags(flag::HELD_MOVEMENT_DONE)
    }

    /// `MapObject_ClearHeldMovementIfActive` (`:125`): returns `true`
    /// when there is nothing to clear or the movement had finished
    /// (then both held bits are cleared); `false` — nothing changed —
    /// while one is still running.
    pub fn clear_held_movement_if_idle(&mut self) -> bool {
        if !self.test_flags(flag::HELD_MOVEMENT) {
            return true;
        }
        if !self.test_flags(flag::HELD_MOVEMENT_DONE) {
            return false;
        }
        self.clear_flags(flag::HELD_MOVEMENT | flag::HELD_MOVEMENT_DONE);
        true
    }

    /// `MapObject_ClearHeldMovement` (`:152`): drop the held command
    /// (`MOVEMENT_NONE`, step 0) and mark the object finished.
    pub fn clear_held_movement(&mut self) {
        self.clear_flags(flag::HELD_MOVEMENT);
        self.set_flags(flag::HELD_MOVEMENT_DONE);
        self.movement_cmd = MovementCmd::None;
        self.movement_step = 0;
    }

    // ----- coordinates (asm/unk_0205FD20.s:2284-2340) -------------------

    /// `sub_02060F78`: previous := current.
    pub fn latch_previous(&mut self) {
        self.previous = self.current;
    }

    /// `sub_02060F24(direction)`: previous := current, then current
    /// advances one tile along `direction` (y unchanged).
    pub fn advance_tile(&mut self, direction: Direction) {
        self.latch_previous();
        self.current[0] = self.current[0].wrapping_add(direction.delta_x());
        self.current[2] = self.current[2].wrapping_add(direction.delta_z());
    }

    /// `sub_0206101C(direction, speed)`: move the position vector by
    /// `speed` along `direction`'s axis.
    pub fn advance_position(&mut self, direction: Direction, speed: i32) {
        match direction {
            Direction::North => self.position.z = self.position.z.wrapping_sub(speed),
            Direction::South => self.position.z = self.position.z.wrapping_add(speed),
            Direction::West => self.position.x = self.position.x.wrapping_sub(speed),
            Direction::East => self.position.x = self.position.x.wrapping_add(speed),
        }
    }

    /// `sub_02061070`: refresh the elevation from the BDHC height map.
    /// No height map is loaded here (BDHC is deferred), so this ports
    /// the flag protocol only: `IGNORE_HEIGHTS` clears `HEIGHT_STALE`;
    /// otherwise the lookup counts as failed and `HEIGHT_STALE` is set
    /// (`asm/unk_0205FD20.s:2445`). Returns pret's result (found).
    pub fn update_height(&mut self) -> bool {
        if self.test_flags(flag::IGNORE_HEIGHTS) {
            self.clear_flags(flag::HEIGHT_STALE);
        } else {
            self.set_flags(flag::HEIGHT_STALE);
        }
        false
    }

    /// `sub_02061108` (`asm/unk_0205FD20.s:2516`): latch the behaviours
    /// of the previous and current tiles (`unkAE`, `unkAC`). Objects
    /// with `flags2` bit 2 keep `TILE_BEHAVIOR_NONE` for both. Returns
    /// `true` (and clears `BEHAVIOR_STALE`) when the current tile has a
    /// behaviour; sets `BEHAVIOR_STALE` otherwise.
    pub fn refresh_behaviors(&mut self, terrain: &dyn Collision) -> bool {
        let (previous, current) = if self.flags2 & (1 << 2) != 0 {
            (BEHAVIOR_NONE, BEHAVIOR_NONE)
        } else {
            (
                terrain.behavior(self.previous[0], self.previous[2]),
                terrain.behavior(self.current[0], self.current[2]),
            )
        };
        self.previous_behavior = u16::from(previous);
        self.current_behavior = u16::from(current);
        if current == BEHAVIOR_NONE {
            self.set_flags(flag::BEHAVIOR_STALE);
            false
        } else {
            self.clear_flags(flag::BEHAVIOR_STALE);
            true
        }
    }

    // ----- the step functions (asm/unk_02062108.s) ----------------------

    /// The 12-byte linear/on-spot work struct in `scratch`:
    /// `{u16 animGroup; s16 frames; u32 direction; fx32 speed}`.
    fn work_frames(&self) -> i16 {
        i16::from_le_bytes([self.scratch[2], self.scratch[3]])
    }

    fn set_work_frames(&mut self, frames: i16) {
        self.scratch[2..4].copy_from_slice(&frames.to_le_bytes());
    }

    fn work_direction(&self) -> u32 {
        u32::from_le_bytes([
            self.scratch[4],
            self.scratch[5],
            self.scratch[6],
            self.scratch[7],
        ])
    }

    fn work_speed(&self) -> i32 {
        i32::from_le_bytes([
            self.scratch[8],
            self.scratch[9],
            self.scratch[10],
            self.scratch[11],
        ])
    }

    /// `sub_0205F3C0(object, 12)`: zero the first 12 scratch bytes.
    fn clear_work(&mut self) {
        self.scratch[..12].fill(0);
    }

    /// `sub_0206247C(direction)` (`:543`): the face step — face, anim
    /// group 0, previous := current, next step.
    fn step_face(&mut self, direction: Direction) {
        self.set_facing_direction(direction);
        self.anim_group = 0;
        self.latch_previous();
        self.movement_step += 1;
    }

    /// `sub_020624CC(direction, speed, frames, anim)` (`:595`): the
    /// linear-move init. Tile coordinates advance now; the position
    /// vector follows over `frames` ticks.
    fn step_linear_init(&mut self, direction: Direction, speed: i32, frames: i16, anim: u16) {
        self.clear_work();
        self.scratch[0..2].copy_from_slice(&anim.to_le_bytes());
        self.set_work_frames(frames);
        self.scratch[4..8].copy_from_slice(&direction.index().to_le_bytes());
        self.scratch[8..12].copy_from_slice(&speed.to_le_bytes());
        self.advance_tile(direction);
        self.set_or_queue_facing(direction);
        self.anim_group = u32::from(anim);
        self.set_flags(flag::START_MOVEMENT);
        self.movement_step += 1;
    }

    /// `MapObjectMovementCmd090_Step1` (`:629`): advance the position
    /// by the work speed, count a frame, finish at zero.
    fn step_linear_advance(&mut self) -> StepResult {
        if let Some(direction) = Direction::from_index(self.work_direction()) {
            let speed = self.work_speed();
            self.advance_position(direction, speed);
        }
        self.update_height();
        let frames = self.work_frames().wrapping_sub(1);
        self.set_work_frames(frames);
        if frames > 0 {
            return StepResult::Yield;
        }
        self.set_flags(flag::END_MOVEMENT | flag::HELD_MOVEMENT_DONE);
        self.latch_previous();
        // sub_0205F484: the sprite callback (presentation) — no state.
        self.anim_group = 0;
        self.movement_step += 1;
        StepResult::Continue
    }

    /// `sub_020627B0(direction, frames, anim)` (`:1044`): the
    /// walk-on-spot init. Note the stored count is `frames + 1`.
    fn step_on_spot_init(&mut self, direction: Direction, frames: i16, anim: u16) {
        self.clear_work();
        self.scratch[0..2].copy_from_slice(&anim.to_le_bytes());
        self.set_work_frames(frames.wrapping_add(1));
        self.set_facing_direction(direction);
        self.anim_group = u32::from(anim);
        self.latch_previous();
        self.movement_step += 1;
    }

    /// `MapObjectMovementCmd040_Step1` (`:1069`): count a frame,
    /// finish at zero — without `END_MOVEMENT`.
    fn step_on_spot_wait(&mut self) -> StepResult {
        let frames = self.work_frames().wrapping_sub(1);
        self.set_work_frames(frames);
        if frames > 0 {
            return StepResult::Yield;
        }
        self.set_flags(flag::HELD_MOVEMENT_DONE);
        self.anim_group = 0;
        self.movement_step += 1;
        StepResult::Continue
    }

    /// `MapObjectMovementCmd098_Step2` (`:534`): the terminal step —
    /// mark finished and yield. It never advances the step, so the
    /// runner re-executes it (harmlessly) every tick until a new
    /// command is loaded.
    fn step_finish(&mut self) -> StepResult {
        self.set_flags(flag::HELD_MOVEMENT_DONE);
        StepResult::Yield
    }

    /// `MapObject_RunMovementCommand(object, command, step)` (`:520`):
    /// dispatch one entry of `gMovementCmdSteps_<command>`.
    ///
    /// # Errors
    /// [`MovementError::Unimplemented`] for families not ported.
    pub fn run_step(&mut self, command: MovementCmd, step: u32) -> Result<StepResult, MovementError> {
        if let Some(direction) = command.direction().filter(|_| command.as_u8() < 4) {
            return Ok(match step {
                0 => {
                    self.step_face(direction);
                    StepResult::Continue
                }
                _ => self.step_finish(),
            });
        }
        if let Some((direction, speed, frames, anim)) = command.linear() {
            return Ok(match step {
                0 => {
                    self.step_linear_init(direction, speed, frames, anim);
                    StepResult::Continue
                }
                1 => self.step_linear_advance(),
                _ => self.step_finish(),
            });
        }
        if let Some((direction, frames, anim)) = command.on_spot() {
            return Ok(match step {
                0 => {
                    self.step_on_spot_init(direction, frames, anim);
                    StepResult::Continue
                }
                1 => self.step_on_spot_wait(),
                _ => self.step_finish(),
            });
        }
        Err(MovementError::Unimplemented(command))
    }

    /// `sub_02062400` (`:471`): run the held command's steps until one
    /// yields. An unimplemented command is reported and retired as if
    /// it had finished, so the object never wedges.
    pub fn run_held_movement(&mut self) -> Option<MovementCmd> {
        loop {
            let command = self.movement_cmd;
            if command == MovementCmd::None {
                return None;
            }
            match self.run_step(command, self.movement_step) {
                Ok(StepResult::Continue) => {}
                Ok(StepResult::Yield) => return None,
                Err(_) => {
                    self.step_finish();
                    return Some(command);
                }
            }
        }
    }

    /// `sub_0205FD30` (`asm/unk_0205FD20.s:28`), the object's per-frame
    /// SysTask body, less the autonomous-movement and effect hooks:
    ///
    /// 1. `sub_0205FE0C`: a stale height is re-fetched.
    /// 2. `sub_0205FE24`: a stale standing-tile behaviour is refreshed;
    ///    becoming valid raises `START_MOVEMENT`.
    /// 3. `sub_0205FE48`: a pending `START_MOVEMENT` refreshes the
    ///    behaviours (`sub_0205FEDC`) and is consumed.
    /// 4. The held movement runs (`sub_02062400`) if one is loaded.
    /// 5. `sub_0205FE6C`: `START_MOVEMENT` raised by the step refreshes
    ///    the behaviours (`sub_0205FF6C`) and is consumed.
    /// 6. `sub_0205FEA4`: `END_MOVEMENT` latches the previous-tile
    ///    behaviour (`sub_0206008C`), refreshes, and is consumed.
    ///
    /// The autonomous movement callbacks (`sub_0205F430`) are no-ops
    /// for the player's movement type 1 and are not ported for others.
    pub fn tick(&mut self, terrain: &dyn Collision) -> TickReport {
        let mut report = TickReport::default();
        if self.test_flags(flag::HEIGHT_STALE) {
            self.update_height();
        }
        if self.test_flags(flag::BEHAVIOR_STALE) && self.refresh_behaviors(terrain) {
            self.set_flags(flag::START_MOVEMENT);
        }
        if self.test_flags(flag::START_MOVEMENT) {
            self.refresh_behaviors(terrain);
            report.started = true;
        }
        self.clear_flags(flag::START_MOVEMENT | flag::UNK16);
        if self.test_flags(flag::HELD_MOVEMENT) {
            report.unimplemented = self.run_held_movement();
        }
        if self.test_flags(flag::UNK16) {
            self.refresh_behaviors(terrain);
        } else if self.test_flags(flag::START_MOVEMENT) {
            self.refresh_behaviors(terrain);
            report.started = true;
        }
        self.clear_flags(flag::START_MOVEMENT | flag::UNK16);
        if self.test_flags(flag::UNK17) {
            self.refresh_behaviors(terrain);
        } else if self.test_flags(flag::END_MOVEMENT) {
            self.end_behavior = self.previous_behavior;
            self.refresh_behaviors(terrain);
            report.ended = true;
        }
        self.clear_flags(flag::END_MOVEMENT | flag::UNK17);
        report
    }
}

/// One entry of a scripted movement list: `{u16 cmd; u16 count}`
/// (`MovementScriptMachineSub_LoopCheck`, `asm/unk_02062108.s:320`
/// reads the count at `+2` and steps the pointer by 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementEntry {
    /// The command to hold.
    pub cmd: MovementCmd,
    /// How many times to repeat it.
    pub count: u16,
}

/// The `MovementScriptMachine` states (`sMovementScriptMachineStateFuncs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListState {
    /// `MovementScriptMachineSub_Init`.
    Init,
    /// `MovementScriptMachineSub_WaitReady`.
    WaitReady,
    /// `MovementScriptMachineSub_SetMovementCommand`.
    SetCommand,
    /// `MovementScriptMachineSub_WaitMovementCommand`.
    WaitCommand,
    /// `MovementScriptMachineSub_LoopCheck`.
    LoopCheck,
    /// `MovementScriptMachineSub_Done`.
    Done,
}

/// `EventObjectMovementMan` (`asm/unk_02062108.s:170`): applies a
/// movement list to one object, one command at a time, exactly as the
/// SysTask does — it runs before the object's own tick each frame
/// (its task priority is the manager's minus one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementList<'a> {
    entries: &'a [MovementEntry],
    state: ListState,
    finished: bool,
    repeat: u32,
    index: usize,
}

impl<'a> MovementList<'a> {
    /// A machine over `entries`; the list ends at the first
    /// [`MovementCmd::End`] entry (or the slice's end).
    #[must_use]
    pub fn new(entries: &'a [MovementEntry]) -> Self {
        Self {
            entries,
            state: ListState::Init,
            finished: false,
            repeat: 0,
            index: 0,
        }
    }

    /// `EventObjectMovementMan_IsFinish`.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn current(&self) -> MovementCmd {
        self.entries
            .get(self.index)
            .map_or(MovementCmd::End, |entry| entry.cmd)
    }

    /// `MovementScriptMachine` (`:241`): run the state functions while
    /// they return 1. Commands the object cannot hold surface as the
    /// error and end the list.
    ///
    /// # Errors
    /// [`MovementError::NotHoldable`] if the list holds `End`/`None`
    /// where a command is expected.
    pub fn tick(&mut self, object: &mut MapObject) -> Result<(), MovementError> {
        loop {
            let again = match self.state {
                ListState::Init => {
                    self.repeat = 0;
                    self.state = ListState::WaitReady;
                    true
                }
                ListState::WaitReady => {
                    if !object.ready_for_movement() {
                        false
                    } else {
                        self.state = ListState::SetCommand;
                        true
                    }
                }
                ListState::SetCommand => {
                    let cmd = self.current();
                    if let Err(e) = object.set_held_movement(cmd) {
                        self.finished = true;
                        self.state = ListState::Done;
                        return Err(e);
                    }
                    self.state = ListState::WaitCommand;
                    false
                }
                ListState::WaitCommand => {
                    if !object.is_movement_idle() {
                        false
                    } else {
                        self.state = ListState::LoopCheck;
                        true
                    }
                }
                ListState::LoopCheck => {
                    self.repeat += 1;
                    let count = self
                        .entries
                        .get(self.index)
                        .map_or(0, |entry| u32::from(entry.count));
                    if self.repeat < count {
                        self.state = ListState::WaitReady;
                        true
                    } else {
                        self.index += 1;
                        if self.current() == MovementCmd::End {
                            self.finished = true;
                            self.state = ListState::Done;
                            false
                        } else {
                            self.state = ListState::Init;
                            true
                        }
                    }
                }
                ListState::Done => false,
            };
            if !again {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_match_the_delta_tables() {
        assert_eq!(Direction::North.delta_z(), -1);
        assert_eq!(Direction::South.delta_z(), 1);
        assert_eq!(Direction::West.delta_x(), -1);
        assert_eq!(Direction::East.delta_x(), 1);
        for dir in Direction::ALL {
            assert_eq!(dir.reverse().reverse(), dir);
            assert_eq!(Direction::from_index(dir.index()), Some(dir));
        }
        assert_eq!(Direction::from_index(4), None);
    }

    #[test]
    fn tile_centres_are_fx32() {
        let v = VecFx32::from_tile(6, 0, 6);
        assert_eq!(v, VecFx32 { x: 0x68000, y: 0, z: 0x68000 });
        assert_eq!(VecFx32::from_tile(0, 1, 0).y, 0x8000);
    }

    #[test]
    fn commands_round_trip_and_the_table_is_dense() {
        for value in 0..MOVEMENT_CMD_COUNT {
            let cmd = MovementCmd::from_u8(value).expect("dense");
            assert_eq!(cmd.as_u8(), value);
        }
        assert_eq!(MovementCmd::from_u8(113), None);
        assert_eq!(MovementCmd::from_u8(254), Some(MovementCmd::End));
        assert_eq!(MovementCmd::from_u8(255), Some(MovementCmd::None));
        assert_eq!(MovementCmd::WalkNormalNorth.as_u8(), 12);
        assert_eq!(MovementCmd::RunEast.as_u8(), 91);
        assert_eq!(MovementCmd::EmoteExclamation.as_u8(), 75);
    }

    #[test]
    fn family_lookup_mirrors_sub_0206234c() {
        assert_eq!(
            MovementCmd::for_direction(family::WALK_NORMAL, Direction::East),
            Some(MovementCmd::WalkNormalEast)
        );
        assert_eq!(
            MovementCmd::for_direction(family::RUN, Direction::South),
            Some(MovementCmd::RunSouth)
        );
        // A row is found by any member, as the asm scans all four.
        assert_eq!(
            MovementCmd::for_direction(0x0D, Direction::West),
            Some(MovementCmd::WalkNormalWest)
        );
        // 0x40-0x4B have no row.
        assert_eq!(MovementCmd::for_direction(0x40, Direction::North), None);
        // The pair rows repeat.
        assert_eq!(MovementCmd::for_direction(0x5C, Direction::West).map(MovementCmd::as_u8), Some(0x5C));
        assert_eq!(MovementCmd::Cmd093.direction(), Some(Direction::South));
        assert_eq!(MovementCmd::WalkOnSpotFastestWest.direction(), Some(Direction::West));
        assert_eq!(MovementCmd::EmoteExclamation.direction(), None);
        assert!(MovementCmd::WalkOnSpotSlowerNorth.is_bump());
        assert!(!MovementCmd::WalkOnSpotFastestNorth.is_bump());
        assert!(MovementCmd::RunWest.is_run());
    }

    #[test]
    fn held_movement_flags_follow_pret() {
        let mut obj = MapObject::create(1, 2, Direction::South, 0, 1);
        assert!(obj.ready_for_movement());
        assert!(obj.is_movement_idle());
        assert_eq!(obj.movement_cmd, MovementCmd::None);
        obj.set_held_movement(MovementCmd::WalkNormalNorth).unwrap();
        assert!(!obj.ready_for_movement());
        assert!(!obj.is_movement_idle());
        assert!(!obj.clear_held_movement_if_idle());
        obj.set_flags(flag::HELD_MOVEMENT_DONE);
        assert!(obj.ready_for_movement());
        assert!(obj.clear_held_movement_if_idle());
        assert!(!obj.test_flags(flag::HELD_MOVEMENT));
        assert_eq!(
            obj.set_held_movement(MovementCmd::End),
            Err(MovementError::NotHoldable(MovementCmd::End))
        );
    }

    #[test]
    fn a_normal_step_takes_eight_ticks_at_0x2000() {
        let mut obj = MapObject::create(6, 6, Direction::South, 0, 1);
        obj.set_flags(flag::IGNORE_HEIGHTS);
        let terrain = NoTerrain;
        obj.tick(&terrain); // consume the creation START flag
        obj.set_held_movement(MovementCmd::WalkNormalNorth).unwrap();
        let start = obj.position;
        for frame in 1..=8 {
            let report = obj.tick(&terrain);
            assert_eq!(obj.position.z, start.z - 0x2000 * frame, "frame {frame}");
            assert_eq!(report.started, frame == 1);
            assert_eq!(report.ended, frame == 8);
            assert_eq!(obj.current, [6, 0, 5]);
            assert_eq!(obj.is_movement_idle(), frame == 8);
        }
        assert_eq!(obj.position, VecFx32::from_tile(6, 0, 5));
        assert_eq!(obj.previous, [6, 0, 5]);
        assert_eq!(obj.movement_step, 2);
        // The terminal step re-runs harmlessly.
        let report = obj.tick(&terrain);
        assert_eq!(report, TickReport::default());
        assert_eq!(obj.movement_step, 2);
    }

    #[test]
    fn walk_on_spot_stores_frames_plus_one() {
        let mut obj = MapObject::create(0, 0, Direction::North, 0, 1);
        obj.set_flags(flag::IGNORE_HEIGHTS);
        obj.tick(&NoTerrain);
        obj.set_held_movement(MovementCmd::WalkOnSpotFastestEast).unwrap();
        // Tick 0: Step0 (3 stored) + Step1 (2); ticks 1, 2 count down.
        obj.tick(&NoTerrain);
        assert_eq!(obj.current_facing, Direction::East);
        assert_eq!(obj.work_frames(), 2);
        assert!(!obj.is_movement_idle());
        obj.tick(&NoTerrain);
        assert!(!obj.is_movement_idle());
        let report = obj.tick(&NoTerrain);
        assert!(obj.is_movement_idle());
        assert!(!report.ended, "on-spot never raises END_MOVEMENT");
        assert_eq!(obj.position, VecFx32::from_tile(0, 0, 0));
    }

    #[test]
    fn face_commands_finish_within_the_tick() {
        let mut obj = MapObject::create(0, 0, Direction::North, 0, 1);
        obj.set_flags(flag::IGNORE_HEIGHTS);
        obj.tick(&NoTerrain);
        obj.set_held_movement(MovementCmd::FaceWest).unwrap();
        obj.tick(&NoTerrain);
        assert!(obj.is_movement_idle());
        assert_eq!(obj.current_facing, Direction::West);
        assert_eq!(obj.movement_step, 1);
    }

    #[test]
    fn unimplemented_commands_are_reported_and_retired() {
        let mut obj = MapObject::create(0, 0, Direction::North, 0, 1);
        obj.set_flags(flag::IGNORE_HEIGHTS);
        obj.tick(&NoTerrain);
        obj.set_held_movement(MovementCmd::EmoteExclamation).unwrap();
        let report = obj.tick(&NoTerrain);
        assert_eq!(report.unimplemented, Some(MovementCmd::EmoteExclamation));
        assert!(obj.is_movement_idle());
    }

    #[test]
    fn movement_lists_run_like_the_script_machine() {
        let list = [
            MovementEntry {
                cmd: MovementCmd::WalkNormalSouth,
                count: 2,
            },
            MovementEntry {
                cmd: MovementCmd::FaceEast,
                count: 1,
            },
            MovementEntry {
                cmd: MovementCmd::End,
                count: 0,
            },
        ];
        let mut obj = MapObject::create(3, 3, Direction::North, 0, 1);
        obj.set_flags(flag::IGNORE_HEIGHTS);
        obj.tick(&NoTerrain);
        let mut machine = MovementList::new(&list);
        let mut frames = 0;
        while !machine.is_finished() {
            machine.tick(&mut obj).unwrap();
            obj.tick(&NoTerrain);
            frames += 1;
            assert!(frames < 64, "list never finished");
        }
        // Two 8-frame steps: frames 0-7 and 8-15 (the machine notices
        // completion on the tick after the last advance, and loads the
        // next command in that same tick), the face command on frame
        // 16, and the END check on frame 17.
        assert_eq!(frames, 18);
        assert_eq!(obj.current, [3, 0, 5]);
        assert_eq!(obj.position, VecFx32::from_tile(3, 0, 5));
        assert_eq!(obj.current_facing, Direction::East);
    }
}
