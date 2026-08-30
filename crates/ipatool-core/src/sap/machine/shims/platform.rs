//! The platform services the guest expects from macOS.
//!
//! None of these are real. The guest queries IOKit for machine identifiers,
//! opens `CoreFP.icxs`, and resolves CoreFP entry points through `dlsym`; each
//! is answered with the smallest response that keeps it moving. The machine
//! identity Apple actually binds the session to is the hardware ID passed to
//! `initialize`, not anything discovered here.

use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    Shim, Shims, argument, checked_size, read_c_string, read_u32, read_u64, set_result, write_u32,
    write_u64,
};
use crate::error::ClientError;
use crate::sap::unicorn::{Guest, reg};

/// Returned where the guest only checks for non-NULL.
const FAKE_HANDLE: u64 = u64::MAX;

/// The descriptor `open` hands out for `CoreFP.icxs`.
const ICXS_DESCRIPTOR: u64 = 3;

const CORE_FP_PATH: &str = "/System/Library/PrivateFrameworks/CoreFP.framework/CoreFP";
const ICXS_PATH: &str = "./../CoreFP.icxs";

/// IOKit keys the guest looks up; anything else resolves to NULL.
const KEY_SERIAL: &str = "IOPlatformSerialNumber";
const KEY_UUID: &str = "IOPlatformUUID";
const KEY_BOARD: &str = "board-id";

const KEYED_MESSAGE: &str = "objectForKey:";

pub fn register(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let services: &[(&[&str], Shim)] = &[
        (
            &[
                "_CFBundleGetMainBundle",
                "_CFDataGetBytePtr",
                "_CFDataGetLength",
                "_CFStringGetLength",
                "_CFStringGetMaximumSizeForEncoding",
                "_CFUUIDCreateString",
                "_IORegistryEntryFromPath",
                "_IORegistryEntrySearchCFProperty",
                "_IOServiceMatching",
                "_getenv",
                "_pthread_self",
                "_CFStringCreateWithCStringNoCopy",
                // Teardown and locking: nothing to release, no contention.
                "_CFRelease",
                "_IOObjectRelease",
                "_close",
                "_close$UNIX2003",
                "_pthread_mutex_lock",
                "_pthread_mutex_unlock",
                "_pthread_rwlock_init",
                "_pthread_rwlock_init$UNIX2003",
                "_pthread_rwlock_unlock",
                "_pthread_rwlock_unlock$UNIX2003",
                "_pthread_rwlock_wrlock",
                "_pthread_rwlock_wrlock$UNIX2003",
            ],
            Shim::ReturnZero,
        ),
        (
            &[
                "_CFDictionaryGetValue",
                "_DADiskCopyDescription",
                "_DADiskCreateFromBSDName",
                "_DASessionCreate",
                "_IORegistryEntryCreateCFProperty",
            ],
            Shim::ReturnFakeHandle,
        ),
        (&["_CFStringCreateWithCString"], Shim::CfStringCreate),
        (&["_CFStringGetCString"], Shim::CfStringGetCString),
        (&["_IOIteratorNext"], Shim::IoIteratorNext),
        (
            &["_IORegistryEntryGetParentEntry"],
            Shim::IoRegistryEntryGetParentEntry,
        ),
        (
            &["_IOServiceGetMatchingServices"],
            Shim::IoServiceGetMatchingServices,
        ),
        (&["_IOServiceGetMatchingService"], Shim::ReturnUint32Max),
        (
            &["_OSAtomicCompareAndSwap32Barrier"],
            Shim::CompareAndSwap32,
        ),
        (&["___error"], Shim::ErrorPointer),
        (
            &["_abort", "___stack_chk_fail", "dyld_stub_binder"],
            Shim::Abort,
        ),
        (&["_arc4random"], Shim::Arc4Random),
        (&["_dlopen"], Shim::Dlopen),
        (&["_dlsym"], Shim::Dlsym),
        (
            &[
                "_fcntl",
                "_fcntl$UNIX2003",
                "_lstat$INODE64",
                "_statfs",
                "_statfs$INODE64",
                "_sysctl",
            ],
            Shim::ReturnMinusOne,
        ),
        (&["_gettimeofday"], Shim::Gettimeofday),
        (&["_objc_msgSend"], Shim::ObjcMsgSend),
        (&["_open", "_open$UNIX2003"], Shim::Open),
        (&["_pthread_once"], Shim::PthreadOnce),
        (&["_read", "_read$UNIX2003"], Shim::Read),
        (&["_sysctlbyname"], Shim::Sysctlbyname),
    ];

    for (names, shim) in services {
        shims.add_aliases(names, shim.clone(), guest)?;
    }

    shims.errno = shims.add_data("guest.errno", &[0u8; 8], guest)?;

    // Read by the prologue of every function compiled with stack protection.
    shims.add_data(
        "___stack_chk_guard",
        &[0xA5, 0x71, 0x3C, 0xD9, 0x86, 0x42, 0xEF, 0x10],
        guest,
    )?;

    // Imported as data rather than called; the guest only passes them along.
    for name in [
        "_kCFAllocatorDefault",
        "_kCFAllocatorNull",
        "_kDADiskDescriptionVolumeUUIDKey",
        "_kIOMasterPortDefault",
    ] {
        shims.add_data(name, &[0u8; 8], guest)?;
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
        Shim::ReturnZero => set_result(guest, 0),
        Shim::ReturnFakeHandle => set_result(guest, FAKE_HANDLE),
        Shim::ReturnUint32Max => set_result(guest, u64::from(u32::MAX)),
        Shim::ReturnMinusOne => set_result(guest, u64::MAX),
        Shim::CfStringCreate => cf_string_create(guest),
        Shim::CfStringGetCString => cf_string_get_c_string(guest),
        Shim::IoIteratorNext => io_iterator_next(shims, guest),
        Shim::IoRegistryEntryGetParentEntry => io_registry_entry_get_parent_entry(guest),
        Shim::IoServiceGetMatchingServices => io_service_get_matching_services(shims, guest),
        Shim::CompareAndSwap32 => compare_and_swap_32(guest),
        Shim::ErrorPointer => set_result(guest, shims.errno),
        Shim::Abort => Err(ClientError::Sap("guest aborted".into())),
        Shim::Arc4Random => arc4random(guest),
        Shim::Dlopen => dlopen(guest),
        Shim::Dlsym => dlsym(shims, guest),
        Shim::Gettimeofday => gettimeofday(guest),
        Shim::ObjcMsgSend => objc_msg_send(guest),
        Shim::Open => open(shims, guest),
        Shim::PthreadOnce => pthread_once(guest),
        Shim::Read => read(shims, guest),
        Shim::Sysctlbyname => sysctlbyname(guest),
        _ => return None,
    })
}

/// Only the identifier keys need to survive as distinguishable handles; every
/// other string the guest builds is one it never reads back.
fn cf_string_create(guest: &Guest<'_>) -> Result<(), ClientError> {
    let value = read_c_string(guest, argument(guest, 1)?)?;

    set_result(
        guest,
        match value.as_str() {
            KEY_SERIAL | KEY_UUID | KEY_BOARD => FAKE_HANDLE,
            _ => 0,
        },
    )
}

fn cf_string_get_c_string(guest: &Guest<'_>) -> Result<(), ClientError> {
    let buffer = argument(guest, 1)?;
    let capacity = argument(guest, 2)?;

    if buffer == 0 || capacity == 0 {
        return set_result(guest, 0);
    }

    // An empty string: the guest checks for success, not for content.
    guest.mem_write(buffer, &[0])?;

    set_result(guest, 1)
}

/// Yields one entry and then stops, so the guest's registry walk terminates.
fn io_iterator_next(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    shims.iterator += 1;

    set_result(guest, u64::from(shims.iterator % 2))
}

fn io_registry_entry_get_parent_entry(guest: &Guest<'_>) -> Result<(), ClientError> {
    let parent = argument(guest, 2)?;

    if parent == 0 {
        return Err(ClientError::Sap(
            "parent registry entry output is null".into(),
        ));
    }

    write_u32(guest, parent, u32::MAX)?;

    set_result(guest, 0)
}

fn io_service_get_matching_services(
    shims: &mut Shims,
    guest: &Guest<'_>,
) -> Result<(), ClientError> {
    let iterator = argument(guest, 2)?;

    if iterator == 0 {
        return Err(ClientError::Sap(
            "matching services iterator output is null".into(),
        ));
    }

    // A fresh iterator: the guest may walk the registry more than once.
    shims.iterator = 0;
    write_u32(guest, iterator, u32::MAX)?;

    set_result(guest, 0)
}

fn compare_and_swap_32(guest: &Guest<'_>) -> Result<(), ClientError> {
    let expected = argument(guest, 0)? as u32;
    let replacement = argument(guest, 1)? as u32;
    let address = argument(guest, 2)?;

    if read_u32(guest, address)? != expected {
        return set_result(guest, 0);
    }

    write_u32(guest, address, replacement)?;

    set_result(guest, 1)
}

fn arc4random(guest: &Guest<'_>) -> Result<(), ClientError> {
    let mut value = [0u8; 4];
    getrandom(&mut value)?;

    set_result(guest, u64::from(u32::from_le_bytes(value)))
}

fn dlopen(guest: &Guest<'_>) -> Result<(), ClientError> {
    let path = read_c_string(guest, argument(guest, 0)?)?;

    set_result(guest, if path == CORE_FP_PATH { FAKE_HANDLE } else { 0 })
}

/// CoreFP is already mapped, so `dlsym` is answered from its export table.
fn dlsym(shims: &Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let name = read_c_string(guest, argument(guest, 1)?)?;
    let address = shims
        .core_exports
        .get(&format!("_{name}"))
        .copied()
        .unwrap_or(0);

    set_result(guest, address)
}

fn gettimeofday(guest: &Guest<'_>) -> Result<(), ClientError> {
    let time = argument(guest, 0)?;
    let zone = argument(guest, 1)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ClientError::Sap(format!("system clock is before the epoch: {e}")))?;

    if time != 0 {
        // struct timeval { time_t tv_sec; suseconds_t tv_usec; }
        let mut value = [0u8; 16];
        value[0..8].copy_from_slice(&now.as_secs().to_le_bytes());
        value[8..12].copy_from_slice(&now.subsec_micros().to_le_bytes());

        guest.mem_write(time, &value)?;
    }

    if zone != 0 {
        guest.mem_write(zone, &[0u8; 8])?;
    }

    set_result(guest, 0)
}

fn objc_msg_send(guest: &Guest<'_>) -> Result<(), ClientError> {
    let selector = read_c_string(guest, argument(guest, 1)?)?;

    set_result(
        guest,
        if selector == KEYED_MESSAGE {
            FAKE_HANDLE
        } else {
            0
        },
    )
}

fn open(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let path = read_c_string(guest, argument(guest, 0)?)?;

    if path != ICXS_PATH {
        return set_result(guest, u64::MAX);
    }

    // Reopening rewinds, so a second pass reads the file from the start.
    shims.icxs_offset = 0;

    set_result(guest, ICXS_DESCRIPTOR)
}

/// Arranges for the initializer to run once, by pushing it as the stub's return
/// address: the stub's `ret` jumps to the initializer, whose own `ret` then
/// returns to the original caller.
fn pthread_once(guest: &Guest<'_>) -> Result<(), ClientError> {
    let control = argument(guest, 0)?;
    let initializer = argument(guest, 1)?;

    if read_u64(guest, control)? == 0 {
        return set_result(guest, 0);
    }

    write_u64(guest, control, 0)?;

    let stack = guest.reg_read(reg::RSP)? - 8;
    write_u64(guest, stack, initializer)?;
    guest.reg_write(reg::RSP, stack)?;

    set_result(guest, 0)
}

fn read(shims: &mut Shims, guest: &Guest<'_>) -> Result<(), ClientError> {
    let descriptor = argument(guest, 0)?;
    let buffer = argument(guest, 1)?;
    let requested = argument(guest, 2)?;

    if descriptor != ICXS_DESCRIPTOR {
        return set_result(guest, u64::MAX);
    }

    let remaining = shims.icxs.len() - shims.icxs_offset;
    let size = checked_size(requested)?.min(remaining);

    if size != 0 {
        guest.mem_write(
            buffer,
            &shims.icxs[shims.icxs_offset..shims.icxs_offset + size],
        )?;

        shims.icxs_offset += size;
    }

    set_result(guest, size as u64)
}

fn sysctlbyname(guest: &Guest<'_>) -> Result<(), ClientError> {
    let length = argument(guest, 2)?;

    // Report success with an empty result rather than failing: the guest treats
    // a missing value as absent hardware, which is the honest answer here.
    if length != 0 {
        write_u64(guest, length, 0)?;
    }

    set_result(guest, 0)
}

/// Fills `out` with operating-system randomness.
///
/// The guest reaches this through `arc4random`, so it has to work everywhere
/// the tool runs — reading `/dev/urandom` directly does not.
fn getrandom(out: &mut [u8]) -> Result<(), ClientError> {
    getrandom::fill(out).map_err(|e| ClientError::Sap(format!("read system randomness: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randomness_is_not_constant() {
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];

        getrandom(&mut first).unwrap();
        getrandom(&mut second).unwrap();

        assert_ne!(first, second);
        assert_ne!(first, [0u8; 32]);
    }

    #[test]
    fn only_identifier_keys_become_handles() {
        for key in [KEY_SERIAL, KEY_UUID, KEY_BOARD] {
            assert!(!key.is_empty());
        }

        // A guard against the constants drifting from what the guest asks for.
        assert_eq!(KEY_SERIAL, "IOPlatformSerialNumber");
        assert_eq!(ICXS_PATH, "./../CoreFP.icxs");
    }
}
