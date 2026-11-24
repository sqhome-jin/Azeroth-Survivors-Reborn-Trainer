//! macOS arm64 image analysis: locates the player array, stat block, and name
//! field by decoding the routines that index the selected character. The
//! patterns were taken from the arm64 game build and mirror the x86 ones:
//! the bounds-checked index (`root`), the critical strike base of 8 that
//! follows the stat block, and the name string clone.
use std::{io, sync::atomic::AtomicBool};

use super::{Array, Candidates, cancelled, scalar};

pub(super) const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_SEGMENT_64: usize = 0x19;
const HEADER_SIZE: usize = 32;

/// Returns the `__text` section as `(offset from the image base, bytes)`.
/// The image base is the `__TEXT` segment address, which is what the running
/// process reports as the module base.
pub(super) fn text_section(blob: &[u8]) -> Option<(usize, &[u8])> {
    if scalar(blob, 0, 4)? != MH_MAGIC_64 as usize {
        return None;
    }
    let commands = scalar(blob, 16, 4)?;
    let commands_size = scalar(blob, 20, 4)?;
    if !(1..=512).contains(&commands) || commands_size > blob.len() {
        return None;
    }
    let mut cursor = HEADER_SIZE;
    let mut text = None;
    for _ in 0..commands {
        let command = scalar(blob, cursor, 4)?;
        let size = scalar(blob, cursor + 4, 4)?;
        if size < 8 || cursor + size > HEADER_SIZE + commands_size {
            return None;
        }
        if command == LC_SEGMENT_64 && blob.get(cursor + 8..cursor + 16)? == b"__TEXT\0\0" {
            let segment_address = scalar(blob, cursor + 24, 8)?;
            let sections = scalar(blob, cursor + 64, 4)?;
            let mut section = cursor + 72;
            for _ in 0..sections {
                if blob.get(section..section + 16)? == b"__text\0\0\0\0\0\0\0\0\0\0" {
                    let address = scalar(blob, section + 32, 8)?;
                    let length = scalar(blob, section + 40, 8)?;
                    let offset = scalar(blob, section + 48, 4)?;
                    if text.is_some() {
                        return None;
                    }
                    text = Some((
                        address.checked_sub(segment_address)?,
                        blob.get(offset..offset.checked_add(length)?)?,
                    ));
                }
                section += 80;
            }
        }
        cursor += size;
    }
    text
}

/// One decoded arm64 instruction form that the scan cares about. Register
/// numbers are the 5-bit fields; 31 is `xzr`/`wzr` or `sp` depending on use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// `ldr xT, [xN, #imm]` with the byte offset already scaled.
    LoadX {
        t: u8,
        n: u8,
        offset: usize,
    },
    /// `ldr wT, [xN, #imm]` with the byte offset already scaled.
    LoadW {
        t: u8,
        n: u8,
        offset: usize,
    },
    /// `cmp xN, xM` (`subs xzr, xN, xM` without a shift).
    Compare {
        n: u8,
        m: u8,
    },
    /// `cmn xN, #imm` (`adds xzr, xN, #imm`).
    CompareNegative {
        n: u8,
        imm: usize,
    },
    /// `add wD, wN, #imm` without a shift.
    AddW {
        d: u8,
        n: u8,
        imm: usize,
    },
    /// `b.<cond>` with the ARM condition code.
    Branch {
        cond: u8,
    },
    /// `cbz xT, …`.
    BranchIfZero {
        t: u8,
    },
    /// `mov xD, #imm` (`movz` without a shift, either register width).
    MoveImmediate {
        d: u8,
        imm: usize,
    },
    /// `mov xD, xM` (`orr xD, xzr, xM`).
    MoveRegister {
        d: u8,
        m: u8,
    },
    /// `madd xD, xN, xM, xA`.
    MultiplyAdd {
        d: u8,
        n: u8,
        m: u8,
        a: u8,
    },
    Call,
    Other(u32),
}

impl Op {
    fn decode(word: u32) -> Self {
        let rt = (word & 31) as u8;
        let rn = ((word >> 5) & 31) as u8;
        let rm = ((word >> 16) & 31) as u8;
        let imm12 = ((word >> 10) & 0xfff) as usize;
        match word {
            _ if word & 0xffc0_0000 == 0xf940_0000 => Op::LoadX {
                t: rt,
                n: rn,
                offset: imm12 * 8,
            },
            _ if word & 0xffc0_0000 == 0xb940_0000 => Op::LoadW {
                t: rt,
                n: rn,
                offset: imm12 * 4,
            },
            _ if word & 0xffe0_fc1f == 0xeb00_001f => Op::Compare { n: rn, m: rm },
            _ if word & 0xffc0_001f == 0xb100_001f => Op::CompareNegative { n: rn, imm: imm12 },
            _ if word & 0xffc0_0000 == 0x1100_0000 => Op::AddW {
                d: rt,
                n: rn,
                imm: imm12,
            },
            _ if word & 0xff00_0010 == 0x5400_0000 => Op::Branch {
                cond: (word & 15) as u8,
            },
            _ if word & 0xff00_0000 == 0xb400_0000 => Op::BranchIfZero { t: rt },
            _ if word & 0x7f80_0000 == 0x5280_0000 => Op::MoveImmediate {
                d: rt,
                imm: ((word >> 5) & 0xffff) as usize,
            },
            _ if word & 0xffe0_ffe0 == 0xaa00_03e0 => Op::MoveRegister { d: rt, m: rm },
            _ if word & 0xffe0_8000 == 0x9b00_0000 => Op::MultiplyAdd {
                d: rt,
                n: rn,
                m: rm,
                a: ((word >> 10) & 31) as u8,
            },
            _ if word & 0xfc00_0000 == 0x9400_0000 => Op::Call,
            _ => Op::Other(word),
        }
    }

    /// The general-purpose register this instruction writes, when it is one of
    /// the forms whose low bits name a destination. Branches, compares, and
    /// calls write none; unknown encodings are treated as writing their low
    /// bits so interleaved instructions can never silently clobber a tracked
    /// register.
    fn writes(self) -> Option<u8> {
        match self {
            Op::LoadX { t, .. } | Op::LoadW { t, .. } => Some(t),
            Op::AddW { d, .. }
            | Op::MoveImmediate { d, .. }
            | Op::MoveRegister { d, .. }
            | Op::MultiplyAdd { d, .. } => Some(d),
            Op::Compare { .. }
            | Op::CompareNegative { .. }
            | Op::Branch { .. }
            | Op::BranchIfZero { .. }
            | Op::Call => None,
            Op::Other(word) => Some((word & 31) as u8),
        }
    }
}

const CONDITION_EQ: u8 = 0;
const CONDITION_HS: u8 = 2;

#[derive(Clone, Copy)]
struct Root {
    array: Array,
    hero: u8,
    /// Index of the instruction after `madd`, relative to the first `ldr`.
    length: usize,
}

/// Matches the bounds-checked index into the player array:
/// `ldr idx; ldr count; cmp idx, count; b.hs; ldr ptr; mov stride; madd hero`.
/// Unrelated instructions may be interleaved as long as they do not write a
/// tracked register.
fn root(ops: &[Op], previous: Option<Op>) -> Option<Root> {
    let [
        Op::LoadX {
            t: idx,
            n: game,
            offset: index,
        },
        rest @ ..,
    ] = ops
    else {
        return None;
    };
    let (idx, game) = (*idx, *game);
    // Some routines save the Game pointer immediately before loading the
    // index into its original register.
    let game = match previous {
        Some(Op::MoveRegister { d, m }) if idx == game && m == game && d != game => d,
        _ => game,
    };
    if game == 31 || idx == 31 || idx == game || *index < 0x100 || *index >= 0x10000 {
        return None;
    }
    let mut tracked = vec![idx, game];
    let mut cursor = 0;
    let mut next = |tracked: &[u8], want: &mut dyn FnMut(Op) -> Option<()>| -> Option<usize> {
        while cursor < rest.len() && cursor < 10 {
            let op = rest[cursor];
            cursor += 1;
            if want(op).is_some() {
                return Some(cursor);
            }
            if op.writes().is_some_and(|reg| tracked.contains(&reg)) {
                return None;
            }
        }
        None
    };
    let mut count_reg = 0;
    let mut count = 0;
    next(&tracked, &mut |op| match op {
        Op::LoadX { t, n, offset } if n == game && t != idx && t != game => {
            count_reg = t;
            count = offset;
            Some(())
        }
        _ => None,
    })?;
    tracked.push(count_reg);
    next(&tracked, &mut |op| {
        (op == Op::Compare {
            n: idx,
            m: count_reg,
        })
        .then_some(())
    })?;
    next(&tracked, &mut |op| {
        (op == Op::Branch { cond: CONDITION_HS }).then_some(())
    })?;
    let mut pointer_reg = 0;
    next(&tracked, &mut |op| match op {
        Op::LoadX { t, n, offset } if n == game && offset + 8 == count && t != idx => {
            pointer_reg = t;
            Some(())
        }
        _ => None,
    })?;
    tracked.push(pointer_reg);
    let pointer = count - 8;
    if !(0x100..*index).contains(&pointer) {
        return None;
    }
    let mut stride_reg = 0;
    let mut stride = 0;
    next(&tracked, &mut |op| match op {
        Op::MoveImmediate { d, imm }
            if (0x200..=0x10000).contains(&imm)
                && imm.is_multiple_of(8)
                && d != idx
                && d != pointer_reg
                && d != 31 =>
        {
            stride_reg = d;
            stride = imm;
            Some(())
        }
        _ => None,
    })?;
    tracked.push(stride_reg);
    let mut hero = 0;
    let length = next(&tracked, &mut |op| match op {
        Op::MultiplyAdd { d, n, m, a }
            if a == pointer_reg
                && ((n == idx && m == stride_reg) || (n == stride_reg && m == idx)) =>
        {
            hero = d;
            Some(())
        }
        _ => None,
    })?;
    Some(Root {
        array: [*index, count, pointer, stride],
        hero,
        length: length + 1,
    })
}

/// Follows `mov` copies of the hero pointer and stops when it is overwritten.
struct Hero {
    registers: Vec<u8>,
}

impl Hero {
    fn new(root: Root) -> Self {
        Self {
            registers: vec![root.hero],
        }
    }

    fn holds(&self, register: u8) -> bool {
        self.registers.contains(&register)
    }

    /// Updates the alias set and returns false once no register holds the hero.
    fn step(&mut self, op: Op) -> bool {
        if let Op::MoveRegister { d, m } = op
            && self.holds(m)
        {
            self.registers.push(d);
            return true;
        }
        if let Some(reg) = op.writes() {
            self.registers.retain(|&held| held != reg);
        }
        !self.registers.is_empty()
    }
}

/// The critical strike display adds its base of 8 right after loading the
/// slot: `ldr wT, [hero, #stats + 16]; add wT, wT, #8`. Returns the stat block
/// offset and the index just past the `add`.
fn crit_anchor(root: Root, tail: &[Op]) -> Option<(usize, usize)> {
    let mut hero = Hero::new(root);
    for (position, &op) in tail.iter().enumerate().take(12) {
        if let Op::LoadW { t, n, offset } = op
            && hero.holds(n)
            && let Some(&Op::AddW {
                d,
                n: source,
                imm: 8,
            }) = tail.get(position + 1)
            && d == t
            && source == t
        {
            let stats = offset.checked_sub(16)?;
            return (stats.is_multiple_of(4) && (0x160..=root.array[3] - 64).contains(&stats))
                .then_some((stats, position + 2));
        }
        if !hero.step(op) {
            return None;
        }
    }
    None
}

/// `ldr cap, [hero, #name]; cmn cap, #1` tests the name's empty marker.
/// Returns the name offset and the index just past the `cmn`.
fn name_check(root: Root, tail: &[Op]) -> Option<(usize, usize)> {
    let [
        Op::LoadX { t, n, offset },
        Op::CompareNegative {
            n: compared,
            imm: 1,
        },
        ..,
    ] = tail
    else {
        return None;
    };
    (*n == root.hero && compared == t).then_some((*offset, 2))
}

/// Cloning the name reads the `(capacity, pointer, length)` triple, allocates
/// `length` bytes with alignment 1, and copies. The allocation call is the
/// distinctive part: `mov x0, len; mov w1, #1; bl`.
fn name_clone(root: Root, tail: &[Op]) -> Option<(usize, usize)> {
    let (name, mut cursor) = name_check(root, tail)?;
    let cap = match tail[0] {
        Op::LoadX { t, .. } => t,
        _ => return None,
    };
    let mut hero = Hero::new(root);
    let mut saw_branch = false;
    let mut length_reg = None;
    let mut pointer_loaded = false;
    let mut saw_zero_check = false;
    let mut argument_set = false;
    let mut alignment_set = false;
    while cursor < tail.len().min(16) {
        let op = tail[cursor];
        cursor += 1;
        match op {
            Op::Branch { cond: CONDITION_EQ } if !saw_branch => saw_branch = true,
            Op::LoadX { t, n, offset } if saw_branch && hero.holds(n) && offset == name + 16 => {
                length_reg = Some(t);
            }
            Op::LoadX { n, offset, .. } if saw_branch && hero.holds(n) && offset == name + 8 => {
                pointer_loaded = true;
            }
            Op::BranchIfZero { t } if Some(t) == length_reg => saw_zero_check = true,
            Op::MoveRegister { d: 0, m } if Some(m) == length_reg && saw_zero_check => {
                argument_set = true;
            }
            Op::MoveImmediate { d: 1, imm: 1 } if saw_zero_check => alignment_set = true,
            Op::Call => {
                return (saw_branch
                    && pointer_loaded
                    && saw_zero_check
                    && argument_set
                    && alignment_set
                    && length_reg != Some(cap))
                .then_some((name, cursor));
            }
            _ => {}
        }
        if !hero.step(op) {
            return None;
        }
    }
    None
}

fn words(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
}

pub(super) fn scan(blob: &[u8], stop: &AtomicBool) -> io::Result<Candidates> {
    let (text_rva, text) = text_section(blob).ok_or_else(|| {
        io::Error::other("Expected a macOS arm64 game image with one code section.")
    })?;
    let mut found = Candidates::default();
    const WINDOW: usize = 40;
    for (index, word) in words(text).enumerate() {
        if index % 0x10000 == 0 {
            cancelled(stop)?;
        }
        // Every root starts with `ldr xT, [xN, #imm]` and a large offset.
        if word & 0xffc0_0000 != 0xf940_0000 || ((word >> 10) & 0xfff) < 0x20 {
            continue;
        }
        let offset = index * 4;
        let ops: Vec<Op> = words(&text[offset..text.len().min(offset + WINDOW * 4)])
            .map(Op::decode)
            .collect();
        let previous = offset
            .checked_sub(4)
            .and_then(|start| words(&text[start..offset]).next())
            .map(Op::decode);
        let Some(root) = root(&ops, previous) else {
            continue;
        };
        let guard_offset = match ops[0] {
            Op::LoadX { t, n, .. } if t == n => offset - 4,
            _ => offset,
        };
        let start = text_rva + guard_offset;
        let tail = &ops[root.length..];
        let guard = |end: usize| {
            (
                start,
                text[guard_offset..offset + 4 * (root.length + end)].to_vec(),
            )
        };
        if let Some((stats, end)) = crit_anchor(root, tail) {
            found
                .arrays
                .entry((root.array, stats))
                .or_default()
                .push(guard(end));
        }
        if let Some((name, end)) = name_check(root, tail) {
            found
                .checks
                .entry((root.array, name))
                .or_default()
                .push(guard(end));
        }
        if let Some((name, end)) = name_clone(root, tail) {
            found
                .drops
                .entry((root.array, name))
                .or_default()
                .push(guard(end));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Vec<u8> {
        // Header, one LC_SEGMENT_64 (__TEXT at 0x100000000) with one __text section.
        let mut blob = vec![0u8; 0x400];
        blob[..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        blob[16..20].copy_from_slice(&1u32.to_le_bytes());
        blob[20..24].copy_from_slice(&152u32.to_le_bytes());
        let segment = HEADER_SIZE;
        blob[segment..segment + 4].copy_from_slice(&(LC_SEGMENT_64 as u32).to_le_bytes());
        blob[segment + 4..segment + 8].copy_from_slice(&152u32.to_le_bytes());
        blob[segment + 8..segment + 16].copy_from_slice(b"__TEXT\0\0");
        blob[segment + 24..segment + 32].copy_from_slice(&0x1_0000_0000u64.to_le_bytes());
        blob[segment + 64..segment + 68].copy_from_slice(&1u32.to_le_bytes());
        let section = segment + 72;
        blob[section..section + 16].copy_from_slice(b"__text\0\0\0\0\0\0\0\0\0\0");
        blob[section + 32..section + 40].copy_from_slice(&0x1_0000_0200u64.to_le_bytes());
        blob[section + 40..section + 48].copy_from_slice(&0x100u64.to_le_bytes());
        blob[section + 48..section + 52].copy_from_slice(&0x200u32.to_le_bytes());
        blob[0x200..0x204].copy_from_slice(&[0xc0, 0x03, 0x5f, 0xd6]); // ret
        blob
    }

    #[test]
    fn analyzes_player_layout_from_arm64_instruction_sequences() {
        // macOS 0.7.4 disassembly. The first routine saves x0 before
        // loading the selected index into x0. The second clones the name.
        let crit = [
            0xaa0003f4u32,
            0xf949a800,
            0xf9470681,
            0xeb01001f,
            0x54001c82,
            0xf9470288,
            0x5283bc09,
            0x9b092015,
            0xb95cdaa8,
            0x11002108,
        ];
        let name = [
            0xf949aa80u32,
            0xf9470681,
            0xeb01001f,
            0x54001e82,
            0xf9470288,
            0x5283bc09,
            0x9b092008,
            0xf94b3109,
            0xb100053f,
            0x54000240,
            0xf94b3916,
            0xb40004d6,
            0xf94b3517,
            0xaa1603e0,
            0x52800021,
            0x942f17b2,
        ];
        let mut blob = image();
        for (bytes, word) in blob[0x200..]
            .chunks_exact_mut(4)
            .zip(crit.into_iter().chain(name))
        {
            bytes.copy_from_slice(&word.to_le_bytes());
        }

        let stop = AtomicBool::new(false);
        let plan = super::super::analyze(&blob, &stop).unwrap();
        assert_eq!(plan.array, [4944, 3592, 3584, 7648]);
        assert_eq!(plan.name, 5728);
        assert_eq!(plan.stats, 7368);
        assert_eq!(plan.guards[0], (0x200, blob[0x200..0x228].to_vec()));

        // Dropping the allocation call leaves a capacity check alone.
        let call = 0x200 + (crit.len() + name.len() - 1) * 4;
        blob[call..call + 4].fill(0);
        assert!(super::super::analyze(&blob, &stop).is_err());
    }

    #[test]
    fn locates_the_text_section_relative_to_the_text_segment() {
        let blob = image();
        let (offset, text) = text_section(&blob).unwrap();
        assert_eq!(offset, 0x200);
        assert_eq!(text.len(), 0x100);
        assert_eq!(&text[..4], &[0xc0, 0x03, 0x5f, 0xd6]);
        assert!(text_section(&blob[..0x100]).is_none());
        assert!(text_section(b"MZ").is_none());
    }
}
