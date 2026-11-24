//! Windows x64 image analysis: locates the player array, stat block, and name
//! field by decoding the routines that index the selected character.
use std::{io, sync::atomic::AtomicBool};

use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};

use super::{Array, Candidates, cancelled, scalar};

fn text_section(blob: &[u8]) -> Option<(usize, &[u8])> {
    if blob.get(..2)? != b"MZ" {
        return None;
    }
    let pe = scalar(blob, 0x3c, 4)?;
    if blob.get(pe..pe + 4)? != b"PE\0\0" || scalar(blob, pe + 4, 2)? != 0x8664 {
        return None;
    }
    let count = scalar(blob, pe + 6, 2)?;
    if !(1..=96).contains(&count) {
        return None;
    }
    let start = pe + 24 + scalar(blob, pe + 20, 2)?;
    let mut sections = blob
        .get(start..start + 40 * count)?
        .chunks_exact(40)
        .filter(|section| section.starts_with(b".text\0\0\0"));
    let section = sections.next()?;
    if sections.next().is_some() {
        return None;
    }
    let rva = scalar(section, 12, 4)?;
    let length = scalar(section, 16, 4)?;
    let offset = scalar(section, 20, 4)?;
    Some((rva, blob.get(offset..offset + length)?))
}

fn register(instruction: &Instruction, index: u32) -> Option<Register> {
    (instruction.op_count() > index && instruction.op_kind(index) == OpKind::Register)
        .then(|| instruction.op_register(index))
}

fn load_register(instruction: &Instruction) -> Option<Register> {
    let destination = register(instruction, 0)?;
    (instruction.mnemonic() == Mnemonic::Mov
        && instruction.op1_kind() == OpKind::Memory
        && destination.is_gpr64()
        && instruction.memory_size().size() == 8)
        .then_some(destination)
}

fn load(instruction: &Instruction) -> Option<(Register, Register, usize)> {
    let destination = load_register(instruction)?;
    if instruction.memory_index() != Register::None {
        return None;
    }
    Some((
        destination,
        instruction.memory_base(),
        instruction.memory_displacement64() as usize,
    ))
}

#[derive(Clone, Copy)]
struct Root {
    array: Array,
    pointer: Register,
    scaled: Register,
    hero: Register,
}

impl Root {
    fn field(self, instruction: &Instruction) -> Option<usize> {
        let base = instruction.memory_base();
        let index = instruction.memory_index();
        ((base == self.hero && index == Register::None)
            || (instruction.memory_index_scale() == 1
                && ((base == self.pointer && index == self.scaled)
                    || (base == self.scaled && index == self.pointer))))
            .then(|| instruction.memory_displacement64() as usize)
    }
}

fn root(instructions: &[Instruction]) -> Option<Root> {
    let [a, b, compare, branch, pointer, multiply, lea] = instructions else {
        return None;
    };
    let (selected, _, index) = load(a)?;
    let (count_reg, game, count) = load(b)?;
    let (pointer_reg, pointer_game, pointer) = load(pointer)?;
    if pointer_game != game
        || selected == count_reg
        || selected == pointer_reg
        || [selected, count_reg].contains(&game)
        || count != pointer + 8
        || !(0x100..index).contains(&pointer)
        || index >= 0x10000
        || compare.mnemonic() != Mnemonic::Cmp
        || register(compare, 0)? != selected
        || register(compare, 1)? != count_reg
        || branch.mnemonic() != Mnemonic::Jae
        || multiply.mnemonic() != Mnemonic::Imul
        || register(multiply, 1)? != selected
        || multiply.op2_kind() != OpKind::Immediate32to64
        || lea.mnemonic() != Mnemonic::Lea
        || lea.op1_kind() != OpKind::Memory
    {
        return None;
    }
    let stride = multiply.immediate32to64() as usize;
    let scaled = register(multiply, 0)?;
    let mut root = Root {
        array: [index, count, pointer, stride],
        pointer: pointer_reg,
        scaled,
        hero: Register::None,
    };
    if !(0x200..=0x10000).contains(&stride)
        || !stride.is_multiple_of(8)
        || scaled == pointer_reg
        || root.field(lea)? != 0
    {
        return None;
    }
    root.hero = register(lea, 0)?;
    Some(root)
}

fn instructions(bytes: &[u8], rva: usize, limit: usize) -> impl Iterator<Item = Instruction> + '_ {
    Decoder::with_ip(64, bytes, rva as u64, DecoderOptions::NONE)
        .into_iter()
        .take_while(|instruction| !instruction.is_invalid())
        .filter(|instruction| instruction.mnemonic() != Mnemonic::Nop)
        .take(limit)
}

fn vector_offset(root: Root, vectors: &[Instruction], branch: &Instruction) -> Option<usize> {
    let mut offsets: Vec<_> = vectors
        .iter()
        .map(|instruction| {
            if !matches!(
                instruction.mnemonic(),
                Mnemonic::Movups | Mnemonic::Movaps | Mnemonic::Movdqu | Mnemonic::Movdqa
            ) || instruction.op1_kind() != OpKind::Memory
                || instruction.memory_size().size() != 16
            {
                return None;
            }
            root.field(instruction)
        })
        .collect::<Option<_>>()?;
    offsets.sort_unstable();
    let first = *offsets.first()?;
    (offsets.len() == 4
        && (0x160..=root.array[3] - 64).contains(&first)
        && first.is_multiple_of(4)
        && offsets == [first, first + 16, first + 32, first + 48]
        && branch.near_branch_target() >= vectors.last()?.next_ip())
    .then_some(first)
}

fn name_check(root: Root, instruction: &Instruction) -> Option<usize> {
    if instruction.mnemonic() == Mnemonic::Cmp
        && instruction.op0_kind() == OpKind::Memory
        && instruction.memory_size().size() == 8
        && instruction.try_immediate(1).ok()? as i64 == -1
    {
        return root.field(instruction);
    }
    None
}

fn name_drop(root: Root, instructions: &[Instruction]) -> Option<usize> {
    let [capacity, cmp, branch, test, empty, pointer, call, ..] = instructions else {
        return None;
    };
    let reg = load_register(capacity)?;
    let offset = root.field(capacity)?;
    if cmp.mnemonic() != Mnemonic::Cmp
        || register(cmp, 0)? != reg
        || cmp.try_immediate(1).ok()? as i64 != -1
        || branch.mnemonic() != Mnemonic::Je
        || test.mnemonic() != Mnemonic::Test
        || register(test, 0)? != reg
        || register(test, 1)? != reg
        || empty.mnemonic() != Mnemonic::Je
        || empty.near_branch_target() != branch.near_branch_target()
        || load_register(pointer)? != Register::RCX
        || pointer.memory_base() == reg
        || pointer.memory_index() == reg
        || root.field(pointer)? != offset.checked_add(8)?
        || call.mnemonic() != Mnemonic::Call
        || empty.near_branch_target() != call.next_ip()
    {
        return None;
    }
    Some(offset)
}

pub(super) fn scan(blob: &[u8], stop: &AtomicBool) -> io::Result<Candidates> {
    let (text_rva, text) = text_section(blob).ok_or_else(|| {
        io::Error::other("Expected a Windows x64 game image with one code section.")
    })?;
    let mut found = Candidates::default();
    for (offset, prefix) in text.windows(3).enumerate() {
        if offset % 0x40000 == 0 {
            cancelled(stop)?;
        }
        if !matches!(prefix, [0x48..=0x4f, 0x8b, 0x80..=0xbf]) {
            continue;
        }
        let start = text_rva + offset;
        let bytes = &text[offset..text.len().min(offset + 176)];
        let mut decoder = instructions(bytes, start, 15);
        let head: Vec<_> = decoder.by_ref().take(7).collect();
        let Some(root) = root(&head) else { continue };
        let tail: Vec<_> = decoder.collect();
        let guard = |end: u64| (start, bytes[..end as usize - start].to_vec());
        if let Some(stat) = tail.get(..4).and_then(|v| vector_offset(root, v, &head[3])) {
            found
                .arrays
                .entry((root.array, stat))
                .or_default()
                .push(guard(tail[3].next_ip()));
        }
        if let Some(name) = tail.first().and_then(|i| name_check(root, i)) {
            found
                .checks
                .entry((root.array, name))
                .or_default()
                .push(guard(tail[0].next_ip()));
        }
        if let Some(name) = name_drop(root, &tail) {
            found
                .drops
                .entry((root.array, name))
                .or_default()
                .push(guard(tail[6].next_ip()));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocated_array_requires_both_independent_identity_routines() {
        let mut image = vec![0; 0x400];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c] = 0x80;
        image[0x80..0x88].copy_from_slice(b"PE\0\0\x64\x86\x01\x00");
        image[0x98..0xa0].copy_from_slice(b".text\0\0\0");
        image[0xa4..0xb0].copy_from_slice(&[0, 0x10, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0]);
        let header = b"\x48\x8b\x89\x38\x0e\0\0\x49\x8b\x95\x90\x0b\0\0\x48\x39\xd1\x0f\x83\x64\0\0\0\x49\x8b\x95\x88\x0b\0\0\x48\x69\xc9\x58\x18\0\0\x48\x8d\x1c\x0a";
        let vectors = b"\x0f\x10\x84\x0a\x40\x17\0\0\xf3\x0f\x6f\x8c\x0a\x50\x17\0\0\xf3\x0f\x6f\x94\x0a\x60\x17\0\0\xf3\x0f\x6f\x9c\x0a\x70\x17\0\0";
        let check = b"\x48\x83\xbb\xd8\x11\0\0\xff";
        let drop = b"\x48\x8b\x83\xd8\x11\0\0\x48\x83\xf8\xff\x74\x11\x48\x85\xc0\x74\x0c\x48\x8b\x8b\xe0\x11\0\0\xe8\0\0\0\0";
        for (offset, tail) in [
            (0x200, vectors.as_slice()),
            (0x280, check.as_slice()),
            (0x300, drop.as_slice()),
        ] {
            image[offset..offset + header.len()].copy_from_slice(header);
            image[offset + header.len()..offset + header.len() + tail.len()].copy_from_slice(tail);
        }
        let stop = AtomicBool::new(false);
        let plan = super::super::analyze(&image, &stop).unwrap();
        assert_eq!(plan.array, [0xE38, 0xB90, 0xB88, 0x1858]);
        assert_eq!(
            (plan.name, plan.stats, plan.guards.len()),
            (0x11D8, 0x1740, 3)
        );
        image[0x300] = 0;
        assert!(
            super::super::analyze(&image, &stop)
                .unwrap_err()
                .to_string()
                .contains("Player identity unavailable")
        );
    }

    #[test]
    fn identity_drop_accepts_indexed_capacity_and_keeps_the_hero_alias() {
        // RAX initially holds the vector pointer and RCX the scaled index.
        // The capacity load replaces RAX, but RBX still holds the hero.
        let owner = Root {
            array: [0x12A0, 0xDC0, 0xDB8, 0x1A90],
            pointer: Register::RAX,
            scaled: Register::RCX,
            hero: Register::RBX,
        };
        let bytes = b"\x48\x8b\x84\x08\x10\x13\0\0\x48\x83\xf8\xff\x74\x11\x48\x85\xc0\x74\x0c\x48\x8b\x8b\x18\x13\0\0\xe8\0\0\0\0";
        let routine: Vec<_> = instructions(bytes, 0x1000, 7).collect();
        assert_eq!(name_drop(owner, &routine), Some(0x1310));

        let clobbered = b"\x48\x8b\x84\x08\x10\x13\0\0\x48\x83\xf8\xff\x74\x12\x48\x85\xc0\x74\x0d\x48\x8b\x8c\x08\x18\x13\0\0\xe8\0\0\0\0";
        let routine: Vec<_> = instructions(clobbered, 0x1000, 7).collect();
        assert_eq!(name_drop(owner, &routine), None);
    }
}
