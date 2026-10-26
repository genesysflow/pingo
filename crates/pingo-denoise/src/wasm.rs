//! Plain C-ABI exports for the browser AudioWorklet. No wasm-bindgen: the
//! worklet instantiates the module itself and shares one in/out frame buffer.

use crate::{DeepFilter, HOP, Strength};

fn no_randomness(_: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
getrandom::register_custom_getrandom!(no_randomness);

pub struct Handle {
    filter: DeepFilter,
    frame: [f32; HOP],
}

fn strength(strong: u32) -> Strength {
    if strong == 0 {
        Strength::Gentle
    } else {
        Strength::Strong
    }
}

/// Returns null when the model cannot be loaded.
#[unsafe(no_mangle)]
pub extern "C" fn pingo_denoise_new(strong: u32) -> *mut Handle {
    match DeepFilter::new(strength(strong)) {
        Ok(filter) => Box::into_raw(Box::new(Handle {
            filter,
            frame: [0.0; HOP],
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `handle` must come from `pingo_denoise_new` and not have been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pingo_denoise_free(handle: *mut Handle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// The 480-sample frame the caller fills before, and reads after, `process`.
///
/// # Safety
/// `handle` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pingo_denoise_frame(handle: *mut Handle) -> *mut f32 {
    unsafe { (*handle).frame.as_mut_ptr() }
}

/// Enhances the frame in place. Returns the local SNR in dB, or NaN on error.
///
/// # Safety
/// `handle` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pingo_denoise_process(handle: *mut Handle) -> f32 {
    let handle = unsafe { &mut *handle };
    handle.filter.process(&mut handle.frame).unwrap_or(f32::NAN)
}

/// # Safety
/// `handle` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pingo_denoise_set_strength(handle: *mut Handle, strong: u32) {
    unsafe { (*handle).filter.set_strength(strength(strong)) }
}

/// # Safety
/// `handle` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pingo_denoise_delay(handle: *mut Handle) -> u32 {
    unsafe { (*handle).filter.delay_samples() as u32 }
}
