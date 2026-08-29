//! Allocator and the memory and string routines the guest imports.
//!
//! The heap is a bump allocator over one mapped region, with a free list that
//! coalesces and gives ground back to the bump cursor. Released blocks are
//! zeroed: the guest's own code is what produces the signing key material, and
//! leaving freed key bytes lying around in guest memory would keep them
//! readable for the rest of the session.

use std::cmp::Ordering;

use super::{
    Allocation, FreeBlock, HEAP_BASE, HEAP_SIZE, PAGE_SIZE, Shim, Shims, argument, checked_size,
    read_c_string, set_result,
};
use crate::error::ClientError;
use crate::sap::unicorn::Guest;

pub fn register(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let services: &[(&[&str], Shim)] = &[
        (&["_malloc"], Shim::Malloc),
        (&["_malloc_good_size"], Shim::MallocGoodSize),
        (&["_malloc_size"], Shim::MallocSize),
        (&["_calloc"], Shim::Calloc),
        (&["_realloc", "_reallocf"], Shim::Realloc),
        (&["_free"], Shim::Free),
        (&["_memcpy", "_memmove"], Shim::Memmove),
        (&["_memset"], Shim::Memset),
        (&["___bzero"], Shim::Bzero),
        (&["___memcpy_chk"], Shim::MemcpyChk),
        (&["___memset_chk"], Shim::MemsetChk),
        (&["_memcmp"], Shim::Memcmp),
        (&["_strcmp"], Shim::Strcmp),
        (&["_strncmp"], Shim::Strncmp),
        (&["_strlen"], Shim::Strlen),
    ];

    for (names, shim) in services {
        shims.add_aliases(names, shim.clone(), guest)?;
    }

    Ok(())
}

/// Returns `None` when `shim` is not one of this module's.
pub fn handle(
    shims: &mut Shims,
    guest: &Guest<'_>,
    shim: &Shim,
) -> Option<Result<(), ClientError>> {
    Some(match shim {
        Shim::Malloc => malloc(shims, guest),
        Shim::MallocGoodSize => malloc_good_size(guest),
        Shim::MallocSize => malloc_size(shims, guest),
        Shim::Calloc => calloc(shims, guest),
        Shim::Realloc => realloc(shims, guest),
        Shim::Free => free(shims, guest),
        Shim::Memmove => memmove(guest),
        Shim::Memset => memset(guest),
        Shim::Bzero => bzero(guest),
        Shim::MemcpyChk => checked_memcpy(guest),
        Shim::MemsetChk => checked_memset(guest),
        Shim::Memcmp => memcmp(guest),
        Shim::Strcmp => strcmp(guest),
        Shim::Strncmp => strncmp(guest),
        Shim::Strlen => strlen(guest),
        _ => return None,
    })
}

fn malloc(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let size = argument(guest, 0)?;
    let address = allocate(shims, size)?;

    set_result(guest, address)
}

fn malloc_good_size(guest: &Guest<'_>) -> Result<(), ClientError> {
    let size = argument(guest, 0)?;

    set_result(guest, reserved_for(size))
}

fn malloc_size(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let address = argument(guest, 0)?;
    let size = shims
        .allocations
        .get(&address)
        .map_or(0, |allocation| allocation.reserved);

    set_result(guest, size)
}

fn calloc(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let count = argument(guest, 0)?;
    let size = argument(guest, 1)?;

    let total = count
        .checked_mul(size)
        .ok_or_else(|| ClientError::Sap("allocation size overflows".into()))?;

    let address = allocate(shims, total)?;

    if total != 0 {
        let cleared = vec![0u8; checked_size(total)?];

        if let Err(e) = guest.mem_write(address, &cleared) {
            let _ = release(shims, guest, address);

            return Err(e);
        }
    }

    set_result(guest, address)
}

fn realloc(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let old_address = argument(guest, 0)?;
    let new_size = argument(guest, 1)?;

    if old_address == 0 {
        let address = allocate(shims, new_size)?;

        return set_result(guest, address);
    }

    let old = *shims
        .allocations
        .get(&old_address)
        .ok_or_else(|| ClientError::Sap(format!("reallocate unknown pointer {old_address:#x}")))?;

    // Shrinking, or growing within the slack the reservation already has,
    // keeps the block in place.
    if new_size <= old.reserved {
        shims.allocations.insert(
            old_address,
            Allocation {
                size: new_size,
                reserved: old.reserved,
            },
        );

        return set_result(guest, old_address);
    }

    let new_address = allocate(shims, new_size)?;

    let copy = || -> Result<(), ClientError> {
        let data = guest.mem_read(old_address, checked_size(old.size)?)?;

        guest.mem_write(new_address, &data)
    };

    if let Err(e) = copy() {
        let _ = release(shims, guest, new_address);

        return Err(e);
    }

    release(shims, guest, old_address)?;

    set_result(guest, new_address)
}

fn free(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let address = argument(guest, 0)?;

    if address != 0 {
        release(shims, guest, address)?;
    }

    set_result(guest, 0)
}

fn reserved_for(size: u64) -> u64 {
    size.max(1).next_multiple_of(16)
}

fn allocate(shims: &mut Shims, size: u64) -> Result<u64, ClientError> {
    if size > super::MAX_TRANSFER {
        return Err(ClientError::Sap(format!(
            "allocation size {size} exceeds limit"
        )));
    }

    let reserved = reserved_for(size);

    // First fit over the free list, splitting anything larger than needed.
    if let Some(index) = shims
        .free_blocks
        .iter()
        .position(|block| block.size >= reserved)
    {
        let block = shims.free_blocks[index];

        if block.size == reserved {
            shims.free_blocks.remove(index);
        } else {
            shims.free_blocks[index].address += reserved;
            shims.free_blocks[index].size -= reserved;
        }

        shims
            .allocations
            .insert(block.address, Allocation { size, reserved });

        return Ok(block.address);
    }

    if shims.heap_cursor > HEAP_SIZE || reserved > HEAP_SIZE - shims.heap_cursor {
        return Err(ClientError::Sap("guest heap exhausted".into()));
    }

    let address = HEAP_BASE + shims.heap_cursor;
    shims.heap_cursor += reserved;
    shims
        .allocations
        .insert(address, Allocation { size, reserved });

    Ok(address)
}

fn release(shims: &mut Shims, guest: &Guest<'_>, address: u64) -> Result<(), ClientError> {
    let allocation = shims
        .allocations
        .remove(&address)
        .ok_or_else(|| ClientError::Sap(format!("free unknown pointer {address:#x}")))?;

    // Key material passes through this heap; do not leave it behind.
    guest.mem_write(address, &vec![0u8; checked_size(allocation.reserved)?])?;

    shims.free_blocks.push(FreeBlock {
        address,
        size: allocation.reserved,
    });

    coalesce(shims);

    Ok(())
}

/// Merges adjacent free blocks, then returns any run that reaches the bump
/// cursor back to it so a long alloc/free cycle does not exhaust the heap.
fn coalesce(shims: &mut Shims) {
    shims.free_blocks.sort_by_key(|block| block.address);

    let mut merged: Vec<FreeBlock> = Vec::with_capacity(shims.free_blocks.len());

    for block in shims.free_blocks.drain(..) {
        match merged.last_mut() {
            Some(last) if last.address + last.size == block.address => last.size += block.size,
            _ => merged.push(block),
        }
    }

    while let Some(last) = merged.last() {
        if last.address + last.size != HEAP_BASE + shims.heap_cursor {
            break;
        }

        shims.heap_cursor -= last.size;
        merged.pop();
    }

    shims.free_blocks = merged;
}

fn memmove(guest: &Guest<'_>) -> Result<(), ClientError> {
    let destination = argument(guest, 0)?;
    let source = argument(guest, 1)?;
    let length = checked_size(argument(guest, 2)?)?;

    // Reading the whole source before writing makes this a memmove: overlapping
    // ranges cannot clobber bytes that have not been copied yet.
    let data = guest.mem_read(source, length)?;
    guest.mem_write(destination, &data)?;

    set_result(guest, destination)
}

fn memset(guest: &Guest<'_>) -> Result<(), ClientError> {
    let destination = argument(guest, 0)?;
    let value = argument(guest, 1)? as u8;
    let length = checked_size(argument(guest, 2)?)?;

    guest.mem_write(destination, &vec![value; length])?;

    set_result(guest, destination)
}

fn bzero(guest: &Guest<'_>) -> Result<(), ClientError> {
    let destination = argument(guest, 0)?;
    let length = checked_size(argument(guest, 1)?)?;

    guest.mem_write(destination, &vec![0u8; length])?;

    set_result(guest, destination)
}

fn checked_memcpy(guest: &Guest<'_>) -> Result<(), ClientError> {
    if argument(guest, 2)? > argument(guest, 3)? {
        return Err(ClientError::Sap("checked copy exceeds destination".into()));
    }

    memmove(guest)
}

fn checked_memset(guest: &Guest<'_>) -> Result<(), ClientError> {
    if argument(guest, 2)? > argument(guest, 3)? {
        return Err(ClientError::Sap("checked fill exceeds destination".into()));
    }

    memset(guest)
}

fn memcmp(guest: &Guest<'_>) -> Result<(), ClientError> {
    let left = argument(guest, 0)?;
    let right = argument(guest, 1)?;
    let length = checked_size(argument(guest, 2)?)?;

    let a = guest.mem_read(left, length)?;
    let b = guest.mem_read(right, length)?;

    set_result(guest, ordering(a.cmp(&b)))
}

fn strcmp(guest: &Guest<'_>) -> Result<(), ClientError> {
    let a = read_c_string(guest, argument(guest, 0)?)?;
    let b = read_c_string(guest, argument(guest, 1)?)?;

    set_result(guest, ordering(a.as_bytes().cmp(b.as_bytes())))
}

fn strncmp(guest: &Guest<'_>) -> Result<(), ClientError> {
    let left = argument(guest, 0)?;
    let right = argument(guest, 1)?;
    let length = argument(guest, 2)?;

    checked_size(length)?;

    let mut offset = 0u64;

    while offset < length {
        let left_address = left
            .checked_add(offset)
            .ok_or_else(|| ClientError::Sap("string comparison address overflows".into()))?;
        let right_address = right
            .checked_add(offset)
            .ok_or_else(|| ClientError::Sap("string comparison address overflows".into()))?;

        // Neither string is known to be terminated, so read only up to the end
        // of the current page on each side; anything further may be unmapped.
        let chunk = (length - offset)
            .min(PAGE_SIZE - left_address % PAGE_SIZE)
            .min(PAGE_SIZE - right_address % PAGE_SIZE);

        let a = guest.mem_read(left_address, chunk as usize)?;
        let b = guest.mem_read(right_address, chunk as usize)?;

        for (left_byte, right_byte) in a.iter().zip(&b) {
            if left_byte != right_byte {
                return set_result(
                    guest,
                    (i64::from(*left_byte) - i64::from(*right_byte)) as u64,
                );
            }

            if *left_byte == 0 {
                return set_result(guest, 0);
            }
        }

        offset += chunk;
    }

    set_result(guest, 0)
}

fn strlen(guest: &Guest<'_>) -> Result<(), ClientError> {
    let value = read_c_string(guest, argument(guest, 0)?)?;

    set_result(guest, value.len() as u64)
}

/// C comparison routines return a sign, delivered in `rax` as a two's
/// complement negative.
fn ordering(order: Ordering) -> u64 {
    match order {
        Ordering::Less => -1i64 as u64,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shims() -> Shims {
        Shims {
            entries: Default::default(),
            symbols: Default::default(),
            code_cursor: 0,
            data_cursor: 0,
            allocations: Default::default(),
            free_blocks: Vec::new(),
            heap_cursor: 0,
            errno: 0,
            icxs: Vec::new(),
            icxs_offset: 0,
            iterator: 0,
            core_exports: Default::default(),
        }
    }

    #[test]
    fn allocations_are_sixteen_byte_aligned_and_never_zero_sized() {
        assert_eq!(reserved_for(0), 16);
        assert_eq!(reserved_for(1), 16);
        assert_eq!(reserved_for(16), 16);
        assert_eq!(reserved_for(17), 32);
    }

    #[test]
    fn allocations_do_not_overlap() {
        let mut shims = shims();

        let first = allocate(&mut shims, 24).unwrap();
        let second = allocate(&mut shims, 8).unwrap();

        assert_eq!(first, HEAP_BASE);
        assert_eq!(second, HEAP_BASE + 32);
    }

    #[test]
    fn freeing_the_newest_block_returns_it_to_the_cursor() {
        let mut shims = shims();

        allocate(&mut shims, 16).unwrap();
        let second = allocate(&mut shims, 16).unwrap();
        assert_eq!(shims.heap_cursor, 32);

        shims.allocations.remove(&second);
        shims.free_blocks.push(FreeBlock {
            address: second,
            size: 16,
        });
        coalesce(&mut shims);

        assert_eq!(shims.heap_cursor, 16);
        assert!(shims.free_blocks.is_empty());
    }

    #[test]
    fn adjacent_free_blocks_merge() {
        let mut shims = shims();
        shims.heap_cursor = 1024;
        shims.free_blocks = vec![
            FreeBlock {
                address: HEAP_BASE + 32,
                size: 16,
            },
            FreeBlock {
                address: HEAP_BASE,
                size: 16,
            },
            FreeBlock {
                address: HEAP_BASE + 16,
                size: 16,
            },
        ];

        coalesce(&mut shims);

        assert_eq!(shims.free_blocks.len(), 1);
        assert_eq!(shims.free_blocks[0].size, 48);
    }

    #[test]
    fn a_freed_block_is_reused_before_the_cursor_grows() {
        let mut shims = shims();
        shims.heap_cursor = 1024;
        shims.free_blocks = vec![FreeBlock {
            address: HEAP_BASE + 512,
            size: 64,
        }];

        assert_eq!(allocate(&mut shims, 32).unwrap(), HEAP_BASE + 512);
        // The remainder stays available rather than being lost.
        assert_eq!(shims.free_blocks[0].size, 32);
        assert_eq!(shims.heap_cursor, 1024);
    }

    #[test]
    fn a_full_heap_is_reported_rather_than_wrapping() {
        let mut shims = shims();
        shims.heap_cursor = HEAP_SIZE;

        assert!(allocate(&mut shims, 16).is_err());
    }

    #[test]
    fn comparison_results_are_c_style() {
        assert_eq!(ordering(Ordering::Equal), 0);
        assert_eq!(ordering(Ordering::Greater), 1);
        assert_eq!(ordering(Ordering::Less) as i64, -1);
    }
}
