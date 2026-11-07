//! Read and set the default app for a file type on macOS.
//!
//! These are the LaunchServices calls behind Finder's "Open with" and "Change
//! All". Every function compiles on every platform. Off macOS the readers find
//! nothing and [`set_default_handler`] returns [`SetError::Unsupported`], so
//! callers branch on what they want to do, not on the build target.
//!
//! The C calls are deprecated in macOS 12 but still work, and nothing else
//! sets another app's default without a user prompt. This is the only
//! unsafe code in the macOS app path, and each block says why it is sound.

use std::fmt;

/// Why a default app could not be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetError {
    /// Not running on macOS.
    Unsupported,
    /// LaunchServices refused, with its `OSStatus`. `-10814` means it has no
    /// app with that bundle identifier.
    Status(i32),
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::Unsupported => f.write_str("only macOS can set a default app this way"),
            SetError::Status(code) => write!(f, "LaunchServices error {code}"),
        }
    }
}

impl std::error::Error for SetError {}

pub use imp::{application_path, default_handler, set_default_handler, uti_for_extension};

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;
    use std::ptr;

    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::error::CFErrorRef;
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::CFURL;

    use super::SetError;

    /// `kLSRolesAll`.
    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    /// `kUTTagClassFilenameExtension`.
    const FILENAME_EXTENSION: &str = "public.filename-extension";

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            roles: u32,
        ) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            content_type: CFStringRef,
            roles: u32,
            handler_bundle_id: CFStringRef,
        ) -> i32;
        fn LSCopyApplicationURLsForBundleIdentifier(
            bundle_id: CFStringRef,
            out_error: *mut CFErrorRef,
        ) -> CFArrayRef;
        fn UTTypeCreatePreferredIdentifierForTag(
            tag_class: CFStringRef,
            tag: CFStringRef,
            conforming_to: CFStringRef,
        ) -> CFStringRef;
    }

    /// A string the caller now owns (a `Copy` or `Create` result), or `None` for null.
    fn owned(raw: CFStringRef) -> Option<String> {
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a non-null CFString returned by a function that
        // follows the Create rule, so this call owns one reference to it, and
        // `wrap_under_create_rule` releases it when the wrapper drops.
        Some(unsafe { CFString::wrap_under_create_rule(raw) }.to_string())
    }

    /// The type identifier macOS gives files with this extension, such as
    /// `public.plain-text` for `txt`.
    pub fn uti_for_extension(extension: &str) -> Option<String> {
        let class = CFString::new(FILENAME_EXTENSION);
        let tag = CFString::new(extension);
        // SAFETY: both strings are live for the call, and a null `conforming_to`
        // asks for no conformance constraint.
        let raw = unsafe {
            UTTypeCreatePreferredIdentifierForTag(
                class.as_concrete_TypeRef(),
                tag.as_concrete_TypeRef(),
                ptr::null(),
            )
        };
        owned(raw)
    }

    /// The bundle identifier of the app that opens files of type `uti`, or `None`
    /// when no app is set.
    pub fn default_handler(uti: &str) -> Option<String> {
        let uti = CFString::new(uti);
        // SAFETY: `uti` is a live CFString for the call.
        let raw =
            unsafe { LSCopyDefaultRoleHandlerForContentType(uti.as_concrete_TypeRef(), ROLES_ALL) };
        owned(raw)
    }

    /// Make the app with `bundle_id` open files of type `uti`, for every role.
    pub fn set_default_handler(uti: &str, bundle_id: &str) -> Result<(), SetError> {
        let uti = CFString::new(uti);
        let bundle_id = CFString::new(bundle_id);
        // SAFETY: both strings are live for the call.
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                uti.as_concrete_TypeRef(),
                ROLES_ALL,
                bundle_id.as_concrete_TypeRef(),
            )
        };
        match status {
            0 => Ok(()),
            code => Err(SetError::Status(code)),
        }
    }

    /// Where the app with `bundle_id` is, as far as LaunchServices knows.
    pub fn application_path(bundle_id: &str) -> Option<PathBuf> {
        let id = CFString::new(bundle_id);
        // SAFETY: `id` is live for the call, and a null error pointer asks for
        // no error object.
        let raw = unsafe {
            LSCopyApplicationURLsForBundleIdentifier(id.as_concrete_TypeRef(), ptr::null_mut())
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is a non-null CFArray of CFURL that the Copy rule hands
        // to this call, which releases it when the wrapper drops.
        let urls: CFArray<CFURL> = unsafe { CFArray::wrap_under_create_rule(raw) };
        urls.iter().next()?.to_path()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_known_extension_has_its_system_type() {
            assert_eq!(
                uti_for_extension("txt").as_deref(),
                Some("public.plain-text")
            );
        }

        #[test]
        fn a_system_app_is_found_by_its_bundle_id() {
            let finder = application_path("com.apple.finder").expect("Finder is registered");
            assert!(finder.ends_with("Finder.app"), "{}", finder.display());
            assert_eq!(application_path("com.slicer-router.no-such-app"), None);
        }

        #[test]
        fn text_files_have_a_default_app() {
            assert!(default_handler("public.plain-text").is_some());
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::PathBuf;

    use super::SetError;

    pub fn uti_for_extension(_extension: &str) -> Option<String> {
        None
    }

    pub fn default_handler(_uti: &str) -> Option<String> {
        None
    }

    pub fn set_default_handler(_uti: &str, _bundle_id: &str) -> Result<(), SetError> {
        Err(SetError::Unsupported)
    }

    pub fn application_path(_bundle_id: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_happened() {
        assert_eq!(
            SetError::Status(-10814).to_string(),
            "LaunchServices error -10814"
        );
        assert!(SetError::Unsupported.to_string().contains("macOS"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn nothing_is_found_and_nothing_can_be_set_off_macos() {
        assert_eq!(uti_for_extension("3mf"), None);
        assert_eq!(default_handler("public.data"), None);
        assert_eq!(
            application_path("com.apple.finder"),
            None::<std::path::PathBuf>
        );
        assert_eq!(
            set_default_handler("public.data", "x"),
            Err(SetError::Unsupported)
        );
    }
}
