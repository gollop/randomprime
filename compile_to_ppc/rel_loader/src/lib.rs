#![no_std]

use core::mem::MaybeUninit;

use primeapi::dol_sdk::dvd::DVDFileInfo;
use primeapi::dol_sdk::os::{OSLink, OSModuleHeader};
use primeapi::alignment_utils::Aligned32SliceMut;

include!("../../patches_config.rs");
#[no_mangle]
pub static REL_CONFIG: RelConfig = RelConfig {
    quickplay_mlvl: 0xFFFFFFFF,
    quickplay_mrea: 0xFFFFFFFF,
};


#[inline(always)]
fn leak_aligned_slice<'a>(len: usize) -> Aligned32SliceMut<'a, MaybeUninit<u8>>
{
    unsafe {
        // Over-allocate and then manually ensure the alignment
        let ptr = primeapi::malloc(len + 31) as *mut MaybeUninit<u8>;
        let slice = core::slice::from_raw_parts_mut(((ptr as usize + 31) & !31) as *mut _, len);
        Aligned32SliceMut::from_slice_unchecked(slice)
    }
}

#[export_name = "rel_loader_hook"]
pub unsafe extern "C" fn rel_loader_hook()
{
    let mut fi = if let Some(fi) = DVDFileInfo::new(b"patches.rel\0") {
        fi
    } else {
        return;
    };
    let rel_size = fi.file_length();

    let mut rel_data = leak_aligned_slice(rel_size as usize);

    {
        let _handle = fi.read_async(rel_data.reborrow(), 0, 0);
    }

    let rel_header = rel_data.as_mut_ptr() as *mut OSModuleHeader;
    let mut bss_data = leak_aligned_slice((&*rel_header).bss_size as usize);

    OSLink(&mut (*rel_header).mod_info, bss_data.as_mut_ptr() as *mut u8);
    if let Some(prolog_ptr) = (*rel_header).prolog_function.func_ptr {
        prolog_ptr();
    }
}
