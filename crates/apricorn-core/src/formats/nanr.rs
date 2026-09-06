//! NANR — Nitro Animation: a bank of cell-animation sequences. Magic
//! `RNAN`.
//!
//! A NANR pairs with a NCER cell bank and a NCGR sheet (see
//! [`crate::formats::ncer`]): each *sequence* steps through frames that
//! reference cells, optionally with transforms.
//!
//! Beyond the shared Nitro container header (see [`crate::formats`]) every
//! retail HeartGold file carries exactly three sections, in order:
//!
//! - **KNBA** (`ABNK` reversed — the SDK's animation-bank block):
//!
//!   ```text
//!   +0x00 2  nSequences
//!   +0x02 2  nFrames     (total, across all sequences)
//!   +0x04 4  sequenceOffset    (always 0x18)
//!   +0x08 4  frameOffset      (always 0x18 + 0x10*nSequences)
//!   +0x0C 4  resultOffset     (body-relative; the results pool base)
//!   +0x10 4  reserved (always 0)
//!   +0x14 4  uaatOffset       (0 = none; body-relative)
//!   ```
//!
//!   followed by the sequence records, the frame records, and the shared
//!   results pool:
//!
//!   - sequence records, `nSequences` × 0x10 bytes:
//!     `u16 frameCount, u16 loopStartFrame, u16 animationElement,
//!     u16 animationType, u32 playbackMode, u32 frameDataOffset`.
//!     `animationElement` picks what a frame's result holds (see
//!     [`AnimElement`]); `animationType` is always 1 on retail;
//!     `frameDataOffset` is relative to the frame area.
//!   - frame records, 8 bytes:
//!     `u32 resultOffset, u16 frameDelay, u16 magic 0xBEEF`.
//!   - the results pool — **deduplicated**: frames may share results
//!     across sequences, so the pool is validated by bounds only (each
//!     frame's `resultOffset` + its element's result size must stay
//!     within the pool, which ends at the UAAT block if present, else at
//!     the section end).
//! - **LBAL** — the label bank (see [`crate::formats`]). Unlike a NCER's
//!   labels, a NANR's label count always equals `nSequences` — the labels
//!   name the sequences in order.
//! - **UEXT** (`TXEU`) — user-extension marker; always 0x0C bytes of zero.
//! - **UAAT** (`TAAU`, embedded at `uaatOffset`, 146 retail files) —
//!   per-sequence and per-frame user attributes; see [`Uaat`].
//!
//! Retail HeartGold (US): 591 members, 2,500 sequences, 6,171 frames; see
//! `docs/nitro-sprite.md` for the worked ground truth.

use crate::formats::{nitro_labels, nitro_sections};
use crate::nds::{NdsError, u16le, u32le};

/// What a frame's result holds (`animationElement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimElement {
    /// A bare cell index (2-byte result).
    Cell,
    /// Scale-rotate-translate (0x10-byte result).
    Srt,
    /// Translate-only (8-byte result).
    Translate,
}

impl AnimElement {
    /// Decodes the raw `animationElement` value.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any value outside 0–2.
    pub(crate) fn from_raw(raw: u16) -> Result<Self, NdsError> {
        match raw {
            0 => Ok(Self::Cell),
            1 => Ok(Self::Srt),
            2 => Ok(Self::Translate),
            _ => Err(NdsError::Invalid {
                what: "unknown NANR animationElement",
            }),
        }
    }

    /// The size of one result of this element, in bytes.
    #[must_use]
    pub fn result_size(self) -> usize {
        match self {
            Self::Cell => 2,
            Self::Srt => 0x10,
            Self::Translate => 8,
        }
    }
}

/// How a sequence plays back (`playbackMode`,
/// `NNSG2dAnimationPlayMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayMode {
    /// Forward, stop at the end (`NNS_G2D_ANIMATIONPLAYMODE_FORWARD`).
    Forward,
    /// Forward, loop back to `loopStartFrame`
    /// (`NNS_G2D_ANIMATIONPLAYMODE_FORWARD_LOOP`).
    ForwardLoop,
    /// Reverse, stop at the end (`NNS_G2D_ANIMATIONPLAYMODE_REVERSE`).
    Reverse,
    /// Reverse, loop (`NNS_G2D_ANIMATIONPLAYMODE_REVERSE_LOOP`).
    ReverseLoop,
}

impl PlayMode {
    /// Decodes the raw `playbackMode` value.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any value outside the enum (retail
    /// files only use Forward and ForwardLoop).
    pub(crate) fn from_raw(raw: u32) -> Result<Self, NdsError> {
        match raw {
            1 => Ok(Self::Forward),
            2 => Ok(Self::ForwardLoop),
            3 => Ok(Self::Reverse),
            4 => Ok(Self::ReverseLoop),
            _ => Err(NdsError::Invalid {
                what: "unknown NANR playbackMode",
            }),
        }
    }
}

/// One frame's decoded result from the shared pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimResult {
    /// A bare cell index (`animationElement` 0).
    Cell {
        /// The NCER cell this frame shows.
        cell: u16,
    },
    /// Scale-rotate-translate (`animationElement` 1). The scales are
    /// raw fx32 fixed-point; `rotation` is the SDK's `rotZ`.
    Srt {
        /// The NCER cell this frame shows.
        cell: u16,
        /// Rotation angle (`rotZ`).
        rotation: u16,
        /// X scale, fx32.
        scale_x: u32,
        /// Y scale, fx32.
        scale_y: u32,
        /// X translation.
        x: i16,
        /// Y translation.
        y: i16,
    },
    /// Translate-only (`animationElement` 2).
    Translate {
        /// The NCER cell this frame shows.
        cell: u16,
        /// X translation.
        x: i16,
        /// Y translation.
        y: i16,
    },
}

/// One frame: a delay plus a reference into the shared results pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The frame's offset in the shared results pool.
    pub result_off: u32,
    /// How many ticks the frame holds before the sequence advances.
    pub delay: u16,
}

/// One animation sequence. Borrows the file bytes; see [`Nanr::parse`].
#[derive(Debug)]
pub struct Sequence<'a> {
    frame_start: usize,
    frames: Vec<Frame>,
    /// Index of the frame the sequence restarts from when looping.
    loop_start: u16,
    element: AnimElement,
    play_mode: PlayMode,
    label: &'a str,
    /// The shared results pool (`resultOffset` .. pool end).
    pool: &'a [u8],
}

impl Sequence<'_> {
    /// The number of frames in the sequence.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// The index of the frame the sequence restarts from when looping;
    /// always strictly less than the frame count.
    #[must_use]
    pub fn loop_start(&self) -> u16 {
        self.loop_start
    }

    /// What each frame's result holds.
    #[must_use]
    pub fn element(&self) -> AnimElement {
        self.element
    }

    /// How the sequence plays back.
    #[must_use]
    pub fn play_mode(&self) -> PlayMode {
        self.play_mode
    }

    /// The sequence's label (one label per sequence, in order).
    #[must_use]
    pub fn label(&self) -> &str {
        self.label
    }

    /// The index of the sequence's first frame within the whole bank's
    /// frame numbering (the UAAT frame attributes use that numbering).
    #[must_use]
    pub fn frame_start(&self) -> usize {
        self.frame_start
    }

    /// The sequence's frames, in order.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Decodes frame `i`'s result from the shared pool.
    #[must_use]
    pub fn result(&self, i: usize) -> Option<AnimResult> {
        let frame = self.frames.get(i)?;
        let at = frame.result_off as usize;
        let pool = self.pool.get(at..)?;
        let u16at = |n: usize| u16::from_le_bytes([pool[2 * n], pool[2 * n + 1]]);
        match self.element {
            AnimElement::Cell => {
                if pool.len() < 2 {
                    return None;
                }
                Some(AnimResult::Cell { cell: u16at(0) })
            }
            AnimElement::Srt => {
                if pool.len() < 0x10 {
                    return None;
                }
                Some(AnimResult::Srt {
                    cell: u16at(0),
                    rotation: u16at(1),
                    scale_x: u32::from_le_bytes(pool[4..8].try_into().ok()?),
                    scale_y: u32::from_le_bytes(pool[8..12].try_into().ok()?),
                    x: u16at(6) as i16,
                    y: u16at(7) as i16,
                })
            }
            AnimElement::Translate => {
                if pool.len() < 8 {
                    return None;
                }
                Some(AnimResult::Translate {
                    cell: u16at(0),
                    x: u16at(2) as i16,
                    y: u16at(3) as i16,
                })
            }
        }
    }
}

/// The UAAT user-attribute block (magic `TAAU`): one `u32` attribute per
/// sequence and one per frame.
///
/// On disk the block carries a header (`u16 nSequences, u16
/// attrsPerFrame(1), u32 reserved(8)`), a 0x0C record per sequence, and
/// *four* pointer arrays — but every pointer value on every retail file
/// is fully derived, so [`Nanr::parse`] validates them and keeps only the
/// two attribute arrays.
#[derive(Debug)]
pub struct Uaat {
    seq_attrs: Vec<u32>,
    frame_attrs: Vec<u32>,
}

impl Uaat {
    /// The per-sequence user attributes, in sequence order.
    #[must_use]
    pub fn seq_attrs(&self) -> &[u32] {
        &self.seq_attrs
    }

    /// The per-frame user attributes, in the bank's global frame
    /// numbering (see [`Sequence::frame_start`]).
    #[must_use]
    pub fn frame_attrs(&self) -> &[u32] {
        &self.frame_attrs
    }

    /// The user attribute of sequence `i`.
    #[must_use]
    pub fn seq_attr(&self, i: usize) -> Option<u32> {
        self.seq_attrs.get(i).copied()
    }

    /// The user attribute of global frame `i` (see
    /// [`Sequence::frame_start`]).
    #[must_use]
    pub fn frame_attr(&self, i: usize) -> Option<u32> {
        self.frame_attrs.get(i).copied()
    }
}

/// A parsed NANR. Borrows the file bytes; see [`Nanr::parse`].
#[derive(Debug)]
pub struct Nanr<'a> {
    /// Container version (always 0x0100 on retail files).
    version: u16,
    sequences: Vec<Sequence<'a>>,
    uaat: Option<Uaat>,
}

/// Whether `data` begins with a NANR header.
#[must_use]
pub fn is_nanr(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'R', b'N', b'A', b'N', 0xFF, 0xFE])
}

impl<'a> Nanr<'a> {
    /// Parses a complete NANR file.
    ///
    /// Beyond the usual container checks this enforces the full retail
    /// layout: the KNBA header constants, that every frame carries its
    /// 0xBEEF marker, that frame areas stay out of the results pool,
    /// that every frame's result stays within the pool, that the label
    /// count equals the sequence count, and — when a UAAT block is
    /// present — its size and every derived pointer.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any truncated, inconsistent, or
    /// non-retail-shaped file.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let (version, sections) = nitro_sections(data, b"RNAN", 3)?;
        if version != 0x0100 {
            return Err(NdsError::Invalid {
                what: "NANR version (always 0x0100 on retail files)",
            });
        }
        let [kbna, labl, uext] = &sections[..] else {
            return Err(NdsError::Invalid {
                what: "NANR section count",
            });
        };
        for (sec, magic, what) in [
            (
                kbna,
                b"KNBA",
                "NANR's first section is not the animation bank",
            ),
            (labl, b"LBAL", "NANR's second section is not the label bank"),
            (
                uext,
                b"TXEU",
                "NANR's third section is not the user-extension block",
            ),
        ] {
            if sec.magic != *magic {
                return Err(NdsError::Invalid { what });
            }
        }
        if uext.size != 0xC || u32le(data, uext.offset + 8)? != 0 {
            return Err(NdsError::Invalid {
                what: "NANR UEXT block (always 0x0C bytes of zero)",
            });
        }

        let body = kbna.offset + 8;
        let body_len = kbna.size - 8;
        let body_end = body + body_len;
        if body_end > data.len() {
            return Err(NdsError::Truncated {
                what: "NANR KNBA section",
                need: body_end,
                got: data.len(),
            });
        }

        // --- KNBA header ---
        let n_seq = u16le(data, body)? as usize;
        let n_frames = u16le(data, body + 2)? as usize;
        if n_seq == 0 {
            return Err(NdsError::Invalid {
                what: "NANR sequence count",
            });
        }
        if u32le(data, body + 4)? as usize != 0x18 {
            return Err(NdsError::Invalid {
                what: "NANR sequenceOffset (always 0x18)",
            });
        }
        let frame_off = u32le(data, body + 8)? as usize;
        if frame_off != 0x18 + 0x10 * n_seq {
            return Err(NdsError::Invalid {
                what: "NANR frameOffset (must follow the sequence records)",
            });
        }
        let result_off = u32le(data, body + 0xC)? as usize;
        if u32le(data, body + 0x10)? != 0 {
            return Err(NdsError::Invalid {
                what: "NANR KNBA reserved field",
            });
        }
        let uaat_off = u32le(data, body + 0x14)? as usize;
        if uaat_off > body_len {
            return Err(NdsError::Invalid {
                what: "NANR uaatOffset (must sit inside the section)",
            });
        }
        let pool_end = if uaat_off != 0 { uaat_off } else { body_len };
        if result_off > pool_end {
            return Err(NdsError::Invalid {
                what: "NANR resultOffset (must sit before the pool end)",
            });
        }
        if frame_off > result_off {
            return Err(NdsError::Invalid {
                what: "NANR frameOffset (must sit before the results pool)",
            });
        }
        let pool = data
            .get(body + result_off..body + pool_end)
            .ok_or(NdsError::Truncated {
                what: "NANR results pool",
                need: body + pool_end,
                got: data.len(),
            })?;

        // --- Sequence and frame records ---
        let mut sequences: Vec<Sequence> = Vec::with_capacity(n_seq);
        let mut total_frames = 0usize;
        for i in 0..n_seq {
            let at = body + 0x18 + 0x10 * i;
            let frame_count = u16le(data, at)? as usize;
            if frame_count == 0 {
                return Err(NdsError::Invalid {
                    what: "NANR sequence frame count",
                });
            }
            let loop_start = u16le(data, at + 2)?;
            if usize::from(loop_start) >= frame_count {
                return Err(NdsError::Invalid {
                    what: "NANR loopStartFrame (must precede the last frame)",
                });
            }
            let element = AnimElement::from_raw(u16le(data, at + 4)?)?;
            if u16le(data, at + 6)? != 1 {
                return Err(NdsError::Invalid {
                    what: "NANR animationType (always 1 on retail files)",
                });
            }
            let play_mode = PlayMode::from_raw(u32le(data, at + 8)?)?;
            let frame_data_off = u32le(data, at + 12)? as usize;

            let frames_start = frame_off + frame_data_off;
            let frames_end = frames_start + 8 * frame_count;
            if frames_start < frame_off || frames_end > result_off {
                return Err(NdsError::Invalid {
                    what: "NANR frame records (must stay out of the results pool)",
                });
            }
            let mut frames = Vec::with_capacity(frame_count);
            for k in 0..frame_count {
                let fr = body + frame_off + frame_data_off + 8 * k;
                let result_frame_off = u32le(data, fr)?;
                let delay = u16le(data, fr + 4)?;
                if u16le(data, fr + 6)? != 0xBEEF {
                    return Err(NdsError::Invalid {
                        what: "NANR frame marker (always 0xBEEF)",
                    });
                }
                if result_frame_off as usize + element.result_size() > pool.len() {
                    return Err(NdsError::Invalid {
                        what: "NANR frame result reaches beyond the pool",
                    });
                }
                frames.push(Frame {
                    result_off: result_frame_off,
                    delay,
                });
            }
            sequences.push(Sequence {
                frame_start: total_frames,
                frames,
                loop_start,
                element,
                play_mode,
                label: "",
                pool,
            });
            total_frames += frame_count;
        }
        if total_frames != n_frames {
            return Err(NdsError::Invalid {
                what: "NANR nFrames (must equal the summed sequence frame counts)",
            });
        }

        // --- Optional UAAT block ---
        let mut uaat = None;
        if uaat_off != 0 {
            uaat = Some(parse_uaat(data, body, uaat_off, body_len, &sequences)?);
        }

        // --- Labels: one per sequence, on every retail file ---
        let labels = nitro_labels(data, labl.offset, labl.size)?;
        if labels.len() != n_seq {
            return Err(NdsError::Invalid {
                what: "NANR label count (must equal the sequence count)",
            });
        }
        for (seq, label) in sequences.iter_mut().zip(labels) {
            seq.label = label;
        }

        Ok(Self {
            version,
            sequences,
            uaat,
        })
    }

    /// The container version (always 0x0100 on retail files).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The number of sequences in the bank.
    #[must_use]
    pub fn sequence_count(&self) -> usize {
        self.sequences.len()
    }

    /// All sequences, in bank order.
    #[must_use]
    pub fn sequences(&self) -> &[Sequence<'a>] {
        &self.sequences
    }

    /// Sequence `i`, if it exists.
    #[must_use]
    pub fn sequence(&self, i: usize) -> Option<&Sequence<'a>> {
        self.sequences.get(i)
    }

    /// The total number of frames across all sequences.
    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.sequences.iter().map(|s| s.frames.len()).sum()
    }

    /// The UAAT user-attribute block, if the bank has one.
    #[must_use]
    pub fn uaat(&self) -> Option<&Uaat> {
        self.uaat.as_ref()
    }

    /// The UAAT user attribute of frame `frame` of sequence `seq`, if
    /// the bank has a UAAT block.
    #[must_use]
    pub fn frame_attr(&self, seq: usize, frame: usize) -> Option<u32> {
        let start = self.sequences.get(seq)?.frame_start;
        self.uaat.as_ref()?.frame_attr(start + frame)
    }
}

/// Parses and fully validates a UAAT block embedded at `uaat_off` of the
/// KNBA body.
///
/// Layout (all offsets body-relative, all pointers relative to the block
/// body start — `uaat_off + 8`):
///
/// ```text
/// +0x00 4  "TAAU"   +0x04 4  size = 0x10 + 0x10*nSeq + 8*nFrames
/// +0x08 2  nSeq     +0x0A 2  attrsPerFrame (1)   +0x0C 4  reserved (8)
/// +0x10     one 0x0C record per sequence:
///           u16 frameCount, u16 0xBEEF, u32 seqAttrPtr, u32 frameAttrPtr
/// +0x10+0x0C*nSeq   u32 frameSinglePtr[nFrames]
///            +4*nFrames  u32 seqAttrs[nSeq]
///            +4*nSeq     u32 frameAttrs[nFrames]   (ends the section)
/// ```
///
/// Every pointer value is fully derived on retail files: the sequence
/// attributes point at `seqAttrs[i]`, the sequence's "double" pointer at
/// its first frame's `frameSinglePtr`, and each single pointer at its
/// `frameAttrs[j]`.
fn parse_uaat(
    data: &[u8],
    body: usize,
    uaat_off: usize,
    body_len: usize,
    sequences: &[Sequence<'_>],
) -> Result<Uaat, NdsError> {
    let n_seq = sequences.len();
    let n_frames: usize = sequences.iter().map(|s| s.frames.len()).sum();
    let at = body + uaat_off;
    let end = at + 0x10 + 0x10 * n_seq + 8 * n_frames;
    if end != body + body_len {
        return Err(NdsError::Invalid {
            what: "NANR UAAT size (must tile the section end)",
        });
    }
    if &data[at..at + 4] != b"TAAU" {
        return Err(NdsError::Invalid {
            what: "NANR UAAT magic",
        });
    }
    if u32le(data, at + 4)? as usize != 0x10 + 0x10 * n_seq + 8 * n_frames {
        return Err(NdsError::Invalid {
            what: "NANR UAAT size field",
        });
    }
    if u16le(data, at + 8)? as usize != n_seq {
        return Err(NdsError::Invalid {
            what: "NANR UAAT nSequences (must match the bank)",
        });
    }
    if u16le(data, at + 0xA)? != 1 {
        return Err(NdsError::Invalid {
            what: "NANR UAAT attrsPerFrame (always 1)",
        });
    }
    if u32le(data, at + 0xC)? != 8 {
        return Err(NdsError::Invalid {
            what: "NANR UAAT reserved field",
        });
    }

    // All pointers are relative to the UAAT body start (+8 into the block).
    let base = uaat_off + 8;
    let ptrs_base = uaat_off + 0x10 + 0x0C * n_seq;
    let seqattr_base = ptrs_base + 4 * n_frames;
    let frameattr_base = seqattr_base + 4 * n_seq;

    // Per-sequence records: frame count and marker must mirror the
    // sequence record; the two pointers must be fully derived.
    for (i, seq) in sequences.iter().enumerate() {
        let rec = at + 0x10 + 0x0C * i;
        if u16le(data, rec)? as usize != seq.frames.len() {
            return Err(NdsError::Invalid {
                what: "NANR UAAT frame count (must mirror the sequence)",
            });
        }
        if u16le(data, rec + 2)? != 0xBEEF {
            return Err(NdsError::Invalid {
                what: "NANR UAAT frame marker (always 0xBEEF)",
            });
        }
        if u32le(data, rec + 4)? as usize != seqattr_base - base + 4 * i {
            return Err(NdsError::Invalid {
                what: "NANR UAAT sequence-attribute pointer",
            });
        }
        if u32le(data, rec + 8)? as usize != ptrs_base - base + 4 * seq.frame_start {
            return Err(NdsError::Invalid {
                what: "NANR UAAT frame-pointer-array pointer",
            });
        }
    }
    for j in 0..n_frames {
        if u32le(data, at + ptrs_base - uaat_off + 4 * j)? as usize != frameattr_base - base + 4 * j
        {
            return Err(NdsError::Invalid {
                what: "NANR UAAT frame-attribute pointer",
            });
        }
    }

    let mut seq_attrs = Vec::with_capacity(n_seq);
    for i in 0..n_seq {
        seq_attrs.push(u32le(data, at + seqattr_base - uaat_off + 4 * i)?);
    }
    let mut frame_attrs = Vec::with_capacity(n_frames);
    for j in 0..n_frames {
        frame_attrs.push(u32le(data, at + frameattr_base - uaat_off + 4 * j)?);
    }
    Ok(Uaat {
        seq_attrs,
        frame_attrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NANR in memory with one sequence of two frames that
    /// share a deduplicated SRT result, plus a UAAT block when
    /// requested. `labels` names the sequences (one label per sequence
    /// is the retail invariant).
    fn build_nanr(with_uaat: bool, labels: &[&[u8]]) -> Vec<u8> {
        let n_seq = 1usize;
        let frame_count = 2usize;

        // KNBA header + one sequence record.
        let mut kbna_body = vec![0u8; 0x18 + 0x10 * n_seq];
        kbna_body[0..2].copy_from_slice(&(n_seq as u16).to_le_bytes());
        kbna_body[2..4].copy_from_slice(&(frame_count as u16).to_le_bytes());
        kbna_body[4..8].copy_from_slice(&0x18u32.to_le_bytes());
        let frame_off = 0x18 + 0x10 * n_seq;
        kbna_body[8..12].copy_from_slice(&(frame_off as u32).to_le_bytes());
        let result_off = frame_off + 8 * frame_count; // frames, then pool
        kbna_body[12..16].copy_from_slice(&(result_off as u32).to_le_bytes());
        let seq_at = 0x18;
        kbna_body[seq_at..seq_at + 2].copy_from_slice(&(frame_count as u16).to_le_bytes());
        kbna_body[seq_at + 2..seq_at + 4].copy_from_slice(&1u16.to_le_bytes()); // loopStart
        kbna_body[seq_at + 4..seq_at + 6].copy_from_slice(&1u16.to_le_bytes()); // SRT
        kbna_body[seq_at + 6..seq_at + 8].copy_from_slice(&1u16.to_le_bytes()); // animType
        kbna_body[seq_at + 8..seq_at + 12].copy_from_slice(&2u32.to_le_bytes()); // FORWARD_LOOP
        kbna_body[seq_at + 12..seq_at + 16].copy_from_slice(&0u32.to_le_bytes()); // frameData

        // Frame records: both reference the same pool offset (patched in
        // once the SRT result lands).
        let mut fr = frame_off;
        for delay in [0u16, 3] {
            kbna_body.resize(fr + 8, 0);
            kbna_body[fr + 4..fr + 6].copy_from_slice(&delay.to_le_bytes());
            kbna_body[fr + 6..fr + 8].copy_from_slice(&0xBEEFu16.to_le_bytes());
            fr += 8;
        }

        // Results pool: one spare u16 at pool offset 0, then the shared
        // SRT result at pool offset 2.
        kbna_body.extend_from_slice(&0xFFFFu16.to_le_bytes());
        let srt_pool_off = kbna_body.len() - result_off;
        kbna_body.extend_from_slice(&5u16.to_le_bytes()); // cell
        kbna_body.extend_from_slice(&90u16.to_le_bytes()); // rotation
        kbna_body.extend_from_slice(&0x100u32.to_le_bytes()); // scaleX (fx32)
        kbna_body.extend_from_slice(&0x180u32.to_le_bytes()); // scaleY
        kbna_body.extend_from_slice(&(-4i16).to_le_bytes()); // x
        kbna_body.extend_from_slice(&6i16.to_le_bytes()); // y
        for k in 0..frame_count {
            let f = frame_off + 8 * k;
            kbna_body[f..f + 4].copy_from_slice(&(srt_pool_off as u32).to_le_bytes());
        }

        // Optional UAAT block.
        let uaat_off = if with_uaat {
            let off = kbna_body.len();
            let uaat_size = 0x10 + 0x10 * n_seq + 8 * frame_count;
            kbna_body.extend_from_slice(b"TAAU");
            kbna_body.extend_from_slice(&(uaat_size as u32).to_le_bytes());
            kbna_body.extend_from_slice(&(n_seq as u16).to_le_bytes());
            kbna_body.extend_from_slice(&1u16.to_le_bytes());
            kbna_body.extend_from_slice(&8u32.to_le_bytes());
            // Per-seq record.
            kbna_body.extend_from_slice(&(frame_count as u16).to_le_bytes());
            kbna_body.extend_from_slice(&0xBEEFu16.to_le_bytes());
            let ptrs_base = off + 0x10 + 0x0C * n_seq;
            let seqattr_base = ptrs_base + 4 * frame_count;
            let frameattr_base = seqattr_base + 4 * n_seq;
            kbna_body.extend_from_slice(&((seqattr_base - off - 8) as u32).to_le_bytes());
            kbna_body.extend_from_slice(&((ptrs_base - off - 8) as u32).to_le_bytes());
            // Frame single pointers.
            for j in 0..frame_count {
                kbna_body
                    .extend_from_slice(&((frameattr_base - off - 8 + 4 * j) as u32).to_le_bytes());
            }
            // Sequence attributes, then frame attributes.
            kbna_body.extend_from_slice(&0xA5u32.to_le_bytes());
            for j in 0..frame_count {
                kbna_body.extend_from_slice(&(0x100 + j as u32).to_le_bytes());
            }
            Some(off)
        } else {
            None
        };

        let kbna_size = 8 + kbna_body.len();

        // LBAL: strictly increasing offsets relative to the table end.
        let mut labl_body = Vec::new();
        let table_size = 4 * labels.len();
        let mut pos = table_size;
        for label in labels {
            labl_body.extend_from_slice(&((pos - table_size) as u32).to_le_bytes());
            pos += label.len() + 1;
        }
        for label in labels {
            labl_body.extend_from_slice(label);
            labl_body.push(0);
        }

        let total = 0x10 + kbna_size + (8 + labl_body.len()) + 0xC;
        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RNAN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&3u16.to_le_bytes());

        let mut off = 0x10;
        rom[off..off + 4].copy_from_slice(b"KNBA");
        rom[off + 4..off + 8].copy_from_slice(&(kbna_size as u32).to_le_bytes());
        rom[off + 8..off + 8 + kbna_body.len()].copy_from_slice(&kbna_body);
        let uaat_field = uaat_off.unwrap_or(0);
        rom[off + 8 + 0x14..off + 8 + 0x18].copy_from_slice(&(uaat_field as u32).to_le_bytes());
        off += kbna_size;

        rom[off..off + 4].copy_from_slice(b"LBAL");
        rom[off + 4..off + 8].copy_from_slice(&((8 + labl_body.len()) as u32).to_le_bytes());
        rom[off + 8..off + 8 + labl_body.len()].copy_from_slice(&labl_body);
        off += 8 + labl_body.len();

        rom[off..off + 4].copy_from_slice(b"TXEU");
        rom[off + 4..off + 8].copy_from_slice(&0xCu32.to_le_bytes());
        rom
    }

    #[test]
    fn parses_sequences_frames_results_and_uaat() {
        for with_uaat in [false, true] {
            let data = build_nanr(with_uaat, &[b"idle"]);
            assert!(is_nanr(&data));
            let nanr = Nanr::parse(&data).expect("synthetic NANR must parse");
            assert_eq!(nanr.version(), 0x0100);
            assert_eq!(nanr.sequence_count(), 1);
            assert_eq!(nanr.total_frames(), 2);

            let seq = nanr.sequence(0).unwrap();
            assert_eq!(seq.frame_count(), 2);
            assert_eq!(seq.loop_start(), 1);
            assert_eq!(seq.element(), AnimElement::Srt);
            assert_eq!(seq.play_mode(), PlayMode::ForwardLoop);
            assert_eq!(seq.label(), "idle");
            assert_eq!(seq.frame_start(), 0);
            assert_eq!(seq.frames()[0].delay, 0);
            assert_eq!(seq.frames()[1].delay, 3);

            // Both frames decode the same shared (deduplicated) SRT result.
            let result = AnimResult::Srt {
                cell: 5,
                rotation: 90,
                scale_x: 0x100,
                scale_y: 0x180,
                x: -4,
                y: 6,
            };
            assert_eq!(seq.result(0), Some(result));
            assert_eq!(seq.result(1), Some(result));
            assert_eq!(seq.result(2), None);

            match (with_uaat, nanr.uaat()) {
                (true, Some(u)) => {
                    assert_eq!(u.seq_attrs(), &[0xA5]);
                    assert_eq!(u.frame_attrs(), &[0x100, 0x101]);
                    assert_eq!(u.seq_attr(0), Some(0xA5));
                    assert_eq!(u.seq_attr(1), None);
                    assert_eq!(nanr.frame_attr(0, 1), Some(0x101));
                    assert_eq!(nanr.frame_attr(0, 2), None);
                    assert_eq!(nanr.frame_attr(1, 0), None);
                }
                (false, None) => {}
                _ => panic!("UAAT presence mismatch"),
            }
        }
    }

    #[test]
    fn rejects_broken_nanr() {
        let good = build_nanr(true, &[b"idle"]);

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"RNAM");
        assert!(Nanr::parse(&bad_magic).is_err());

        assert!(Nanr::parse(&good[..good.len() - 2]).is_err());

        let field = |rom: &mut [u8], at: usize, value: u32| {
            rom[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };

        // KNBA body starts at 0x18. Break each header invariant in turn.
        let mut bad_seq_off = good.clone();
        field(&mut bad_seq_off, 0x18 + 4, 0x20);
        assert!(Nanr::parse(&bad_seq_off).is_err());

        let mut bad_reserved = good.clone();
        field(&mut bad_reserved, 0x18 + 0x10, 1);
        assert!(Nanr::parse(&bad_reserved).is_err());

        // The sequence record sits at body+0x18, its frames at body+0x28.
        let mut bad_loop = good.clone();
        bad_loop[0x18 + 0x18 + 2..0x18 + 0x18 + 4].copy_from_slice(&2u16.to_le_bytes());
        assert!(Nanr::parse(&bad_loop).is_err());

        let mut bad_element = good.clone();
        bad_element[0x18 + 0x18 + 4..0x18 + 0x18 + 6].copy_from_slice(&3u16.to_le_bytes());
        assert!(Nanr::parse(&bad_element).is_err());

        let mut bad_type = good.clone();
        bad_type[0x18 + 0x18 + 6..0x18 + 0x18 + 8].copy_from_slice(&0u16.to_le_bytes());
        assert!(Nanr::parse(&bad_type).is_err());

        let mut bad_play = good.clone();
        field(&mut bad_play, 0x18 + 0x18 + 8, 0);
        assert!(Nanr::parse(&bad_play).is_err());

        // Frame records: marker, pool bounds, and the frame-count total.
        let mut bad_beef = good.clone();
        bad_beef[0x18 + 0x28 + 6..0x18 + 0x28 + 8].copy_from_slice(&0xDEADu16.to_le_bytes());
        assert!(Nanr::parse(&bad_beef).is_err());

        let mut bad_result = good.clone();
        field(&mut bad_result, 0x18 + 0x28, 0x1000);
        assert!(Nanr::parse(&bad_result).is_err());

        let mut bad_total = good.clone();
        bad_total[0x18 + 2..0x18 + 4].copy_from_slice(&3u16.to_le_bytes());
        assert!(Nanr::parse(&bad_total).is_err());

        // Label count must equal the sequence count: two labels, one
        // sequence.
        let bad_labels = build_nanr(true, &[b"idle", b"run"]);
        assert!(Nanr::parse(&bad_labels).is_err());

        // Break the UAAT reserved field (UAAT starts where the pool ends).
        let uaat_field =
            u32::from_le_bytes(good[0x18 + 0x14..0x18 + 0x18].try_into().unwrap()) as usize;
        let mut bad_uaat = good.clone();
        field(&mut bad_uaat, 0x18 + uaat_field + 0xC, 0);
        assert!(Nanr::parse(&bad_uaat).is_err());

        // Break the UAAT sequence-attribute pointer derivation.
        let mut bad_ptr = good.clone();
        field(&mut bad_ptr, 0x18 + uaat_field + 0x14, 0);
        assert!(Nanr::parse(&bad_ptr).is_err());

        assert!(!is_nanr(b"not a nanr at all"));
        assert!(Nanr::parse(b"RNAN").is_err());
    }
}
