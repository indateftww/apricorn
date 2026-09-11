//! Executor tests: the early-game command subset run from
//! hand-assembled bytecode against the [`RecordingHost`] mock — every
//! handler family, the `Task_RunScripts` frame driver, `CallStd`
//! context spawning, the synchronous map-load runner, and the
//! non-panicking error paths (unknown, unimplemented, truncated).

use crate::formats::EOS;
use crate::input::{Keys, key};
use crate::rng::Lcrng;
use crate::save::vars_flags::{TRAINER_FLAG_BASE, VAR_LOTO_NUMBER_LO, VAR_PLAYER_STARTER};
use crate::text::string::GameString;

use super::bank::{MapBanks, SCRDEF_END, ScriptBank};
use super::commands::{MovementCommand, Opcode as Op};
use super::context::{Mode, NativeWait, ScriptContext, comparison};
use super::env::{FrameStatus, ScriptEnvironment};
use super::host::{
    AppRequest, FieldAction, FieldQuery, HostEvent, OBJ_PLAYER, PartyMon, PrintTarget,
    RecordingHost, ScriptHost, WaitFor, dir,
};
use super::{ScriptError, implemented_opcodes, is_implemented};

/// The map banks every test host reports (the bedroom's).
const MAP: MapBanks = MapBanks {
    scripts: 846,
    messages: 546,
};
/// `VAR_SPECIAL_RESULT`.
const RESULT: u16 = 0x800C;
/// Saved temporary variables.
const V0: u16 = 0x4000;
const V1: u16 = 0x4001;
const V2: u16 = 0x4002;
/// `CHAR_A`.
const A: u16 = 299;

/// A bytecode assembler over one bank member. The entry table is
/// reserved up front so [`Asm::here`] is a *member* offset — what the
/// `ScrDef` entries and every relative word measure.
struct Asm {
    code: Vec<u8>,
    entries: Vec<usize>,
    scripts: usize,
}

impl Asm {
    fn new(scripts: usize) -> Self {
        Self {
            code: vec![0; scripts * 4 + 2],
            entries: Vec::with_capacity(scripts),
            scripts,
        }
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    /// Marks the next script's entry point.
    fn entry(&mut self) -> &mut Self {
        self.entries.push(self.here());
        self
    }

    fn op(&mut self, op: Op) -> &mut Self {
        self.raw(op.code())
    }

    fn raw(&mut self, op: u16) -> &mut Self {
        self.code.extend_from_slice(&op.to_le_bytes());
        self
    }

    fn b(&mut self, v: u8) -> &mut Self {
        self.code.push(v);
        self
    }

    fn h(&mut self, v: u16) -> &mut Self {
        self.code.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn w(&mut self, v: u32) -> &mut Self {
        self.code.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// A relative word to an already-known target (`.word dest-.-4`).
    fn rel(&mut self, target: usize) -> &mut Self {
        let after = self.here() + 4;
        self.w((target as i64 - after as i64) as i32 as u32)
    }

    /// A relative word to be patched later; returns its position.
    fn fixup(&mut self) -> usize {
        let at = self.here();
        self.w(0);
        at
    }

    /// Points the word at `at` to `target`.
    fn patch(&mut self, at: usize, target: usize) -> &mut Self {
        let value = (target as i64 - (at as i64 + 4)) as i32 as u32;
        self.code[at..at + 4].copy_from_slice(&value.to_le_bytes());
        self
    }

    /// A movement list, `EndMovement` appended.
    fn movement(&mut self, steps: &[(u16, u16)]) -> &mut Self {
        for &(command, length) in steps {
            self.h(command).h(length);
        }
        self.h(254).h(0)
    }

    fn finish(mut self) -> Vec<u8> {
        assert_eq!(self.entries.len(), self.scripts, "every reserved entry defined");
        for (i, &target) in self.entries.iter().enumerate() {
            let value = (target - (i * 4 + 4)) as u32;
            self.code[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let end = self.scripts * 4;
        self.code[end..end + 2].copy_from_slice(&SCRDEF_END.to_le_bytes());
        self.code
    }
}

/// A single-script bank.
fn one(build: impl FnOnce(&mut Asm)) -> Vec<u8> {
    let mut a = Asm::new(1);
    a.entry();
    build(&mut a);
    a.finish()
}

/// Encodes `messages` as an `a/0/2/7` member with key 0 (the entry
/// table is then plain; the text still carries Decrypt2's rolling XOR
/// exactly as `MsgBank::to_bytes` writes it).
fn msg_bank(messages: &[&[u16]]) -> Vec<u8> {
    let count = messages.len();
    let mut out = Vec::new();
    out.extend_from_slice(&(count as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let mut cursor = (4 + 8 * count) as u32;
    for m in messages {
        let len = (m.len() + 1) as u32;
        out.extend_from_slice(&cursor.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        cursor += 2 * len;
    }
    for (n, m) in messages.iter().enumerate() {
        let mut seed = ((n as u64 + 1) * 596_947) as u16;
        for &unit in m.iter().chain(std::iter::once(&EOS)) {
            out.extend_from_slice(&(unit ^ seed).to_le_bytes());
            seed = seed.wrapping_add(18_749);
        }
    }
    out
}

/// A host over `bank` as the map's script bank, with a three-message
/// text bank ("A", "AB", "ABC").
fn host(bank: Vec<u8>) -> RecordingHost {
    let mut h = RecordingHost::new();
    h.scripts.insert(MAP.scripts, bank);
    h.messages
        .insert(MAP.messages, msg_bank(&[&[A], &[A, A + 1], &[A, A + 1, A + 2]]));
    h.map_banks = MAP;
    h
}

/// An environment set up for map script 1 (index 0 of the map bank).
fn env() -> ScriptEnvironment {
    let mut e = ScriptEnvironment::new();
    e.setup(1, None, dir::SOUTH);
    e
}

/// Runs frames until the task finishes; `(frames, status)`.
fn run(env: &mut ScriptEnvironment, host: &mut RecordingHost, max: usize) -> (usize, FrameStatus) {
    for frame in 1..=max {
        match env.run_frame(host).unwrap() {
            FrameStatus::Running => {}
            done => return (frame, done),
        }
    }
    panic!("script did not finish in {max} frames: {:?}", host.events);
}

/// Builds, runs to completion, and hands back the host and environment.
fn run_one(build: impl FnOnce(&mut Asm)) -> (RecordingHost, ScriptEnvironment, usize) {
    let mut h = host(one(build));
    let mut e = env();
    let (frames, status) = run(&mut e, &mut h, 200);
    assert_eq!(status, FrameStatus::Finished { callback: false });
    (h, e, frames)
}

fn var(h: &mut RecordingHost, v: u16) -> u16 {
    h.vars_flags().var(v).unwrap()
}

fn flag(h: &mut RecordingHost, f: u16) -> bool {
    h.vars_flags().flag(f)
}

fn set_var(h: &mut RecordingHost, v: u16, value: u16) {
    assert!(h.vars_flags().set_var(v, value));
}

fn set_flag(h: &mut RecordingHost, f: u16) {
    assert!(h.vars_flags().set_flag(f));
}

/// A context over `bank` positioned at script 0, with the same three
/// messages decoded — for stepping single commands without the driver.
fn context(bank: Vec<u8>) -> ScriptContext {
    let messages = vec![vec![A, EOS], vec![A, A + 1, EOS], vec![A, A + 1, A + 2, EOS]];
    let mut ctx = ScriptContext::new(ScriptBank::parse(bank).unwrap(), Some(messages), 846, 546);
    ctx.run_by_index(0).unwrap();
    ctx
}

// ---------------------------------------------------------------------
// control flow, registers, comparisons
// ---------------------------------------------------------------------

#[test]
fn registers_compare_branch_call_and_return() {
    let (mut h, e, frames) = run_one(|a| {
        a.op(Op::LoadByte).b(0).b(5);
        a.op(Op::LoadWord).b(1).w(0x1122_3344);
        a.op(Op::CopyLocal).b(2).b(1);
        a.op(Op::CompareLocalToValue).b(0).b(5); // eq
        let taken = a.op(Op::GoToIf).b(1).fixup(); // GoToIfEq
        a.op(Op::SetVar).h(V0).h(99); // skipped
        let after = a.here();
        a.patch(taken, after);
        a.op(Op::SetVar).h(V1).h(7);
        a.op(Op::CompareLocalToLocal).b(1).b(2); // 0x44 == 0x44 (truncated to u8)
        let not_taken = a.op(Op::CallIf).b(5).fixup(); // CallIfNe: not taken
        let call = a.op(Op::Call).fixup();
        a.op(Op::SetVar).h(V2).h(1);
        a.op(Op::End);
        let sub = a.here();
        a.patch(call, sub).patch(not_taken, sub);
        a.op(Op::AddVar).h(V1).h(3); // literal addend
        a.op(Op::Return);
    });
    assert_eq!(frames, 1, "no command yields");
    assert_eq!(var(&mut h, V0), 0);
    assert_eq!(var(&mut h, V1), 10);
    assert_eq!(var(&mut h, V2), 1);
    assert_eq!(e.active_count(), 0);
    assert!(h.events_without_polls().is_empty());
}

#[test]
fn backward_goto_loops_and_var_to_var_compare() {
    let (mut h, _, _) = run_one(|a| {
        a.op(Op::SetVar).h(V0).h(3);
        a.op(Op::SetVar).h(V2).h(0);
        let top = a.here();
        a.op(Op::SubVar).h(V0).h(1);
        a.op(Op::AddVar).h(V1).h(V0); // V1 += V0 (a var addend)
        a.op(Op::CompareVarToVar).h(V0).h(V2);
        a.op(Op::GoToIf).b(5).rel(top); // GoToIfNe
        a.op(Op::End);
    });
    assert_eq!(var(&mut h, V0), 0);
    assert_eq!(var(&mut h, V1), 2 + 1);
}

#[test]
fn condition_table_covers_all_six_conditions() {
    // For each (condition, value) pair, compare V0 = 5 against `value`
    // and branch to a SetVar V1 = 1.
    for (condition, value, expect) in [
        (0u8, 6u16, true),  // lt
        (0, 5, false),
        (1, 5, true), // eq
        (1, 4, false),
        (2, 4, true), // gt
        (2, 5, false),
        (3, 5, true), // le
        (3, 4, false),
        (4, 5, true), // ge
        (4, 6, false),
        (5, 4, true), // ne
        (5, 5, false),
    ] {
        let (mut h, _, _) = run_one(|a| {
            a.op(Op::SetVar).h(V0).h(5);
            a.op(Op::CompareVarToValue).h(V0).h(value);
            let f = a.op(Op::GoToIf).b(condition).fixup();
            a.op(Op::End);
            let target = a.here();
            a.patch(f, target);
            a.op(Op::SetVar).h(V1).h(1);
            a.op(Op::End);
        });
        assert_eq!(var(&mut h, V1) == 1, expect, "condition {condition} vs {value}");
    }
}

#[test]
fn object_and_direction_gotos_use_the_environment() {
    let bank = one(|a| {
        let obj = a.op(Op::ObjectGoTo).b(6).fixup(); // not the last-interacted
        let dir_ = a.op(Op::DirectionGoTo).b(dir::EAST as u8).fixup();
        a.op(Op::SetVar).h(V0).h(1);
        a.op(Op::End);
        let t = a.here();
        a.patch(dir_, t);
        a.op(Op::SetVar).h(V1).h(1);
        let obj2 = a.op(Op::ObjectGoTo).b(5).fixup();
        a.op(Op::End);
        let t2 = a.here();
        a.patch(obj, t2).patch(obj2, t2);
        a.op(Op::SetVar).h(V2).h(1);
        a.op(Op::End);
    });
    let mut h = host(bank);
    let mut e = ScriptEnvironment::new();
    e.setup(1, Some(5), dir::EAST);
    assert_eq!(e.special_var(0xD), Some(5), "VAR_SPECIAL_LAST_TALKED");
    assert_eq!(e.last_interacted(), Some(5));
    run(&mut e, &mut h, 5);
    assert_eq!(var(&mut h, V0), 0);
    assert_eq!(var(&mut h, V1), 1);
    assert_eq!(var(&mut h, V2), 1);
}

#[test]
fn nop_and_dummies_do_nothing() {
    let (_, _, frames) = run_one(|a| {
        a.op(Op::Nop).op(Op::Dummy).op(Op::Dummy486).op(Op::End);
    });
    assert_eq!(frames, 1);
}

// ---------------------------------------------------------------------
// flags and variables
// ---------------------------------------------------------------------

#[test]
fn flags_saved_temporary_and_trainer() {
    let (mut h, e, _) = run_one(|a| {
        a.op(Op::SetFlag).h(0x27E);
        a.op(Op::CheckFlag).h(0x27E);
        let f = a.op(Op::GoToIf).b(1).fixup(); // set → comparison 1 → eq
        a.op(Op::End);
        let t = a.here();
        a.patch(f, t);
        a.op(Op::ClearFlag).h(0x27E);
        a.op(Op::SetFlag).h(0x4001); // temporary
        a.op(Op::SetVar).h(V0).h(0x123);
        a.op(Op::SetFlagVar).h(V0);
        a.op(Op::CheckFlagVar).h(V0).h(V1);
        a.op(Op::ClearFlagVar).h(V0);
        a.op(Op::CheckFlagVar).h(V0).h(V2);
        a.op(Op::SetTrainerFlag).h(5);
        a.op(Op::CheckTrainerFlag).h(5);
        let g = a.op(Op::GoToIf).b(1).fixup();
        a.op(Op::End);
        let t = a.here();
        a.patch(g, t);
        a.op(Op::SetVar).h(RESULT).h(1);
        a.op(Op::ClearTrainerFlag).h(5);
        a.op(Op::End);
    });
    assert!(!flag(&mut h, 0x27E));
    assert!(h.temp_flags.flag(0x4001));
    assert!(!flag(&mut h, 0x4001), "temporaries never reach the save");
    assert_eq!(var(&mut h, V1), 1);
    assert_eq!(var(&mut h, V2), 0);
    assert!(!flag(&mut h, 0x123));
    assert!(!flag(&mut h, TRAINER_FLAG_BASE + 5));
    assert_eq!(e.special_var(0xC), Some(1));
}

#[test]
fn variables_special_literal_and_copies() {
    let (mut h, e, _) = run_one(|a| {
        a.op(Op::SetVar).h(RESULT).h(9);
        a.op(Op::CopyVar).h(V0).h(RESULT);
        a.op(Op::SetOrCopyVar).h(V1).h(V0); // a variable source
        a.op(Op::SetOrCopyVar).h(V2).h(42); // a literal source
        a.op(Op::SubVar).h(V2).h(43); // wraps
        a.op(Op::SetStarterChoice).h(155);
        a.op(Op::End);
    });
    assert_eq!(var(&mut h, V0), 9);
    assert_eq!(var(&mut h, V1), 9);
    assert_eq!(var(&mut h, V2), 0xFFFF);
    assert_eq!(e.special_var(0xC), Some(9));
    assert_eq!(var(&mut h, VAR_PLAYER_STARTER), 155);
    assert_eq!(e.var_get(&mut h, 7), Some(7), "literals read as themselves");
    assert_eq!(e.var_get(&mut h, 0x800E), None, "past the special range");
}

#[test]
fn random_draws_the_field_rng_and_yields() {
    let mut h = host(one(|a| {
        a.op(Op::Random).h(V0).h(10);
        a.op(Op::End);
    }));
    h.rng = Lcrng::new(0xC0FFEE);
    let mut expected = Lcrng::new(0xC0FFEE);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 5);
    assert_eq!(frames, 2, "Random returns TRUE");
    assert_eq!(var(&mut h, V0), expected.next_u16() % 10);
    assert_eq!(h.rng, expected);
}

#[test]
fn unresolvable_variables_stop_the_context() {
    let mut h = host(one(|a| {
        a.op(Op::SetVar).h(0x3000).h(1);
        a.op(Op::End);
    }));
    let mut e = env();
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::BadVar {
            var: 0x3000,
            offset: 6
        })
    );
    assert_eq!(e.active_count(), 0);
}

// ---------------------------------------------------------------------
// waits: the pause timer, input
// ---------------------------------------------------------------------

#[test]
fn wait_counts_frames_through_the_named_variable() {
    let mut h = host(one(|a| {
        a.op(Op::Wait).h(3).h(RESULT);
        a.op(Op::End);
    }));
    let mut e = env();
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.special_var(0xC), Some(3));
    assert_eq!(e.context(0).unwrap().native(), Some(&NativeWait::PauseTimer));
    for left in [2, 1, 0] {
        assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
        assert_eq!(e.special_var(0xC), Some(left));
    }
    // The wait held on the frame it hit zero; bytecode resumes next frame.
    assert_eq!(e.context(0).unwrap().mode(), Mode::Bytecode);
    assert_eq!(
        e.run_frame(&mut h).unwrap(),
        FrameStatus::Finished { callback: false }
    );
}

#[test]
fn button_waits_read_new_keys() {
    // WaitButton: A/B, or a d-pad press that also turns the player.
    let mut h = host(one(|a| {
        a.op(Op::WaitButton).op(Op::End);
    }));
    let mut e = env();
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    h.keys = Keys(key::LEFT);
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(h.actions(), vec![&FieldAction::SetPlayerFacing(dir::WEST)]);
    h.keys = Keys::IDLE;
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Finished { callback: false });

    // WaitButtonOrDpad: any of them, no turn.
    let mut h = host(one(|a| {
        a.op(Op::WaitButtonOrDpad).op(Op::End);
    }));
    let mut e = env();
    e.run_frame(&mut h).unwrap();
    h.keys = Keys(key::UP);
    e.run_frame(&mut h).unwrap();
    assert!(h.actions().is_empty());
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Finished { callback: false });

    // WaitABPress ignores the d-pad.
    let mut h = host(one(|a| {
        a.op(Op::WaitABPress).op(Op::End);
    }));
    let mut e = env();
    e.run_frame(&mut h).unwrap();
    h.keys = Keys(key::UP);
    e.run_frame(&mut h).unwrap();
    assert_eq!(e.context(0).unwrap().mode(), Mode::Native);
    h.keys = Keys(key::B);
    e.run_frame(&mut h).unwrap();
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Finished { callback: false });

    // WaitButtonOrDelay: `--data[0] == 0` without a press.
    let mut h = host(one(|a| {
        a.op(Op::WaitButtonOrDelay).h(2).op(Op::End);
    }));
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 4);
    let mut h = host(one(|a| {
        a.op(Op::WaitButtonOrDelay).h(100).op(Op::End);
    }));
    h.keys = Keys(key::A);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 3);
}

// ---------------------------------------------------------------------
// the dialogue window and messages
// ---------------------------------------------------------------------

#[test]
fn npc_msg_opens_the_window_once_prints_and_waits() {
    let mut h = host(one(|a| {
        a.op(Op::BufferPlayersName).b(0);
        a.op(Op::NPCMsg).b(2);
        a.op(Op::NPCMsg).b(0);
        a.op(Op::CloseMsg);
        a.op(Op::End);
    }));
    h.player_name = GameString::from_units(&[A + 6, A + 14, A + 11, A + 3]);
    h.pending.insert(WaitFor::PrintFinished, 2);
    h.queries.insert(FieldQuery::TextFrameDelay, 4);
    let mut e = env();
    // Frame 1: the name is buffered, the window opens, "ABC" prints.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.message_format().field(0).unwrap().units(), h.player_name.units());
    assert!(e.window_open() && e.textbox_open());
    assert_eq!(e.last_message().units(), &[A, A + 1, A + 2]);
    let ev = h.events_without_polls();
    assert_eq!(ev.len(), 2);
    assert_eq!(*ev[0], HostEvent::DialogOpen);
    let HostEvent::Print {
        target,
        text,
        params,
    } = ev[1]
    else {
        panic!("{:?}", ev[1]);
    };
    assert_eq!(*target, PrintTarget::Dialog);
    assert_eq!(text.units(), &[A, A + 1, A + 2]);
    assert_eq!((params.font, params.frame_delay, params.can_ab_speed_up), (1, 4, true));
    assert!(!params.instant);
    // Two pending polls, the third completes, then the second print
    // without a second open.
    e.run_frame(&mut h).unwrap();
    e.run_frame(&mut h).unwrap();
    assert_eq!(e.context(0).unwrap().mode(), Mode::Native);
    e.run_frame(&mut h).unwrap();
    assert_eq!(e.context(0).unwrap().mode(), Mode::Bytecode);
    e.run_frame(&mut h).unwrap();
    assert_eq!(h.prints().len(), 2);
    assert_eq!(
        h.events.iter().filter(|e| **e == HostEvent::DialogOpen).count(),
        1
    );
    let (_, status) = run(&mut e, &mut h, 5);
    assert_eq!(status, FrameStatus::Finished { callback: false });
    assert_eq!(h.events.last(), Some(&HostEvent::DialogClose));
    assert!(!e.window_open() && !e.textbox_open());
}

#[test]
fn open_hold_and_message_variants() {
    let mut h = host(one(|a| {
        a.op(Op::OpenMsg);
        a.op(Op::SetVar).h(V0).h(0x101); // NPCMsgVar truncates to u8 → 1
        a.op(Op::NPCMsgVar).h(V0);
        a.op(Op::NonNPCMsgVar).h(2);
        a.op(Op::GenderMsgBox).b(0).b(1);
        a.op(Op::HoldMsg);
        a.op(Op::End);
    }));
    h.queries.insert(FieldQuery::PlayerGender, 1);
    let mut e = env();
    run(&mut e, &mut h, 20);
    let prints: Vec<usize> = h.prints().iter().map(|t| t.len()).collect();
    assert_eq!(prints, vec![2, 3, 2], "message 1, 2, then the female one");
    let params: Vec<bool> = h
        .events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Print { params, .. } => Some(params.can_ab_speed_up),
            _ => None,
        })
        .collect();
    assert_eq!(params, vec![false, true, true]);
    assert_eq!(
        h.events.iter().filter(|e| **e == HostEvent::DialogOpen).count(),
        1,
        "OpenMsg created the window; the prints reused it"
    );
    assert!(h.actions().contains(&&FieldAction::DialogHold));
    assert!(!e.window_open());
}

#[test]
fn extern_messages_and_std_bank_lookup() {
    let mut h = host(one(|a| {
        a.op(Op::GetStdMsgNaix).h(0).h(V0); // 752
        a.op(Op::GetStdMsgNaix).h(9).h(V1); // out of range → 0
        a.op(Op::MsgBoxExtern).h(V0).h(1);
        a.op(Op::End);
    }));
    h.messages.insert(752, msg_bank(&[&[A], &[A + 25, A + 25, A + 25, A + 25]]));
    let mut e = env();
    run(&mut e, &mut h, 10);
    assert_eq!(var(&mut h, V0), 752);
    assert_eq!(var(&mut h, V1), 0);
    assert_eq!(h.prints()[0].units(), &[A + 25; 4]);

    // A bank the host does not have is an error, not a panic.
    let mut h = host(one(|a| {
        a.op(Op::MsgBoxExtern).h(30).h(0).op(Op::End);
    }));
    let mut e = env();
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::MessagesMissing { bank: 30 })
    );
    // A message past the bank likewise.
    let mut h = host(one(|a| {
        a.op(Op::NPCMsg).b(7).op(Op::End);
    }));
    let mut e = env();
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::NoSuchMessage { bank: 546, id: 7 })
    );
}

#[test]
fn yes_no_writes_the_choice() {
    let mut h = host(one(|a| {
        a.op(Op::YesNo).h(RESULT).op(Op::End);
    }));
    h.pending.insert(WaitFor::YesNo, 1);
    h.poll_values.insert(WaitFor::YesNo, 1);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 4);
    assert_eq!(h.actions(), vec![&FieldAction::YesNoOpen]);
    assert_eq!(e.special_var(0xC), Some(1), "1 = no");
}

// ---------------------------------------------------------------------
// signposts
// ---------------------------------------------------------------------

#[test]
fn direction_signpost_prints_instantly_and_wait_signpost_dismisses() {
    let mut h = host(one(|a| {
        a.op(Op::DirectionSignpost).b(0).b(3).h(60).h(RESULT);
        a.op(Op::WaitSignpost).h(RESULT);
        a.op(Op::End);
    }));
    let mut e = env();
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert!(e.textbox_open() && !e.window_open(), "signposts never create the dialogue window");
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::SignpostSet { kind: 3, map: 60 },
            &FieldAction::SignpostCommand(1),
            &FieldAction::SignpostDoCurrent,
        ]
    );
    let HostEvent::Print { target, params, .. } = h.events.last().unwrap() else {
        panic!()
    };
    assert_eq!(*target, PrintTarget::Signpost);
    assert!(params.instant);
    assert_eq!(params.color, Some([2, 10, 15]));
    // DirectionSignpost yields without a native: WaitSignpost runs next frame.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.context(0).unwrap().native(), Some(&NativeWait::WaitSignpost));
    h.keys = Keys(key::DOWN);
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.special_var(0xC), Some(0));
    assert!(!e.textbox_open());
    assert_eq!(h.actions().last(), Some(&&FieldAction::SetPlayerFacing(dir::SOUTH)));
}

#[test]
fn trainer_tips_finishes_by_print_or_dpad() {
    let mut h = host(one(|a| {
        a.op(Op::SetSignpostMap).b(1).h(2);
        a.op(Op::SetSignpostAction).b(4);
        a.op(Op::WaitSignpostAction);
        a.op(Op::TrainerTips).b(1).h(RESULT);
        a.op(Op::End);
    }));
    h.pending.insert(WaitFor::PrintFinished, 1);
    let mut e = env();
    let (_, status) = run(&mut e, &mut h, 20);
    assert_eq!(status, FrameStatus::Finished { callback: false });
    assert_eq!(e.special_var(0xC), Some(2), "the print finished");
    assert!(h.actions().contains(&&FieldAction::SignpostCommand(4)));
    assert!(h.events.contains(&HostEvent::Poll(WaitFor::SignpostCommandFinished)));

    let mut h = host(one(|a| {
        a.op(Op::TrainerTips).b(1).h(RESULT).op(Op::End);
    }));
    h.pending.insert(WaitFor::PrintFinished, 100);
    let mut e = env();
    e.run_frame(&mut h).unwrap();
    h.keys = Keys(key::RIGHT);
    e.run_frame(&mut h).unwrap();
    assert_eq!(e.special_var(0xC), Some(0), "cancelled by the d-pad");
    assert!(h.actions().contains(&&FieldAction::RemoveTextPrinter));
    assert!(h.actions().contains(&&FieldAction::SetPlayerFacing(dir::EAST)));
}

// ---------------------------------------------------------------------
// sound
// ---------------------------------------------------------------------

#[test]
fn sound_commands_record_and_wait() {
    let (h, _, _) = run_one(|a| {
        a.op(Op::PlayBGM).h(1);
        a.op(Op::StopBGM).h(0);
        a.op(Op::ResetBGM);
        a.op(Op::FadeOutBGM).h(2).h(30);
        a.op(Op::FadeInBGM).h(20);
        a.op(Op::TempBGM).h(3);
        a.op(Op::SetVar).h(V0).h(1234);
        a.op(Op::PlaySE).h(V0);
        a.op(Op::WaitSE).h(V0);
        a.op(Op::PlayCry).h(0).h(152);
        a.op(Op::WaitCry);
        a.op(Op::PlayFanfare).h(99);
        a.op(Op::WaitFanfare);
        a.op(Op::End);
    });
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::PlayBgm(1),
            &FieldAction::StopBgm,
            &FieldAction::ResetBgm,
            &FieldAction::FadeOutBgm { seq: 2, length: 30 },
            &FieldAction::FadeInBgm(20),
            &FieldAction::TempBgm(3),
            &FieldAction::PlaySe(1234),
            &FieldAction::PlayCry {
                species: 152,
                form: 0
            },
            &FieldAction::PlayFanfare(99),
        ]
    );
    let polls: Vec<WaitFor> = h
        .events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Poll(w) => Some(*w),
            _ => None,
        })
        .collect();
    assert_eq!(
        polls,
        vec![
            WaitFor::BgmFadeFinished,
            WaitFor::BgmFadeFinished,
            WaitFor::SeFinished(1234),
            WaitFor::CryFinished,
            WaitFor::FanfareFinished,
        ]
    );
}

// ---------------------------------------------------------------------
// objects and movement
// ---------------------------------------------------------------------

#[test]
fn apply_movement_decodes_the_list_and_wait_movement_polls() {
    let mut h = host(one(|a| {
        let list = a.op(Op::ApplyMovement).h(OBJ_PLAYER).fixup();
        let list2 = a.op(Op::ApplyMovement).h(9).fixup(); // a missing object: still FALSE
        a.op(Op::WaitMovement);
        a.op(Op::End);
        let at = a.here();
        a.patch(list, at).patch(list2, at);
        a.movement(&[(12, 3), (4, 1)]);
    }));
    h.pending.insert(WaitFor::MovementFinished, 2);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 5, "two pending polls, one that completes, End");
    let HostEvent::Movement { object, steps } = &h.events[0] else {
        panic!()
    };
    assert_eq!(*object, OBJ_PLAYER);
    assert_eq!(
        steps,
        &vec![
            MovementCommand {
                command: 12,
                length: 3
            },
            MovementCommand {
                command: 4,
                length: 1
            },
            MovementCommand {
                command: 254,
                length: 0
            },
        ]
    );
    assert!(matches!(h.events[1], HostEvent::Movement { object: 9, .. }));
}

#[test]
fn lock_release_and_object_queries() {
    let mut h = host(one(|a| {
        a.op(Op::LockAll);
        a.op(Op::Lock).h(3);
        a.op(Op::Release).h(3);
        a.op(Op::ShowPerson).h(4);
        a.op(Op::HidePerson).h(4);
        a.op(Op::FacePlayer);
        a.op(Op::GetPlayerCoords).h(V0).h(V1);
        a.op(Op::GetPersonCoords).h(8).h(V2).h(RESULT);
        a.op(Op::GetPlayerFacing).h(0x8000);
        a.op(Op::MovePersonFacing).h(4).h(10).h(0).h(20).h(dir::WEST);
        a.op(Op::ReleaseAll);
        a.op(Op::End);
    }));
    h.player_position = (6, 7);
    h.queries.insert(FieldQuery::PlayerFacing, dir::NORTH.into());
    h.action_results.push_back(1); // LockAll: wait for the pause dance
    h.pending.insert(WaitFor::LockSettled, 1);
    let mut e = ScriptEnvironment::new();
    e.setup(1, Some(2), dir::SOUTH);
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 5, "LockAll yields, two polls; ReleaseAll yields; End");
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::LockAll {
                last_interacted: Some(2)
            },
            &FieldAction::LockObject(3),
            &FieldAction::ReleaseObject(3),
            &FieldAction::ShowObject(4),
            &FieldAction::HideObject(4),
            &FieldAction::FacePlayer { object: 2 },
            &FieldAction::SetObjectPosition {
                object: 4,
                x: 10,
                y: 0,
                z: 20,
                direction: dir::WEST
            },
            &FieldAction::ReleaseAll,
        ]
    );
    assert_eq!((var(&mut h, V0), var(&mut h, V1)), (6, 7));
    assert_eq!((var(&mut h, V2), e.special_var(0xC)), (255, Some(255)), "no object 8");
    assert_eq!(e.special_var(0), Some(dir::NORTH));

    // Without a last-interacted object FacePlayer is a no-op and LockAll
    // does not wait when the host says so.
    let (h, _, frames) = run_one(|a| {
        a.op(Op::LockAll).op(Op::FacePlayer).op(Op::End);
    });
    assert_eq!(frames, 2);
    assert_eq!(
        h.actions(),
        vec![&FieldAction::LockAll {
            last_interacted: None
        }]
    );
}

// ---------------------------------------------------------------------
// items, party, money, misc queries
// ---------------------------------------------------------------------

#[test]
fn item_commands_go_through_the_bag() {
    let mut h = host(one(|a| {
        a.op(Op::GiveItem).h(5).h(2).h(V0);
        a.op(Op::TakeItem).h(5).h(1).h(V1);
        a.op(Op::HasSpaceForItem).h(5).h(1).h(V2);
        a.op(Op::HasItem).h(5).h(1).h(RESULT);
        a.op(Op::GetItemPocket).h(5).h(0x8000);
        a.op(Op::End);
    }));
    h.action_results.extend([1, 0]);
    h.queries.insert(FieldQuery::BagHasSpace { item: 5, quantity: 1 }, 1);
    h.queries.insert(FieldQuery::BagHasItem { item: 5, quantity: 1 }, 1);
    h.queries.insert(FieldQuery::ItemPocket(5), 3);
    let mut e = env();
    run(&mut e, &mut h, 5);
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::BagAdd {
                item: 5,
                quantity: 2
            },
            &FieldAction::BagTake {
                item: 5,
                quantity: 1
            },
        ]
    );
    assert_eq!((var(&mut h, V0), var(&mut h, V1), var(&mut h, V2)), (1, 0, 1));
    assert_eq!((e.special_var(0xC), e.special_var(0)), (Some(1), Some(3)));
}

#[test]
fn party_and_player_queries() {
    let mut h = host(one(|a| {
        a.op(Op::GetPlayerGender).h(V0);
        a.op(Op::GetFriendSprite).h(V1);
        a.op(Op::GetPartyCount).h(V2);
        a.op(Op::GetPartyMonSpecies).h(0).h(0x8000);
        a.op(Op::GetPartyMonSpecies).h(1).h(0x8001); // an egg
        a.op(Op::GetPartyMonSpecies).h(5).h(0x8002); // no such slot
        a.op(Op::MonGetFriendship).h(0x8003).h(0);
        a.op(Op::GetPartyMonForm2).h(0).h(0x8004);
        a.op(Op::MonHasRibbon).h(0x8005).h(0).h(7);
        a.op(Op::GiveRibbon).h(0).h(7);
        a.op(Op::GetPartyLeadAlive).h(0x8006);
        a.op(Op::CheckBadge).h(1).h(0x8007);
        a.op(Op::HealParty);
        a.op(Op::End);
    }));
    h.queries.insert(FieldQuery::PlayerGender, 1);
    h.queries.insert(FieldQuery::PartyCount, 2);
    h.queries.insert(FieldQuery::MonHasRibbon { slot: 0, ribbon: 7 }, 1);
    h.queries.insert(FieldQuery::PartyLeadAlive, 1);
    h.queries.insert(FieldQuery::HasBadge(1), 1);
    h.party = vec![
        PartyMon {
            species: 155,
            form: 2,
            is_egg: false,
            friendship: 70,
            nickname: GameString::from_units(&[A]),
        },
        PartyMon {
            species: 172,
            form: 0,
            is_egg: true,
            friendship: 0,
            nickname: GameString::new(),
        },
    ];
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 5);
    assert_eq!(frames, 2, "GetFriendSprite returns TRUE");
    assert_eq!((var(&mut h, V0), var(&mut h, V1), var(&mut h, V2)), (1, 0, 2));
    let specials: Vec<u16> = (0..8).map(|i| e.special_var(i).unwrap()).collect();
    assert_eq!(specials, vec![155, 0, 0, 70, 2, 1, 1, 1]);
    assert!(h.actions().contains(&&FieldAction::GiveRibbon { slot: 0, ribbon: 7 }));
    assert!(h.actions().contains(&&FieldAction::HealParty));
    // A male player gets the heroine's sprite.
    let (mut h, _, _) = run_one(|a| {
        a.op(Op::GetFriendSprite).h(V1).op(Op::End);
    });
    assert_eq!(var(&mut h, V1), 97);
}

#[test]
fn money_time_version_photo_and_follower_queries() {
    let mut h = host(one(|a| {
        a.op(Op::HasEnoughMoneyVar).h(V0).h(50);
        a.op(Op::HasEnoughMoneyVar).h(V1).h(150);
        a.op(Op::Cmd377).h(V2);
        a.op(Op::Cmd379).h(0x8000);
        a.op(Op::GetWeekday).h(0x8001);
        a.op(Op::GetGameVersion).h(0x8002);
        a.op(Op::PhotoAlbumIsFull).h(0x8003);
        a.op(Op::CheckBankBalance).h(0x8004).w(70_000);
        a.op(Op::BankOrWalletIsFull).h(0).h(0x8005);
        a.op(Op::BankOrWalletIsFull).h(1).h(0x8006);
        a.op(Op::Cmd729).h(0x8007);
        a.op(Op::Cmd596).h(0x8008);
        a.op(Op::Cmd600);
        a.op(Op::LotoIDSet);
        a.op(Op::End);
    }));
    h.queries.insert(FieldQuery::Money, 100);
    h.queries.insert(FieldQuery::MailboxCount, 4);
    h.queries.insert(FieldQuery::TimeOfDay, 3);
    h.queries.insert(FieldQuery::Weekday, 2);
    h.queries.insert(FieldQuery::PhotoCount, 36);
    h.queries.insert(FieldQuery::BankBalance, 999_999);
    h.queries.insert(FieldQuery::FollowMonActive, 1);
    h.queries.insert(FieldQuery::FollowMonState596, 5);
    h.queries.insert(FieldQuery::FollowMonState600, 1);
    h.rng = Lcrng::new(1);
    let mut expected = Lcrng::new(1);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 10);
    assert_eq!(frames, 4, "BankOrWalletIsFull ×2 and Cmd600 yield");
    assert_eq!((var(&mut h, V0), var(&mut h, V1), var(&mut h, V2)), (1, 0, 4));
    let specials: Vec<u16> = (0..9).map(|i| e.special_var(i).unwrap()).collect();
    assert_eq!(specials, vec![3, 2, 7, 1, 1, 1, 0, 1, 5]);
    let _lo = expected.next_u16();
    let hi = expected.next_u16();
    assert_eq!(var(&mut h, VAR_LOTO_NUMBER_LO), hi, "retail writes both halves to LO");
    assert_eq!(h.rng, expected);
}

// ---------------------------------------------------------------------
// placeholder buffers
// ---------------------------------------------------------------------

#[test]
fn buffer_commands_fill_message_format_fields() {
    let mut h = host(one(|a| {
        a.op(Op::BufferPlayersName).b(0);
        a.op(Op::BufferRivalsName).b(1);
        a.op(Op::BufferFriendsName).b(2);
        a.op(Op::BufferMonSpeciesName).b(3).h(0);
        a.op(Op::BufferPartyMonSpeciesNameIndef).b(4).h(0);
        a.op(Op::BufferPartyMonNick).b(5).h(0);
        a.op(Op::BufferItemName).b(6).h(1);
        a.op(Op::BufferItemNamePlural).b(7).h(1);
        a.op(Op::BufferPocketName).b(0).h(2);
        a.op(Op::End);
    }));
    h.player_name = GameString::from_units(&[A + 15]);
    h.rival_name = GameString::from_units(&[A + 17]);
    h.party = vec![PartyMon {
        species: 1,
        form: 0,
        is_egg: false,
        friendship: 0,
        nickname: GameString::from_units(&[A + 13]),
    }];
    h.messages.insert(445, msg_bank(&[&[A + 4], &[A + 11]])); // Ethan, Lyra
    h.messages.insert(237, msg_bank(&[&[A], &[A + 1]]));
    h.messages.insert(238, msg_bank(&[&[A], &[A + 2]]));
    h.messages.insert(222, msg_bank(&[&[A], &[A + 8]]));
    h.messages.insert(224, msg_bank(&[&[A], &[A + 8, A + 18]]));
    h.messages.insert(226, msg_bank(&[&[A], &[A], &[A + 15, A + 14]]));
    let mut e = env();
    run(&mut e, &mut h, 5);
    let field = |n: usize| e.message_format().field(n).unwrap().units().to_vec();
    assert_eq!(field(1), vec![A + 17]);
    assert_eq!(field(2), vec![A + 11], "a male player's friend is Lyra");
    assert_eq!(field(3), vec![A + 1]);
    assert_eq!(field(4), vec![A + 2]);
    assert_eq!(field(5), vec![A + 13]);
    assert_eq!(field(6), vec![A + 8]);
    assert_eq!(field(7), vec![A + 8, A + 18]);
    assert_eq!(field(0), vec![A + 15, A + 14], "overwritten by the pocket name");
}

// ---------------------------------------------------------------------
// applications, child tasks, fades, warps
// ---------------------------------------------------------------------

#[test]
fn applications_hand_control_to_the_host_and_take_a_result() {
    let mut h = host(one(|a| {
        a.op(Op::NameRival).h(RESULT);
        a.op(Op::NicknameInput).h(0).h(V0);
        a.op(Op::ChooseStarter);
        a.op(Op::CatchingTutorial);
        a.op(Op::Cmd376);
        a.op(Op::End);
    }));
    h.party = vec![PartyMon {
        species: 158,
        form: 0,
        is_egg: false,
        friendship: 0,
        nickname: GameString::from_units(&[A + 19]),
    }];
    h.poll_values.insert(WaitFor::App, 1);
    h.pending.insert(WaitFor::App, 1);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 30);
    assert_eq!(frames, 12, "five launches, the first with a pending poll");
    let launches: Vec<&AppRequest> = h
        .events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Launch(app) => Some(app),
            _ => None,
        })
        .collect();
    assert_eq!(launches.len(), 5);
    assert!(matches!(
        launches[0],
        AppRequest::NamingScreen {
            kind: 1,
            max_len: 7,
            ..
        }
    ));
    let AppRequest::NamingScreen {
        kind: 2,
        species: 158,
        max_len: 10,
        party_slot: 0,
        initial,
    } = launches[1]
    else {
        panic!("{:?}", launches[1]);
    };
    assert_eq!(initial.units(), &[A + 19]);
    assert_eq!(launches[2], &AppRequest::ChooseStarter);
    assert_eq!(launches[3], &AppRequest::CatchingTutorial);
    assert_eq!(launches[4], &AppRequest::Mail);
    assert_eq!(e.special_var(0xC), Some(1));
    assert_eq!(var(&mut h, V0), 1);
}

#[test]
fn child_tasks_suspend_the_script_until_they_return() {
    let mut h = host(one(|a| {
        a.op(Op::FadeScreen).h(6).h(1).h(0).h(0);
        a.op(Op::WaitFade);
        a.op(Op::Warp).h(60).h(0xFFFF).h(4).h(5).h(dir::NORTH);
        a.op(Op::RestoreOverworld);
        a.op(Op::Cmd436);
        a.op(Op::CameronPhoto).h(3);
        a.op(Op::Cmd582).h(61).h(8).h(9);
        a.op(Op::End);
    }));
    h.pending.insert(WaitFor::ChildTask, 1);
    let mut e = env();
    // Frame 1: FadeScreen, WaitFade installs its native wait. Frame 2:
    // the fade poll holds; bytecode resumes next frame (RunScriptCommand
    // returns TRUE after flipping the mode). Frame 3: Warp pushes the
    // child task — the C returns TRUE with the context still in
    // BYTECODE mode, and Task_RunScripts is not the active task.
    for _ in 0..3 {
        assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    }
    assert!(e.child_task_pending());
    assert_eq!(e.context(0).unwrap().mode(), Mode::Bytecode);
    assert_eq!(e.context(0).unwrap().native(), None);
    assert_eq!(h.actions().len(), 2, "FadeScreen and Warp");
    // Frame 4: the child is still up (the mock's one pending poll) —
    // nothing runs. Frame 5: the child returns and, as
    // FieldSystem_RunTaskFrame pops to the parent in the same frame,
    // RestoreOverworld runs at once and pushes the next child.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert!(e.child_task_pending());
    assert_eq!(h.actions().len(), 2, "no command ran while the child task was up");
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(h.actions().len(), 3, "the frame the child returned ran RestoreOverworld");
    assert!(e.child_task_pending());
    // Frames 6, 7: Cmd436 and CameronPhoto each run the frame the
    // previous child returns; frame 8: Cmd582 and End.
    let (frames, status) = run(&mut e, &mut h, 30);
    assert_eq!(frames, 3);
    assert_eq!(status, FrameStatus::Finished { callback: false });
    assert!(!e.child_task_pending());
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::FadeScreen {
                duration: 6,
                speed: 1,
                kind: 0,
                color: 0
            },
            &FieldAction::Warp {
                map: 60,
                x: 4,
                y: 5,
                direction: dir::NORTH
            },
            &FieldAction::RestoreOverworld,
            &FieldAction::LeaveOverworld,
            &FieldAction::TakePhoto(3),
            &FieldAction::SetSpecialSpawn { map: 61, x: 8, y: 9 },
        ]
    );
    assert_eq!(
        h.events.iter().filter(|e| **e == HostEvent::Poll(WaitFor::ChildTask)).count(),
        5
    );
    assert!(h.events.contains(&HostEvent::Poll(WaitFor::FadeFinished)));
}

// ---------------------------------------------------------------------
// the phone, the follower, overlay-1 effects, Elm's lab
// ---------------------------------------------------------------------

#[test]
fn phone_follower_and_effect_commands() {
    let (h, _, frames) = run_one(|a| {
        a.op(Op::RegisterGearNumber).h(74);
        a.op(Op::RegisterGearNumber).h(75); // out of range: nothing
        a.op(Op::UnsetPhoneCallTrigger).b(2);
        a.op(Op::ToggleFollowingPokemonMovement).h(1); // inactive: nothing
        a.op(Op::WaitFollowingPokemonMovement); // yields
        a.op(Op::FollowingPokemonMovement).h(3); // yields
        a.op(Op::Cmd605).b(1).b(2);
        a.op(Op::Cmd608);
        a.op(Op::Cmd609); // yields
        a.op(Op::Cmd307).h(1).h(2).h(3).h(4).b(5);
        a.op(Op::Cmd308).b(1); // yields
        a.op(Op::Cmd309).b(2).op(Op::Cmd310).b(3).op(Op::Cmd311).b(4);
        a.op(Op::End);
    });
    assert_eq!(frames, 5);
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::RegisterPhoneNumber(74),
            &FieldAction::ClearPhoneCallTrigger(2),
            &FieldAction::Effect307 {
                x: 3 + 32,
                y: 4 + 64,
                kind: 5
            },
            &FieldAction::Effect308(1),
            &FieldAction::Effect309(2),
            &FieldAction::Effect310(3),
            &FieldAction::Effect311(4),
        ]
    );

    // With a follower active the same commands reach it.
    let mut h = host(one(|a| {
        a.op(Op::ToggleFollowingPokemonMovement).h(1);
        a.op(Op::WaitFollowingPokemonMovement);
        a.op(Op::FollowingPokemonMovement).h(3);
        a.op(Op::Cmd605).b(1).b(2);
        a.op(Op::Cmd608);
        a.op(Op::Cmd609);
        a.op(Op::ToggleFollowingPokemonMovement).h(0);
        a.op(Op::End);
    }));
    h.queries.insert(FieldQuery::FollowMonActive, 1);
    h.pending.insert(WaitFor::FollowMonPaused, 1);
    let mut e = env();
    run(&mut e, &mut h, 20);
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::FollowMonPause(true),
            &FieldAction::FollowMonMovement(3),
            &FieldAction::FollowMonEffect605 { a: 1, b: 2 },
            &FieldAction::FollowMonEffect608,
            &FieldAction::FollowMonEffect609,
            &FieldAction::FollowMonPause(false),
        ]
    );
    assert!(h.events.contains(&HostEvent::Poll(WaitFor::FollowMonPaused)));
}

#[test]
fn starter_balls_depend_on_story_flags_and_the_party() {
    let balls = |setup: &dyn Fn(&mut RecordingHost)| {
        let mut h = host(one(|a| {
            a.op(Op::PlaceStarterBallsInElmsLab).op(Op::End);
        }));
        setup(&mut h);
        let mut e = env();
        run(&mut e, &mut h, 5);
        h.actions()
            .iter()
            .filter_map(|a| match a {
                FieldAction::LoadMapProp { prop: 0x8D, x, z } => Some((*x, *z)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(balls(&|_| {}), vec![(131, 65), (141, 65), (136, 72)]);
    assert_eq!(
        balls(&|h| {
            h.queries.insert(FieldQuery::PartyCount, 1);
        })
        .len(),
        2
    );
    assert_eq!(balls(&|h| set_flag(h, 0x99)).len(), 1);
    assert_eq!(balls(&|h| set_flag(h, 0x73)).len(), 0);
}

// ---------------------------------------------------------------------
// menus, the money box, Mom's savings, scene control
// ---------------------------------------------------------------------

#[test]
fn touchscreen_menu_and_menu_choice() {
    let mut h = host(one(|a| {
        a.op(Op::TouchscreenMenuHide);
        a.op(Op::TouchscreenMenuShow);
        a.op(Op::TouchscreenMenuHide);
        a.op(Op::GetMenuChoice).h(RESULT);
        a.op(Op::End);
    }));
    h.poll_values.insert(WaitFor::MenuChoice, 2);
    let mut e = env();
    run(&mut e, &mut h, 20);
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::TouchscreenMenu(3),
            &FieldAction::TouchscreenMenu(0),
            &FieldAction::TouchscreenMenu(3),
            &FieldAction::MenuChoiceBegin,
        ]
    );
    assert_eq!(e.special_var(0xC), Some(2));
    // Already hidden: nothing to do.
    let mut h = host(one(|a| {
        a.op(Op::TouchscreenMenuHide).op(Op::End);
    }));
    h.queries.insert(FieldQuery::TouchscreenMenuMode, 3);
    let mut e = env();
    let (frames, _) = run(&mut e, &mut h, 5);
    assert_eq!(frames, 1);
    assert!(h.actions().is_empty());
}

#[test]
fn list_menu_result_goes_to_the_menu_init_variable() {
    let mut h = host(one(|a| {
        a.op(Op::MenuInit).b(1).b(1).b(0).b(1).h(RESULT);
        a.op(Op::MenuItemAdd).h(1).h(255).h(0);
        a.op(Op::MenuItemAdd).h(2).h(255).h(1);
        a.op(Op::MenuExec);
        a.op(Op::End);
    }));
    h.poll_values.insert(WaitFor::MenuExec, 1);
    let mut e = env();
    // sub_02041770 ends with `ctx->data[0] = var`: after MenuInit's
    // frame the register names the result variable for MenuExec.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.context(0).unwrap().registers()[0], u32::from(RESULT));
    run(&mut e, &mut h, 20);
    assert_eq!(e.special_var(0xC), Some(1));
    let actions = h.actions();
    assert_eq!(
        actions[0],
        &FieldAction::MenuInit {
            x: 1,
            y: 1,
            cursor: 0,
            cancellable: 1,
            result_var: RESULT
        }
    );
    let FieldAction::MenuAddItem {
        text,
        position: 255,
        value: 1,
    } = actions[2]
    else {
        panic!("{:?}", actions[2]);
    };
    assert_eq!(text.units(), &[A, A + 1, A + 2]);
    assert_eq!(actions[3], &FieldAction::MenuExec);
}

#[test]
fn list_menu_result_goes_to_a_saved_variable_too() {
    let mut h = host(one(|a| {
        a.op(Op::MenuInit).b(1).b(1).b(0).b(1).h(V1);
        a.op(Op::MenuItemAdd).h(1).h(255).h(0);
        a.op(Op::MenuExec);
        a.op(Op::End);
    }));
    h.poll_values.insert(WaitFor::MenuExec, 7);
    let mut e = env();
    run(&mut e, &mut h, 20);
    assert_eq!(var(&mut h, V1), 7);
    assert_eq!(e.special_var(0xC), Some(0), "VAR_SPECIAL_RESULT untouched");
}

#[test]
fn menu_exec_without_a_menu_init_variable_is_an_error() {
    // ScrCmd_MenuExec resolves `GetVarPointer(data[0])`; a register
    // no MenuInit filled (0) is no variable — NULL in the C.
    let mut h = host(one(|a| {
        a.op(Op::MenuExec);
        a.op(Op::End);
    }));
    let mut e = env();
    let err = e.run_frame(&mut h).unwrap_err();
    assert!(matches!(err, ScriptError::BadVar { var: 0, .. }), "{err:?}");
    assert!(h.actions().is_empty(), "the menu never ran");
}

#[test]
fn bank_transaction_money_box_and_end_callback() {
    let bank = one(|a| {
        a.op(Op::Cmd795).h(1).h(2);
        a.op(Op::BankTransaction).h(0).h(RESULT);
        a.op(Op::Cmd796);
        a.op(Op::Cmd061);
        a.op(Op::End);
    });
    let mut h = host(bank.clone());
    h.poll_values.insert(WaitFor::BankTransaction, 1);
    // MAPSEC_NEW_BARK_TOWN (126): not the Mystery Zone, so ScrCmd_061
    // arms scrctx_end_cb.
    h.queries.insert(FieldQuery::MapSec, 126);
    let mut e = env();
    let (_, status) = run(&mut e, &mut h, 20);
    assert_eq!(status, FrameStatus::Finished { callback: true });
    assert!(e.end_callback_armed());
    assert_eq!(
        h.actions(),
        vec![
            &FieldAction::MoneyBoxShow { x: 1, y: 2 },
            &FieldAction::BankTransaction { mode: 0 },
            &FieldAction::MoneyBoxHide,
        ]
    );
    assert_eq!(e.special_var(0xC), Some(1));

    // In the Mystery Zone (MAPSEC_MYSTERY_ZONE, 0 — the mock's default
    // answer) sub_0204031C leaves the callback alone.
    let mut h = host(bank);
    h.poll_values.insert(WaitFor::BankTransaction, 1);
    let mut e = env();
    let (_, status) = run(&mut e, &mut h, 20);
    assert_eq!(status, FrameStatus::Finished { callback: false });
    assert!(!e.end_callback_armed());
}

// ---------------------------------------------------------------------
// CallStd and the three contexts
// ---------------------------------------------------------------------

#[test]
fn call_std_spawns_a_context_that_releases_its_caller() {
    // Map script: CallStd std_misc + 0, then read what it left.
    let mut h = host(one(|a| {
        a.op(Op::CallStd).h(2000);
        a.op(Op::CopyVar).h(V1).h(V0);
        a.op(Op::End);
    }));
    let mut std = Asm::new(1);
    std.entry();
    std.op(Op::SetVar).h(V0).h(7);
    std.op(Op::RestartCurrentScript);
    std.op(Op::SetVar).h(V2).h(1); // the callee keeps running after releasing
    std.op(Op::End);
    h.scripts.insert(3, std.finish());
    let mut e = env();
    // Frame 1: the caller spawns slot 1 and waits; the callee runs in the
    // same frame to its end and is destroyed.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.active_count(), 1);
    assert!(e.context(1).is_none());
    assert_eq!(e.context(0).unwrap().native(), Some(&NativeWait::WaitStd));
    assert_eq!(var(&mut h, V0), 7);
    assert_eq!(var(&mut h, V2), 1);
    // Frame 2: the wait sees its bit cleared; frame 3: the caller finishes.
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Running);
    assert_eq!(e.run_frame(&mut h).unwrap(), FrameStatus::Finished { callback: false });
    assert_eq!(var(&mut h, V1), 7);
}

#[test]
fn a_fourth_context_is_refused() {
    // Every script CallStds std_misc + 0, which CallStds it again...
    let mut h = host(one(|a| {
        a.op(Op::CallStd).h(2000).op(Op::End);
    }));
    let mut std = Asm::new(1);
    std.entry();
    std.op(Op::CallStd).h(2000).op(Op::End);
    h.scripts.insert(3, std.finish());
    let mut e = env();
    assert_eq!(e.run_frame(&mut h), Err(ScriptError::TooManyContexts));
    assert_eq!(e.active_count(), 2, "the two live callers keep their slots");
}

// ---------------------------------------------------------------------
// the map-load runner and errors
// ---------------------------------------------------------------------

#[test]
fn map_load_scripts_run_synchronously_across_yields() {
    let mut h = host(one(|a| {
        a.op(Op::GetFriendSprite).h(V0); // yields
        a.op(Op::SetFlag).h(0x27E);
        a.op(Op::End);
    }));
    let mut e = ScriptEnvironment::new();
    assert_eq!(e.run_map_load_script(&mut h, 1, 100), Ok(2));
    assert!(flag(&mut h, 0x27E));
    assert_eq!(var(&mut h, V0), 97);
    assert_eq!(e.active_script(), 1);

    let mut h = host(one(|a| {
        let top = a.here();
        a.op(Op::Random).h(V0).h(3);
        a.op(Op::GoTo).rel(top);
    }));
    let mut e = ScriptEnvironment::new();
    assert_eq!(
        e.run_map_load_script(&mut h, 1, 50),
        Err(ScriptError::Runaway { steps: 50 })
    );
}

#[test]
fn unknown_unimplemented_and_truncated_commands_are_errors_not_panics() {
    let mut h = host(one(|a| {
        a.op(Op::LoadByteFromAddr).b(0).w(0x0200_0000);
    }));
    let mut e = env();
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::Unimplemented {
            opcode: Op::LoadByteFromAddr,
            offset: 6
        })
    );
    assert_eq!(e.active_count(), 0);

    let mut h = host(one(|a| {
        a.raw(900);
    }));
    let mut e = env();
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::UnknownOpcode {
            opcode: 900,
            offset: 6
        })
    );

    let mut h = host(one(|a| {
        a.op(Op::SetVar).h(V0); // the value is missing
    }));
    let mut e = env();
    assert_eq!(e.run_frame(&mut h), Err(ScriptError::Truncated { offset: 10 }));

    // A bank the host does not have.
    let mut h = RecordingHost::new();
    h.map_banks = MAP;
    let mut e = env();
    assert_eq!(e.run_frame(&mut h), Err(ScriptError::BankMissing { bank: 846 }));
    // A script index past the table.
    let mut h = host(one(|a| {
        a.op(Op::End);
    }));
    let mut e = ScriptEnvironment::new();
    e.setup(2, None, 0);
    assert_eq!(
        e.run_frame(&mut h),
        Err(ScriptError::NoSuchScript { index: 1, count: 1 })
    );
}

#[test]
fn a_context_can_be_stepped_directly() {
    let mut ctx = context(one(|a| {
        a.op(Op::NPCMsg).b(1).op(Op::CloseMsg).op(Op::End);
    }));
    let mut e = ScriptEnvironment::new();
    let mut h = RecordingHost::new();
    assert_eq!(ctx.step(&mut e, &mut h), Ok(true));
    assert_eq!(ctx.mode(), Mode::Native);
    assert_eq!(ctx.native(), Some(&NativeWait::Poll(WaitFor::PrintFinished)));
    assert_eq!(ctx.step(&mut e, &mut h), Ok(true));
    assert_eq!(ctx.mode(), Mode::Bytecode);
    assert_eq!(ctx.step(&mut e, &mut h), Ok(false));
    assert_eq!(ctx.mode(), Mode::Stopped);
    assert_eq!(ctx.step(&mut e, &mut h), Ok(false), "stopped stays stopped");
    assert_eq!(h.prints()[0].units(), &[A, A + 1]);
    assert_eq!(ctx.comparison(), comparison::LESS);
    assert_eq!(ctx.registers(), &[0; 4]);
    assert!(ctx.stack().is_empty());
}

#[test]
fn implemented_set_is_consistent() {
    let listed = implemented_opcodes();
    assert!(listed.len() >= 140);
    assert!(listed.windows(2).all(|w| w[0] < w[1]));
    assert!(listed.iter().all(|&op| is_implemented(op)));
    assert!(!is_implemented(Op::LoadByteFromAddr));
    assert!(is_implemented(Op::End) && is_implemented(Op::BufferDeptStoreFloorNo) == false);
    // Every listed opcode decodes with its declared layout.
    for op in listed {
        let _ = op.operands();
    }
    let _ = set_var;
}
