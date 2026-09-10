//! ARM/Thumb encodings exercised by the save initializers.
use apricorn_harness::arm::{exec::Cpu, mem::Memory};
fn cpu(words: &[u32]) -> Cpu {
    let mut mem = Memory::new();
    for (i, &w) in words.iter().enumerate() {
        mem.write32(0x02000000 + i as u32 * 4, w).unwrap();
    }
    let mut cpu = Cpu::new(mem);
    cpu.jump(0x02000000);
    cpu
}
#[test]
fn cmp_r9_is_not_a_multiply() {
    let mut c = cpu(&[0xe1510009]);
    c.set_reg(1, 11);
    c.set_reg(9, 11);
    c.step().unwrap();
    assert!(c.flags().z);
}
#[test]
fn multiply_accumulate_and_long_register_order() {
    // MLA r0,r3,r0,r2; UMULL r12,r3,r4,r1; SMULLS r0,r1,r2,r3.
    let mut c = cpu(&[0xe0202093]);
    c.set_reg(0, 7);
    c.set_reg(3, 5);
    c.set_reg(2, 9);
    c.step().unwrap();
    assert_eq!(c.regs()[0], 44);
    let mut c = cpu(&[0xe083c194]);
    c.set_reg(4, u32::MAX);
    c.set_reg(1, 3);
    c.step().unwrap();
    assert_eq!(c.regs()[12], 0xfffffffd);
    assert_eq!(c.regs()[3], 2);
    let mut c = cpu(&[0xe0d10392]);
    c.set_reg(2, u32::MAX);
    c.set_reg(3, 2);
    c.step().unwrap();
    assert_eq!(&c.regs()[..2], &[0xfffffffe, u32::MAX]);
    assert!(c.flags().n);
    assert!(!c.flags().z);
}
#[test]
fn register_offset_uses_the_encoded_immediate_shift() {
    // str r3,[r0,r12,lsl #2]
    let mut c = cpu(&[0xe780310c]);
    c.set_reg(0, 0x02200000);
    c.set_reg(12, 3);
    c.set_reg(3, 0x1234);
    c.step().unwrap();
    assert_eq!(c.mem().read32(0x0220000c).unwrap(), 0x1234);
}
#[test]
fn thumb_blx_suffix_enters_word_aligned_arm_and_returns() {
    // BLX from halfword alignment 2 to ARM at +0x10.
    let mut c = cpu(&[0xf0000000, 0x0000e805, 0, 0, 0xe3a00007, 0xe12fff1e]);
    c.jump(0x02000003);
    c.step().unwrap();
    assert!(!c.is_thumb());
    assert_eq!(c.regs()[15], 0x02000010);
    assert_eq!(c.regs()[14], 0x02000007);
    c.step().unwrap();
    c.step().unwrap();
    assert!(c.is_thumb());
    assert_eq!(c.regs()[15], 0x02000006);
    assert_eq!(c.regs()[0], 7);
}
