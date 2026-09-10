//! In-memory new-game region. Ports the 42 save-array initializers and
//! overlay_36's pre-/post-Oak mutations. Card bytes are only made by
//! `snapshot`; beginning a new game does not overwrite an existing card.
use super::{BLOCK_RAW_SIZES, BLOCKS, DYNAMIC_REGION_SIZE, SaveData};
use crate::{
    assets::{AssetStore, AssetsError},
    formats::MsgBank,
    rng::{Lcrng, Mt19937, prandom},
    rtc::RtcDateTime,
    text::string::GameString,
};

/// Frozen console properties used by system-info and offline DWC setup.
/// These are explicit inputs, separate from both gameplay RNG streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsoleProfile {
    /// Owner RTC offset, in seconds.
    pub rtc_offset: i64,
    /// Console MAC address.
    pub mac: [u8; 6],
    /// Owner birthday month/day.
    pub birthday: [u8; 2],
    /// The console's 43-bit authentication ID.
    pub auth_id: u64,
    /// XOR of OS_GetLowEntropyData's eight words.
    pub entropy: u32,
}

/// Structured new-game state backed by the original save-block layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGameData {
    blocks: Vec<Vec<u8>>,
    avatar_table: Vec<u8>,
    safari_table: Vec<u8>,
    friend_names: [Vec<u16>; 2],
    marill_icon_palette: u8,
    /// Whether the post-Oak initializer has run (it may run only once).
    pub oak_complete: bool,
}
fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn read32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn encrypt_zero(b: &mut [u8], seed: u32) {
    let mut r = Lcrng::new(seed);
    for w in b.chunks_exact_mut(2) {
        w.copy_from_slice(&r.next_u16().to_le_bytes());
    }
}
fn zero_mon(b: &mut [u8]) {
    encrypt_zero(&mut b[8..136], 0);
    if b.len() > 136 {
        encrypt_zero(&mut b[136..236], 0);
    }
}
fn mail_message(b: &mut [u8], bank: u16, no: u16, words: [u16; 2]) {
    for (i, v) in [bank, no, words[0], words[1]].into_iter().enumerate() {
        put16(b, i * 2, v);
    }
}
fn mail_init(b: &mut [u8]) {
    b[5] = 2;
    b[6] = 7;
    b[7] = 255;
    b[8..30].fill(255);
    for p in [32, 40, 48] {
        mail_message(&mut b[p..p + 8], 65535, 0, [65535; 2]);
    }
}
fn flat_name(b: &mut [u8], units: &[u16]) {
    for (slot, v) in b
        .chunks_exact_mut(2)
        .zip(units.iter().copied().chain(std::iter::once(65535)))
    {
        slot.copy_from_slice(&v.to_le_bytes());
    }
}
impl NewGameData {
    /// Runs all array defaults, then `NewGame_InitSaveData`'s room, money,
    /// fishing record and flag. The two roamer seeds consume MT draws.
    pub fn initialize(
        store: &AssetStore,
        rtc: RtcDateTime,
        vblank: u32,
        mt: &mut Mt19937,
        console: ConsoleProfile,
    ) -> Result<Self, AssetsError> {
        let arm = store.arm9_image()?;
        let messages = |bank, id| -> Result<Vec<u16>, AssetsError> {
            let bytes = store.member("a/0/2/7", bank)?;
            let msg = MsgBank::parse(&bytes).map_err(|source| AssetsError::Corrupt {
                what: format!("new-game message bank {bank}"),
                source,
            })?;
            Ok(msg
                .message(id)
                .ok_or_else(|| AssetsError::Missing(format!("new-game message {bank}:{id}")))?
                .to_vec())
        };
        let mut blocks: Vec<Vec<u8>> = BLOCK_RAW_SIZES
            .iter()
            .map(|&n| vec![0; n as usize])
            .collect();
        // Save_PlayerData_Init, LocalFieldData_Init, Pokedex_Init.
        blocks[1][0] = 1;
        blocks[1][0x1d] = 2;
        blocks[1][0x20] = 7;
        put32(&mut blocks[2], 0, 6);
        for i in 0..6 {
            zero_mon(&mut blocks[2][8 + i * 236..8 + (i + 1) * 236]);
        }
        put16(&mut blocks[5], 0x68, 1);
        put32(&mut blocks[6], 0, 0xbeefcafe);
        blocks[6][0x43] = 255;
        blocks[6][0x83] = 255;
        blocks[6][0x108..0x144].fill(255);
        blocks[6][0x338..0x33f].fill(255);
        for i in 0..2 {
            zero_mon(&mut blocks[7][i * 236..i * 236 + 136]);
        }
        for i in 0..16 {
            put16(&mut blocks[8], i * 136, 65535);
        }
        // Save_Misc_Init's original byte-count quirk fills four rival
        // name characters (8 bytes), not eight characters.
        blocks[9][0x270..0x278].fill(255);
        blocks[9][0x280..0x288].fill(255);
        mail_message(&mut blocks[9][0x2a0..0x2a8], 4, 0, [0x501, 65535]);
        blocks[9][0x2a8..0x2d0].fill(255);
        // Fashion data has twelve 0x74 and four 0x98 record headers.
        for i in 0..12 {
            put16(&mut blocks[12], i * 0x74, 0x1234);
        }
        for i in 0..4 {
            put16(&mut blocks[12], 0x594 + i * 0x98, 0x1234);
        }
        blocks[12][0x81c..0x82e].fill(0x12);
        for mail in blocks[13].chunks_exact_mut(56) {
            mail_init(mail);
        }
        for i in 0..6 {
            put16(&mut blocks[14], i * 44, 65535);
            put16(&mut blocks[14], i * 44 + 16, 65535);
        }
        for i in 0..8 {
            put32(&mut blocks[15], i * 4, 140);
        }
        let salt = (vblank | (vblank << 8)) as u16;
        put16(&mut blocks[16], 0x1be, salt);
        encrypt_zero(&mut blocks[16][8..0x1bc], (salt as u32) << 16);
        blocks[19][0x957] = 1;
        // sub_0202D254's four MailMessageTemplate expansions.
        for (i, (bank, no, word)) in [(0, 0, 0x459), (1, 0, 0x473), (2, 0, 0x57b), (1, 4, 0x453)]
            .into_iter()
            .enumerate()
        {
            mail_message(
                &mut blocks[19][0xabc + i * 8..0xac4 + i * 8],
                bank,
                no,
                [word, 65535],
            );
        }
        for i in 0..2 {
            put32(&mut blocks[21], i * 4, mt.next_u32());
        }
        for record in blocks[24].chunks_exact_mut(24) {
            record[8..24].fill(255);
        }
        for i in 0..32 {
            put16(&mut blocks[25], 0x1c0 + i * 56, 65535);
            put16(&mut blocks[25], 0x1d0 + i * 56, 65535);
            blocks[25][0x1ee + i * 56] = 2;
        }
        init_dwc(&mut blocks[25], console);
        for i in 0..6 {
            zero_mon(&mut blocks[28][i * 236..(i + 1) * 236]);
        }
        blocks[30][0] = 2;
        blocks[31][0x34] = 1;
        put32(&mut blocks[32], 0, u32::MAX);
        blocks[34][0] = 3;
        blocks[34][2] = 128;
        blocks[34][3] = 128;
        for p in (0x4b8..=0x5e0).step_by(4) {
            blocks[34][p] = 4;
        }
        blocks[34][0x5fe] = 7;
        blocks[34][0x60d..0x657].fill(255);
        blocks[35][0x5e2] = 2;
        blocks[35][0x5e3] = 7;
        blocks[35][0x5e8..0x5f8].fill(255);
        for i in 0..36 {
            blocks[36][0x0c + i * 132..0x34 + i * 132].fill(255);
        }
        for i in 0..11 {
            for j in 0..5 {
                put16(&mut blocks[37], 0x2cc + i * 44 + j * 8, 65535);
            }
        }
        for i in 0..9 {
            for j in 0..5 {
                put16(&mut blocks[37], 0x528 + i * 164 + j * 8, 65535);
            }
        }
        for i in 0..3 {
            blocks[38][0x28 + i * 32..0x38 + i * 32].fill(255);
        }
        put32(&mut blocks[39], 0x130, 3);
        for i in 0..10 {
            blocks[40][i * 384 + 8..i * 384 + 24].fill(255);
            for j in 0..6 {
                blocks[40][i * 384 + 0x54 + j * 56..i * 384 + 0x68 + j * 56].fill(255);
            }
        }
        for i in 0..18 {
            for j in 0..30 {
                zero_mon(&mut blocks[41][i * 4096 + j * 136..i * 4096 + (j + 1) * 136]);
            }
            blocks[41][0x122d8 + i] = (i % 16) as u8;
            flat_name(
                &mut blocks[41][0x12008 + i * 40..0x12008 + (i + 1) * 40],
                &messages(24, i + 6)?,
            );
        }
        let mut result = Self {
            blocks,
            avatar_table: arm[0xfca44..0xfca74].to_vec(),
            safari_table: arm[0xf6888..0xf68c4].to_vec(),
            friend_names: [messages(445, 0)?, messages(445, 1)?],
            marill_icon_palette: arm[0xffc10 + 183],
            oak_complete: false,
        };
        result.init_clock(rtc);
        put32(&mut result.blocks[1], 0x18, 3000);
        for (p, v) in [64, u32::MAX, 6, 6, 1].into_iter().enumerate() {
            put32(&mut result.blocks[5], p * 4, v);
        }
        put16(&mut result.blocks[4], 0x35 * 2, 56150);
        result.blocks[4][0x2e0 + 0x960 / 8] |= 1;
        Ok(result)
    }

    /// The original bytes of one initialized block (before its outer CRC).
    pub fn block(&self, id: usize) -> Option<&[u8]> {
        self.blocks.get(id).map(Vec::as_slice)
    }
    /// Full trainer ID, including the hidden upper half.
    pub fn trainer_id(&self) -> u32 {
        read32(&self.blocks[1], 0x14)
    }
    /// Starting money, in Pokédollars.
    pub fn money(&self) -> u32 {
        read32(&self.blocks[1], 0x18)
    }
    /// Original Location fields: map, warp, x, y, direction.
    pub fn position(&self) -> [u32; 5] {
        std::array::from_fn(|i| read32(&self.blocks[5], i * 4))
    }
    /// Runs overlay_36's post-Oak pass once, preserving its draw order.
    pub fn finish_oak(
        &mut self,
        name: &GameString,
        gender: u8,
        rtc: RtcDateTime,
        mt: &mut Mt19937,
        lc: &mut Lcrng,
        console: ConsoleProfile,
    ) {
        assert!(
            !self.oak_complete,
            "post-Oak initialization cannot run twice"
        );
        assert!(gender <= 1);
        flat_name(&mut self.blocks[1][4..20], name.units());
        self.blocks[1][0x1c] = gender;
        self.blocks[0][..8].copy_from_slice(&console.rtc_offset.to_le_bytes());
        self.blocks[0][8..14].copy_from_slice(&console.mac);
        self.blocks[0][14..16].copy_from_slice(&console.birthday);
        self.init_clock(rtc);
        let group = mt.next_u32();
        let group_rand = prandom(group);
        put32(&mut self.blocks[14], 44 + 36, group);
        put32(&mut self.blocks[14], 44 + 40, group_rand);
        put32(
            &mut self.blocks[19],
            0x958,
            group_rand.wrapping_mul(0x5d588b65).wrapping_add(1),
        );
        let id = mt.next_u32();
        put32(&mut self.blocks[1], 0x14, id);
        self.blocks[1][0x1f] = self.avatar_table[((id as usize % 8) + gender as usize * 8) * 3];
        for i in 0..6 {
            self.blocks[35][i * 122] = self.safari_table[(id as usize % 10) * 6 + i];
        }
        for i in 0..128 {
            self.blocks[9][i * 4] = 1;
            self.blocks[9][i * 4 + 1] = 1;
        }
        for i in 0..10 {
            put32(&mut self.blocks[39], 0xfc + i * 4, mt.next_u32());
        }
        // CreateMon(MARILL,1,fixedIV=0) uses two LCRandom calls for
        // personality. The temporary mon is freed after copying its icon.
        lc.next_u16();
        lc.next_u16();
        let mail = &mut self.blocks[13][..56];
        mail[4] = 1 - gender;
        mail[7] = 9;
        flat_name(
            &mut mail[8..24],
            &self.friend_names[usize::from(1 - gender)],
        );
        put16(mail, 24, 183 | ((self.marill_icon_palette as u16) << 12));
        mail_message(&mut mail[32..40], 2, 4, [1371, 1417]);
        mail_message(&mut mail[40..48], 3, 1, [1479, 65535]);
        self.oak_complete = true;
    }
    fn init_clock(&mut self, rtc: RtcDateTime) {
        let b = &mut self.blocks[0];
        put32(b, 16, 1);
        let fields = [
            rtc.year % 100,
            rtc.month,
            rtc.day,
            rtc.week,
            rtc.hour,
            rtc.minute,
            rtc.second,
        ];
        for (i, v) in fields.into_iter().enumerate() {
            put32(b, 20 + i * 4, v);
        }
        let year = 2000 + rtc.year % 100;
        let mut days = 0u32;
        for y in 2000..year {
            days += if y % 4 == 0 { 366 } else { 365 };
        }
        for m in 1..rtc.month {
            days += match m {
                4 | 6 | 9 | 11 => 30,
                2 => {
                    if year % 4 == 0 {
                        29
                    } else {
                        28
                    }
                }
                _ => 31,
            };
        }
        days += rtc.day.saturating_sub(1);
        put32(b, 48, days);
        let seconds = days as u64 * 86400
            + rtc.hour as u64 * 3600
            + rtc.minute as u64 * 60
            + rtc.second as u64;
        b[52..60].copy_from_slice(&seconds.to_le_bytes());
        b[60..72].fill(0);
        for (i, v) in fields.into_iter().enumerate() {
            put32(&mut self.blocks[9], 0x230 + i * 4, v);
        }
    }
    /// Produces a checksummed card snapshot in memory. No disk write occurs.
    pub fn snapshot(&self) -> SaveData {
        let mut region = vec![0; DYNAMIC_REGION_SIZE];
        for (id, data) in self.blocks.iter().enumerate() {
            let p = BLOCKS[id].offset as usize;
            region[p..p + data.len()].copy_from_slice(data);
        }
        SaveData::from_new_region(&region)
    }
}
fn init_dwc(b: &mut [u8], console: ConsoleProfile) {
    put32(b, 0, 64);
    put32(b, 4, ((console.auth_id >> 32) as u32 & 0x7ff) | (1 << 11));
    put32(b, 8, console.auth_id as u32);
    let random = (console.entropy as u64)
        .wrapping_mul(0x5d588b656c078965)
        .wrapping_add(0x269ec3);
    put32(b, 12, (random >> 32) as u32);
    put32(b, 0x24, 0x4144414a);
    let mut crc = u32::MAX;
    for &v in &b[..60] {
        crc ^= v as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    put32(b, 60, !crc);
}
