//! Field area lighting — the time-of-day light templates that give the
//! overworld its day/night look. HGSS tints no 2D palette for this: the
//! four hardware lights and the global material colors the 3D map is
//! drawn with are swapped by wall-clock time (`docs/day-night.md`).
//!
//! pret has no C for it; the asm is the spec.
//! `asm/overlay_01_021E90C0.s:2207` `AreaLightManager_New`, `:2278`
//! `AreaLightManager_UpdateActiveTemplate`, `:2384` `ov01_021EA3E0`
//! (the text loader), `:2640` `ov01_021EA578` (one light line),
//! `:2758` `ov01_021EA668` (one color line), `:2331` `ov01_021EA300`
//! (apply a template), `:2810` `LoadAreaOrDungeonLightTxt` (the
//! one-shot variant); the archive table `ov01_02206450` (`:2875`) over
//! the five path strings at `:2885–2898`. The setters a template writes
//! through are `asm/model_attributes.s:268–340` over pret's
//! `ModelAttributes` (`include/field/model_attributes.h`). Which
//! archive a map uses: `asm/overlay_01_021FB878.s:249`
//! `AreaDataManager_GetAreaLightArchiveID` over the area-data record's
//! light-type byte, plus the story-flag override at
//! `src/field/fieldmap.c:758`. Per-frame, `src/field/fieldmap.c:421`
//! runs the update.
//!
//! The archives are CR-delimited ASCII tables, not binary: the loader
//! walks them with `Ascii_GetDelim`/`Ascii_StrToL`
//! (`src/ascii_util.c`), ten lines per record — a threshold line, four
//! light lines (`enable,r,g,b,x,y,z`), four material-color lines
//! (`r,g,b`) and a blank separator — until a line starting `EOF`. Each
//! record becomes a 0x30-byte struct: `u32 until` at 0, the
//! light-enable mask byte at 4, `u16 color[4]` at 6, `s16 vector[4][3]`
//! at 0xE, then `u16` diffuse/ambient/specular/emission at
//! 0x26/0x28/0x2A/0x2C. Thresholds are in **half-seconds of the day**:
//! both selection paths compare them against `GF_RTC_TimeToSec() / 2`,
//! so a day is 43200 and the last record ends there. Values step, they
//! never interpolate. `data/arealight.narc` (four members in the same
//! text format with different numbers) is an unreferenced leftover: no
//! code names it, the five `.txt` paths are the whole table.
use crate::{
    assets::{AssetStore, AssetsError},
    nds::NdsError,
};

/// The five archives `ov01_02206450` can load, in archive-ID order
/// (`asm/overlay_01_021E90C0.s:2875–2898`).
pub const ARCHIVE_PATHS: [&str; 5] = [
    "data/area00light.txt",
    "data/area01light.txt",
    "data/area02light.txt",
    "data/dun20_01light.txt",
    "data/dun20_02light.txt",
];

/// `GX_LIGHTS_COUNT` — hardware lights per template.
pub const LIGHT_COUNT: usize = 4;

/// Lines per record in the text: threshold, four lights, four material
/// colors, one separator.
pub const RECORD_LINES: usize = 10;

/// The record size the loader allocates per entry (`0x30`).
pub const RECORD_BYTES: usize = 0x30;

/// A day in the threshold unit: `GF_RTC_TimeToSec() / 2` peaks at
/// 43199, so a record ending at 43200 lasts until midnight.
pub const HALF_SECONDS_PER_DAY: u32 = 43_200;

/// `Ascii_GetDelim`'s copy limit (`src/ascii_util.c:14`): a line
/// without its delimiter inside 256 bytes yields `NULL` in the game.
pub const MAX_LINE: usize = 256;

/// The `x`/`y`/`z` clamp of a light vector, `±1.0` in fx16.
pub const VECTOR_LIMIT: i16 = 4096;

/// One of the four hardware lights in a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Light {
    /// Whether the record's enable field was `1` — the bit the loader
    /// sets in the record's mask byte (`ov01_021EA3E0`, `strb` at
    /// offset 4).
    pub enabled: bool,
    /// `GXRgb`: `r | g << 5 | b << 10` of the record's three color
    /// fields, `0` when disabled.
    pub color: u16,
    /// `VecFx16`: the record's three vector fields, each truncated to
    /// 16 bits then clamped to `±4096` (`ov01_021EA578`).
    pub vector: [i16; 3],
}

/// One record of an archive: the lighting in force until `until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightTemplate {
    /// End of this record's window, in half-seconds of the day
    /// (compared against `GF_RTC_TimeToSec() / 2`).
    pub until: u32,
    /// The four hardware lights (`NNS_G3dGlbLightVector/Color`).
    pub lights: [Light; LIGHT_COUNT],
    /// `ModelAttributes::diffuse`.
    pub diffuse: u16,
    /// `ModelAttributes::ambient`.
    pub ambient: u16,
    /// `ModelAttributes::specular`.
    pub specular: u16,
    /// `ModelAttributes::emission`.
    pub emission: u16,
}

/// The lighting subset of pret's `ModelAttributes`
/// (`include/field/model_attributes.h`) — every field a template
/// writes, in the struct's own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModelLighting {
    /// `lightVectors[4]` (`VecFx16`, offset 0).
    pub light_vectors: [[i16; 3]; LIGHT_COUNT],
    /// `lightColors[4]` (`GXRgb`, offset 0x18).
    pub light_colors: [u16; LIGHT_COUNT],
    /// `diffuse` (offset 0x20).
    pub diffuse: u16,
    /// `ambient` (offset 0x22).
    pub ambient: u16,
    /// `specular` (offset 0x24).
    pub specular: u16,
    /// `emission` (offset 0x26).
    pub emission: u16,
    /// `setDiffuseColorAsVertexColor` (offset 0x28) — a template
    /// always writes `FALSE`.
    pub diffuse_is_vertex_color: bool,
    /// `enableSpecularReflectShininessTable` (offset 0x2C) — a template
    /// always writes `FALSE`.
    pub specular_shininess: bool,
}

impl LightTemplate {
    /// The record's light-enable mask byte: bit `i` for light `i`.
    #[must_use]
    pub fn light_mask(&self) -> u8 {
        self.lights
            .iter()
            .enumerate()
            .filter(|(_, light)| light.enabled)
            .fold(0, |mask, (i, _)| mask | (1 << i))
    }

    /// `ov01_021EA300` — writes the template into the model
    /// attributes: an enabled light's vector and color, a disabled
    /// light's zeros; then diffuse (vertex-color flag off, no global
    /// apply), ambient (applied), specular (shininess off, no apply),
    /// emission (applied).
    pub fn apply(&self, attrs: &mut ModelLighting) {
        for (i, light) in self.lights.iter().enumerate() {
            if light.enabled {
                attrs.light_vectors[i] = light.vector;
                attrs.light_colors[i] = light.color;
            } else {
                attrs.light_vectors[i] = [0; 3];
                attrs.light_colors[i] = 0;
            }
        }
        attrs.diffuse = self.diffuse;
        attrs.diffuse_is_vertex_color = false;
        attrs.ambient = self.ambient;
        attrs.specular = self.specular;
        attrs.specular_shininess = false;
        attrs.emission = self.emission;
    }
}

/// `AreaDataManager_GetAreaLightArchiveID`
/// (`asm/overlay_01_021FB878.s:249`) plus the `fieldmap.c:758`
/// override: the area-data record's light-type byte picks archive
/// 1 for type 0, 0 for type 1, 3 for type 2 (the dungeon set, which
/// `CheckFlag96A` promotes to 4), and 0 for anything else. Archive 2
/// is reachable only through `LoadAreaOrDungeonLightTxt`'s callers.
#[must_use]
pub fn archive_for_light_type(light_type: u8, dungeon_variant: bool) -> usize {
    let id = match light_type {
        0 => 1,
        1 => 0,
        2 => 3,
        _ => 0,
    };
    if id == 3 && dungeon_variant { 4 } else { id }
}

/// A parsed light archive: the records in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaLightArchive {
    templates: Vec<LightTemplate>,
}

impl AreaLightArchive {
    /// Parses one archive text exactly as `ov01_021EA3E0` does: a
    /// counting pass to the `EOF` line, then one record per ten
    /// lines.
    ///
    /// # Errors
    /// Returns [`NdsError::Invalid`] when a line exceeds the 256-byte
    /// buffer, the text ends before an `EOF` line, no record precedes
    /// it, or a threshold is negative (the game would compare the bit
    /// pattern two different ways).
    pub fn parse(text: &[u8]) -> Result<Self, NdsError> {
        let count = count_records(text)?;
        if count == 0 {
            return Err(NdsError::Invalid {
                what: "area light table has no records before EOF",
            });
        }
        let mut lines = Lines::new(text);
        let mut templates = Vec::with_capacity(count);
        for _ in 0..count {
            let until = ascii_to_long(Fields::new(lines.next()?).next());
            let until = u32::try_from(until).map_err(|_| NdsError::Invalid {
                what: "area light record has a negative threshold",
            })?;
            let mut lights = [Light::default(); LIGHT_COUNT];
            for light in &mut lights {
                *light = parse_light(lines.next()?);
            }
            let diffuse = parse_color(lines.next()?);
            let ambient = parse_color(lines.next()?);
            let specular = parse_color(lines.next()?);
            let emission = parse_color(lines.next()?);
            lines.next()?; // the separator
            templates.push(LightTemplate {
                until,
                lights,
                diffuse,
                ambient,
                specular,
                emission,
            });
        }
        Ok(Self { templates })
    }

    /// Loads archive `id` (an index into [`ARCHIVE_PATHS`]) from the
    /// cart.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] for an unknown id, a missing file,
    /// or a text that does not parse.
    pub fn load(store: &AssetStore, id: usize) -> Result<Self, AssetsError> {
        let path = ARCHIVE_PATHS
            .get(id)
            .ok_or_else(|| AssetsError::Missing(format!("area light archive {id}")))?;
        let text = store.nitrofs_file(path)?;
        Self::parse(&text).map_err(|source| AssetsError::Corrupt {
            what: (*path).to_owned(),
            source,
        })
    }

    /// The records in file order.
    #[must_use]
    pub fn templates(&self) -> &[LightTemplate] {
        &self.templates
    }

    /// The record `AreaLightManager_New` (and
    /// `LoadAreaOrDungeonLightTxt`) starts on at `seconds_of_day`
    /// (`GF_RTC_TimeToSec()`): the first whose `until` exceeds
    /// `seconds_of_day / 2` (an unsigned compare), or 0 when none does.
    /// So record `i` covers `[until[i-1], until[i])` half-seconds, and a
    /// leading `until == 0` record is never the initial pick.
    #[must_use]
    pub fn initial_index(&self, seconds_of_day: u32) -> usize {
        let half = seconds_of_day / 2;
        self.templates
            .iter()
            .position(|t| t.until > half)
            .unwrap_or(0)
    }

    /// The template in force at `seconds_of_day` — [`Self::initial_index`]
    /// resolved.
    #[must_use]
    pub fn template_at(&self, seconds_of_day: u32) -> &LightTemplate {
        &self.templates[self.initial_index(seconds_of_day)]
    }

    /// The half-second window `[start, end)` record `index` claims —
    /// the previous record's `until` (0 for the first) to its own.
    #[must_use]
    pub fn window(&self, index: usize) -> (u32, u32) {
        let start = if index == 0 {
            0
        } else {
            self.templates[index - 1].until
        };
        (start, self.templates[index].until)
    }
}

/// pret's `AreaLightManager` (a 0x14-byte struct: record count,
/// record pointer, active index, the `ModelAttributes` it drives, and
/// an enable flag) — the per-map state `fieldmap.c` creates and steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaLightManager {
    archive: AreaLightArchive,
    active: usize,
    enabled: bool,
}

impl AreaLightManager {
    /// `AreaLightManager_New`: picks the record for `seconds_of_day`
    /// ([`AreaLightArchive::initial_index`]), enables the manager, and
    /// applies that record to `attrs`.
    #[must_use]
    pub fn new(archive: AreaLightArchive, seconds_of_day: u32, attrs: &mut ModelLighting) -> Self {
        let active = archive.initial_index(seconds_of_day);
        archive.templates[active].apply(attrs);
        Self {
            archive,
            active,
            enabled: true,
        }
    }

    /// `AreaLightManager_UpdateActiveTemplate`, once per field frame.
    /// With more than one record, the active index advances by one —
    /// wrapping to 0 — whenever `seconds_of_day / 2` falls outside the
    /// active record's window `[previous.until, active.until)` (signed
    /// compares), and the new record is applied to `attrs` when the
    /// manager is enabled. Returns whether a record was applied.
    ///
    /// Two consequences worth knowing: a jump of several records takes
    /// as many frames to catch up, and at midnight the clock falls
    /// below the last record's window start, so the manager steps to
    /// record 0 — the `until == 0` record every area archive begins
    /// with — for exactly one frame before stepping on to record 1.
    pub fn update(&mut self, seconds_of_day: u32, attrs: &mut ModelLighting) -> bool {
        let count = self.archive.templates.len();
        if count <= 1 {
            return false;
        }
        let half = i64::from(seconds_of_day / 2);
        let (start, end) = self.archive.window(self.active);
        if half >= i64::from(end) || half < i64::from(start) {
            self.active += 1;
            if self.active >= count {
                self.active = 0;
            }
            if self.enabled {
                self.archive.templates[self.active].apply(attrs);
                return true;
            }
        }
        false
    }

    /// The manager's enable flag (offset 0x10): when clear, the index
    /// still advances but nothing is applied.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The active record's index.
    #[must_use]
    pub fn active(&self) -> usize {
        self.active
    }

    /// The active record.
    #[must_use]
    pub fn template(&self) -> &LightTemplate {
        &self.archive.templates[self.active]
    }

    /// The archive the manager steps through.
    #[must_use]
    pub fn archive(&self) -> &AreaLightArchive {
        &self.archive
    }
}

/// `ov01_021EA3E0`'s first pass: records until a line starting `EOF`,
/// checked before each record and again on each record's tenth line.
fn count_records(text: &[u8]) -> Result<usize, NdsError> {
    let mut lines = Lines::new(text);
    let mut count = 0;
    loop {
        let mut line = lines.next()?;
        if is_eof(line) {
            return Ok(count);
        }
        for _ in 1..RECORD_LINES {
            line = lines.next()?;
        }
        count += 1;
        if is_eof(line) {
            return Ok(count);
        }
        if lines.exhausted() {
            return Err(NdsError::Invalid {
                what: "area light table ends without an EOF line",
            });
        }
    }
}

/// The loader's `EOF` test: the line's first three bytes.
fn is_eof(line: &[u8]) -> bool {
    line.starts_with(b"EOF")
}

/// `ov01_021EA578`: `enable,r,g,b,x,y,z`. An enable field other than 1
/// leaves the light off (its vector untouched — zero from the record
/// fill). A parsed color of exactly `0xFFFF` is indistinguishable from
/// the "off" marker the loader uses internally, so it turns the light
/// off too (the record's mask bit clears and the color is zeroed).
fn parse_light(line: &[u8]) -> Light {
    let mut fields = Fields::new(line);
    if ascii_to_long(fields.next()) != 1 {
        return Light::default();
    }
    let color = pack_color(&mut fields);
    let mut vector = [0i16; 3];
    for axis in &mut vector {
        // `strh` truncates the s32, then the clamp runs on the s16.
        let raw = ascii_to_long(fields.next()) as i16;
        *axis = raw.clamp(-VECTOR_LIMIT, VECTOR_LIMIT);
    }
    let enabled = color != 0xFFFF;
    Light {
        enabled,
        color: if enabled { color } else { 0 },
        vector,
    }
}

/// `ov01_021EA668`: `r,g,b` into a `GXRgb`.
fn parse_color(line: &[u8]) -> u16 {
    pack_color(&mut Fields::new(line))
}

/// Three fields, each truncated to 16 bits (`strh`), packed
/// `r | g << 5 | b << 10` in 32-bit registers and stored as 16.
fn pack_color(fields: &mut Fields<'_>) -> u16 {
    let r = ascii_to_long(fields.next()) as u16;
    let g = ascii_to_long(fields.next()) as u16;
    let b = ascii_to_long(fields.next()) as u16;
    (u32::from(r) | (u32::from(g) << 5) | (u32::from(b) << 10)) as u16
}

/// `Ascii_StrToL` (`src/ascii_util.c:28`) verbatim: digits are read
/// right to left with a running power of ten; a non-digit in the first
/// position negates on `-` and is otherwise ignored, a non-digit
/// anywhere else returns `-1`; arithmetic wraps as the C's does.
fn ascii_to_long(text: &[u8]) -> i32 {
    let mut pow10: i32 = 1;
    let mut num: i32 = 0;
    for (i, &c) in text.iter().enumerate().rev() {
        if c.is_ascii_digit() {
            num = num.wrapping_add(pow10.wrapping_mul(i32::from(c - b'0')));
        } else if i == 0 {
            if c == b'-' {
                num = num.wrapping_neg();
            }
        } else {
            return -1;
        }
        pow10 = pow10.wrapping_mul(10);
    }
    num
}

/// `Ascii_GetDelim` with `'\r'` over the whole file: each call copies
/// up to 256 bytes to the delimiter (or a NUL — the end of the text
/// reads as one), then skips the delimiter and a `'\n'` after it.
struct Lines<'a> {
    text: &'a [u8],
    pos: usize,
}

impl<'a> Lines<'a> {
    fn new(text: &'a [u8]) -> Self {
        Self { text, pos: 0 }
    }

    fn byte(&self, at: usize) -> u8 {
        self.text.get(at).copied().unwrap_or(0)
    }

    fn next(&mut self) -> Result<&'a [u8], NdsError> {
        let start = self.pos;
        for i in 0..MAX_LINE {
            let b = self.byte(start + i);
            if b == b'\r' || b == 0 {
                let end = (start + i).min(self.text.len());
                let line = &self.text[start.min(end)..end];
                self.pos = start + i + 1;
                if b == b'\r' && self.byte(self.pos) == b'\n' {
                    self.pos += 1;
                }
                return Ok(line);
            }
        }
        Err(NdsError::Invalid {
            what: "area light line exceeds the 256-byte line buffer",
        })
    }

    fn exhausted(&self) -> bool {
        self.pos >= self.text.len()
    }
}

/// `Ascii_GetDelim` with `','` over one copied line: a field per call,
/// empty once the line is spent (the C would read on past the NUL).
struct Fields<'a> {
    line: &'a [u8],
    pos: usize,
}

impl<'a> Fields<'a> {
    fn new(line: &'a [u8]) -> Self {
        Self { line, pos: 0 }
    }

    fn next(&mut self) -> &'a [u8] {
        let rest = &self.line[self.pos.min(self.line.len())..];
        match rest.iter().position(|&b| b == b',' || b == 0) {
            Some(i) => {
                self.pos += i + 1;
                &rest[..i]
            }
            None => {
                self.pos = self.line.len() + 1;
                rest
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-record archive in the loader's exact shape, with
    /// invented numbers: record 0 (`until` 0) lights 0 and 2 on, record
    /// 1 ends at 1800 (01:00), record 2 at midnight. Light 2's `z`
    /// overshoots the clamp, light 1's enable field is `0`, light 3's
    /// is `2`.
    const FIXTURE: &[u8] = b"0,\r\n\
1,1,2,3,-10,-20,-30,\r\n\
0,0,0,0,0,0,0,\r\n\
1,4,5,6,0,0,5000,\r\n\
2,7,7,7,1,1,1,\r\n\
1,2,3,\r\n\
4,5,6,\r\n\
7,8,9,\r\n\
10,11,12,\r\n\
\r\n\
1800,\r\n\
1,9,9,9,100,200,300,\r\n\
0,0,0,0,0,0,0,\r\n\
0,0,0,0,0,0,0,\r\n\
0,0,0,0,0,0,0,\r\n\
1,1,1,\r\n\
2,2,2,\r\n\
3,3,3,\r\n\
4,4,4,\r\n\
\r\n\
43200,\r\n\
1,1,2,3,-10,-20,-30,\r\n\
0,0,0,0,0,0,0,\r\n\
1,4,5,6,0,0,-5000,\r\n\
0,0,0,0,0,0,0,\r\n\
1,2,3,\r\n\
4,5,6,\r\n\
7,8,9,\r\n\
10,11,12,\r\n\
\r\n\
EOF";

    fn rgb(r: u16, g: u16, b: u16) -> u16 {
        r | g << 5 | b << 10
    }

    #[test]
    fn parses_records_fields_and_clamps() {
        let archive = AreaLightArchive::parse(FIXTURE).expect("fixture parses");
        let t = archive.templates();
        assert_eq!(t.len(), 3);
        assert_eq!([t[0].until, t[1].until, t[2].until], [0, 1800, 43200]);
        let first = &t[0];
        assert_eq!(
            first.lights[0],
            Light {
                enabled: true,
                color: rgb(1, 2, 3),
                vector: [-10, -20, -30]
            }
        );
        assert_eq!(first.lights[1], Light::default());
        assert_eq!(
            first.lights[2],
            Light {
                enabled: true,
                color: rgb(4, 5, 6),
                vector: [0, 0, 4096]
            }
        );
        // Enable field 2 is not 1: off, vector untouched.
        assert_eq!(first.lights[3], Light::default());
        assert_eq!(first.light_mask(), 0b0101);
        assert_eq!(first.diffuse, rgb(1, 2, 3));
        assert_eq!(first.ambient, rgb(4, 5, 6));
        assert_eq!(first.specular, rgb(7, 8, 9));
        assert_eq!(first.emission, rgb(10, 11, 12));
        assert_eq!(t[2].lights[2].vector, [0, 0, -4096]);
        assert_eq!(t[1].light_mask(), 0b0001);
        assert_eq!(archive.window(0), (0, 0));
        assert_eq!(archive.window(1), (0, 1800));
        assert_eq!(archive.window(2), (1800, 43200));
    }

    #[test]
    fn apply_writes_every_attribute_and_zeroes_disabled_lights() {
        let archive = AreaLightArchive::parse(FIXTURE).expect("fixture parses");
        let mut attrs = ModelLighting {
            light_vectors: [[7; 3]; 4],
            light_colors: [7; 4],
            diffuse_is_vertex_color: true,
            specular_shininess: true,
            ..ModelLighting::default()
        };
        archive.templates()[0].apply(&mut attrs);
        assert_eq!(attrs.light_vectors, [[-10, -20, -30], [0; 3], [0, 0, 4096], [0; 3]]);
        assert_eq!(attrs.light_colors, [rgb(1, 2, 3), 0, rgb(4, 5, 6), 0]);
        assert_eq!(
            (attrs.diffuse, attrs.ambient, attrs.specular, attrs.emission),
            (rgb(1, 2, 3), rgb(4, 5, 6), rgb(7, 8, 9), rgb(10, 11, 12))
        );
        assert!(!attrs.diffuse_is_vertex_color);
        assert!(!attrs.specular_shininess);
    }

    #[test]
    fn selection_compares_half_seconds_against_until() {
        let archive = AreaLightArchive::parse(FIXTURE).expect("fixture parses");
        // A leading until-0 record is never the initial pick.
        assert_eq!(archive.initial_index(0), 1);
        assert_eq!(archive.initial_index(3599), 1); // half 1799 < 1800
        assert_eq!(archive.initial_index(3600), 2); // half 1800
        assert_eq!(archive.initial_index(86_399), 2); // half 43199 < 43200
        assert_eq!(archive.template_at(0).until, 1800);
        // Nothing beyond the last threshold: the fallback is record 0.
        let mut early = FIXTURE.to_vec();
        let last = early.windows(7).position(|w| w == b"43200,\r").expect("record 2");
        early.splice(last..last + 6, b"100,".iter().copied());
        let early = AreaLightArchive::parse(&early).expect("parses");
        assert_eq!(early.templates()[2].until, 100);
        // The scan is first-match, so the out-of-order 100 never wins
        // over 1800; past every threshold the pick is record 0.
        assert_eq!(early.initial_index(199), 1);
        assert_eq!(early.initial_index(3599), 1);
        assert_eq!(early.initial_index(3600), 0);
    }

    #[test]
    fn manager_steps_one_record_per_update_and_wraps_at_midnight() {
        let archive = AreaLightArchive::parse(FIXTURE).expect("fixture parses");
        let mut attrs = ModelLighting::default();
        let mut manager = AreaLightManager::new(archive, 0, &mut attrs);
        assert_eq!(manager.active(), 1);
        assert_eq!(attrs.light_colors[0], rgb(9, 9, 9));
        assert!(!manager.update(3599, &mut attrs));
        assert!(manager.update(3600, &mut attrs));
        assert_eq!(manager.active(), 2);
        assert_eq!(attrs.light_vectors[2], [0, 0, -4096]);
        assert!(!manager.update(86_399, &mut attrs));
        // Midnight: below the window start → wrap to record 0 for one
        // update, then on to record 1.
        assert!(manager.update(0, &mut attrs));
        assert_eq!(manager.active(), 0);
        assert_eq!(attrs.light_vectors[2], [0, 0, 4096]);
        assert!(manager.update(0, &mut attrs));
        assert_eq!(manager.active(), 1);
        assert!(!manager.update(1, &mut attrs));
        // A jump of two records takes two updates to catch up.
        assert!(manager.update(80_000, &mut attrs));
        assert_eq!(manager.active(), 2);
        assert!(!manager.update(80_000, &mut attrs));
        // Disabled: the index moves, nothing is applied.
        manager.set_enabled(false);
        let before = attrs;
        assert!(!manager.update(0, &mut attrs));
        assert_eq!(manager.active(), 0);
        assert_eq!(attrs, before);
    }

    #[test]
    fn single_record_archives_never_update() {
        // The first record alone, then EOF.
        let mut text = FIXTURE.to_vec();
        let second = text.windows(6).position(|w| w == b"1800,\r").expect("record 1");
        text.truncate(second);
        text.extend_from_slice(b"EOF");
        let archive = AreaLightArchive::parse(&text).expect("one record");
        assert_eq!(archive.templates().len(), 1);
        let mut attrs = ModelLighting::default();
        let mut manager = AreaLightManager::new(archive, 40_000, &mut attrs);
        assert_eq!(manager.active(), 0);
        assert!(!manager.update(0, &mut attrs));
        assert!(!manager.update(86_399, &mut attrs));
    }

    #[test]
    fn eof_on_a_tenth_line_ends_the_count() {
        // Nine lines then EOF as the separator slot: one record, and
        // the second pass reads EOF as the separator harmlessly.
        let mut text = Vec::new();
        for line in FIXTURE.split(|&b| b == b'\n').take(9) {
            text.extend_from_slice(line);
            text.push(b'\n');
        }
        text.extend_from_slice(b"EOF\r\n");
        let archive = AreaLightArchive::parse(&text).expect("parses");
        assert_eq!(archive.templates().len(), 1);
        assert_eq!(archive.templates()[0].light_mask(), 0b0101);
    }

    #[test]
    fn malformed_texts_are_rejected() {
        assert!(AreaLightArchive::parse(b"EOF").is_err(), "no records");
        assert!(AreaLightArchive::parse(b"").is_err(), "empty");
        let mut long = vec![b'1'; 300];
        long.extend_from_slice(b"\r\nEOF");
        assert!(AreaLightArchive::parse(&long).is_err(), "overlong line");
        let mut negative = FIXTURE.to_vec();
        negative.splice(0..2, b"-1,".iter().copied());
        assert!(AreaLightArchive::parse(&negative).is_err(), "negative threshold");
    }

    #[test]
    fn str_to_long_matches_the_ascii_util_quirks() {
        assert_eq!(ascii_to_long(b"123"), 123);
        assert_eq!(ascii_to_long(b"-5"), -5);
        assert_eq!(ascii_to_long(b""), 0);
        // A leading non-digit is ignored, not an error...
        assert_eq!(ascii_to_long(b"x42"), 42);
        // ...but one anywhere else is -1.
        assert_eq!(ascii_to_long(b"4x2"), -1);
        assert_eq!(ascii_to_long(b"12 "), -1);
    }

    #[test]
    fn lines_and_fields_follow_get_delim() {
        let mut lines = Lines::new(b"a,b\r\nc\rd\n\r\ne");
        assert_eq!(lines.next().unwrap(), b"a,b");
        assert_eq!(lines.next().unwrap(), b"c");
        // A bare CR skips no LF; the LF then heads the next line.
        assert_eq!(lines.next().unwrap(), b"d\n");
        assert_eq!(lines.next().unwrap(), b"e");
        assert!(lines.exhausted());
        assert_eq!(lines.next().unwrap(), b"");
        let mut fields = Fields::new(b"1,22,,333");
        assert_eq!(fields.next(), b"1");
        assert_eq!(fields.next(), b"22");
        assert_eq!(fields.next(), b"");
        assert_eq!(fields.next(), b"333");
        assert_eq!(fields.next(), b"");
    }

    #[test]
    fn archive_ids_follow_the_light_type_switch() {
        assert_eq!(archive_for_light_type(0, false), 1);
        assert_eq!(archive_for_light_type(1, false), 0);
        assert_eq!(archive_for_light_type(2, false), 3);
        assert_eq!(archive_for_light_type(2, true), 4);
        assert_eq!(archive_for_light_type(3, true), 0);
        assert_eq!(archive_for_light_type(0, true), 1);
        assert_eq!(ARCHIVE_PATHS.len(), 5);
    }
}
