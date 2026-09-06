//! NitroSDK file formats stored inside the ROM.
//!
//! The NitroFS is only a shell; the game's actual content lives in NARC
//! archives (see [`narc`]), which in turn hold the graphics/audio/text
//! formats (NCGR, NCLR, SDAT, …) that later modules parse.

pub mod narc;

pub use narc::{Narc, is_narc};
