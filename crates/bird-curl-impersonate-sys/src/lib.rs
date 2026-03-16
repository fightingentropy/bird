#![allow(clippy::missing_safety_doc)]

use std::ffi::c_char;
use std::os::raw::c_int;

pub type EasyCode = curl_sys::CURLcode;

pub const CURLE_OK: EasyCode = curl_sys::CURLE_OK;

#[cfg(all(target_os = "macos", bird_native_impersonation))]
unsafe extern "C" {
    fn curl_easy_impersonate(
        curl: *mut curl_sys::CURL,
        target: *const c_char,
        default_headers: c_int,
    ) -> curl_sys::CURLcode;
}

#[cfg(all(target_os = "macos", bird_native_impersonation))]
pub unsafe fn easy_impersonate(
    curl: *mut curl_sys::CURL,
    target: *const c_char,
    default_headers: c_int,
) -> EasyCode {
    unsafe { curl_easy_impersonate(curl, target, default_headers) }
}

#[cfg(not(all(target_os = "macos", bird_native_impersonation)))]
pub unsafe fn easy_impersonate(
    _curl: *mut curl_sys::CURL,
    _target: *const c_char,
    _default_headers: c_int,
) -> EasyCode {
    curl_sys::CURLE_FAILED_INIT
}

pub const fn native_impersonation_enabled() -> bool {
    cfg!(all(target_os = "macos", bird_native_impersonation))
}
