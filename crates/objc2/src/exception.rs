//! # `@throw` and `@try/@catch` exceptions.
//!
//! By default, if the [`msg_send!`] macro causes an exception to be thrown,
//! this will unwind into Rust, resulting in undefined behavior. However, this
//! crate has an `"catch-all"` feature which, when enabled, wraps each
//! [`msg_send!`] in a `@catch` and panics if an exception is caught,
//! preventing Objective-C from unwinding into Rust.
//!
//! Most of the functionality in this module is only available when the
//! `"exception"` feature is enabled.
//!
//! See the following links for more information:
//! - [Exception Programming Topics for Cocoa](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Exceptions/Exceptions.html)
//! - [The Objective-C Programming Language - Exception Handling](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/ObjectiveC/Chapters/ocExceptionHandling.html)
//! - [Exception Handling in LLVM](https://llvm.org/docs/ExceptionHandling.html)
//!
//! [`msg_send!`]: crate::msg_send

// TODO: Test this with panic=abort, and ensure that the code-size is
// reasonable in that case.

#[cfg(feature = "exception")]
use core::ffi::c_void;
use core::ffi::CStr;
use core::fmt;
#[cfg(feature = "exception")]
use core::mem;
use core::ops::Deref;
use core::panic::RefUnwindSafe;
use core::panic::UnwindSafe;
#[cfg(feature = "exception")]
use core::ptr;
use std::error::Error;

use crate::encode::{Encoding, RefEncode};
#[cfg(feature = "exception")]
use crate::ffi;
#[cfg(feature = "catch-all")]
use crate::ffi::NSUInteger;
use crate::rc::{autoreleasepool_leaking, Retained};
use crate::runtime::__nsstring::nsstring_to_str;
use crate::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use crate::{extern_methods, sel, Message};
#[cfg(feature = "catch-all")]
use crate::{msg_send, msg_send_id};

/// An Objective-C exception.
///
/// While highly recommended that any exceptions you intend to throw are
/// subclasses of `NSException`, this is not required by the runtime (similar
/// to how Rust can panic with arbitrary payloads using [`panic_any`]).
///
/// [`panic_any`]: std::panic::panic_any
#[repr(transparent)]
pub struct Exception(AnyObject);

unsafe impl RefEncode for Exception {
    const ENCODING_REF: Encoding = Encoding::Object;
}

unsafe impl Message for Exception {}

impl Deref for Exception {
    type Target = AnyObject;

    #[inline]
    fn deref(&self) -> &AnyObject {
        &self.0
    }
}

impl AsRef<AnyObject> for Exception {
    #[inline]
    fn as_ref(&self) -> &AnyObject {
        self
    }
}

impl Exception {
    fn is_nsexception(&self) -> Option<bool> {
        if self.class().responds_to(sel!(isKindOfClass:)) {
            // SAFETY: We only use `isKindOfClass:` on NSObject
            let obj: *const Exception = self;
            let obj = unsafe { obj.cast::<NSObject>().as_ref().unwrap() };
            // Get class dynamically instead of with `class!` macro
            let name = CStr::from_bytes_with_nul(b"NSException\0").unwrap();
            Some(obj.isKindOfClass(AnyClass::get(name)?))
        } else {
            Some(false)
        }
    }

    #[cfg(feature = "catch-all")]
    pub(crate) fn stack_trace(&self) -> impl fmt::Display + '_ {
        struct Helper<'a>(&'a Exception);

        impl fmt::Display for Helper<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if let Some(true) = self.0.is_nsexception() {
                    autoreleasepool_leaking(|pool| {
                        // SAFETY: The object is an `NSException`.
                        // Returns `NSArray<NSString *>`.
                        let call_stack_symbols: Option<Retained<NSObject>> =
                            unsafe { msg_send_id![self.0, callStackSymbols] };
                        if let Some(call_stack_symbols) = call_stack_symbols {
                            writeln!(f, "stack backtrace:")?;

                            // SAFETY: `call_stack_symbols` is an `NSArray`, and
                            // `count` returns `NSUInteger`.
                            let count: NSUInteger =
                                unsafe { msg_send![&call_stack_symbols, count] };
                            let mut i = 0;
                            while i < count {
                                // SAFETY: The index is in-bounds (so no exception will be thrown).
                                let symbol: Retained<NSObject> =
                                    unsafe { msg_send_id![&call_stack_symbols, objectAtIndex: i] };
                                // SAFETY: The symbol is an NSString, and is not used
                                // beyond this scope.
                                let symbol = unsafe { nsstring_to_str(&symbol, pool) };
                                writeln!(f, "{symbol}")?;
                                i += 1;
                            }
                        }
                        Ok(())
                    })
                } else {
                    Ok(())
                }
            }
        }

        Helper(self)
    }
}

extern_methods!(
    unsafe impl Exception {
        // Only safe on NSException
        // Returns NSString
        #[method_id(name)]
        unsafe fn name(&self) -> Option<Retained<NSObject>>;

        // Only safe on NSException
        // Returns NSString
        #[method_id(reason)]
        unsafe fn reason(&self) -> Option<Retained<NSObject>>;
    }
);

// Note: We can't implement `Send` nor `Sync` since the exception could be
// anything!

impl fmt::Debug for Exception {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exception ")?;

        // Attempt to present a somewhat usable error message if the exception
        // is an instance of NSException.
        if let Some(true) = self.is_nsexception() {
            autoreleasepool_leaking(|pool| {
                // SAFETY: Just checked that object is an NSException
                let (name, reason) = unsafe { (self.name(), self.reason()) };

                // SAFETY:
                // - `name` and `reason` are guaranteed to be `NSString`s.
                // - We control the scope in which they are alive, so we know
                //   they are not moved outside the current autorelease pool.
                //
                // Note that these strings are immutable (`NSException` is
                // immutable, and the properties are marked as `readonly` and
                // `copy` and are copied upon creation), so we also don't have
                // to worry about the string being mutated under our feet.
                let name = name
                    .as_deref()
                    .map(|name| unsafe { nsstring_to_str(name, pool) });
                let reason = reason
                    .as_deref()
                    .map(|reason| unsafe { nsstring_to_str(reason, pool) });

                let obj: &AnyObject = self.as_ref();
                write!(f, "{obj:?} '{}'", name.unwrap_or_default())?;
                if let Some(reason) = reason {
                    write!(f, " reason: {reason}")?;
                } else {
                    write!(f, " reason: (NULL)")?;
                }
                Ok(())
            })
        } else {
            // Fall back to `AnyObject` Debug
            write!(f, "{:?}", self.0)
        }
    }
}

impl fmt::Display for Exception {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        autoreleasepool_leaking(|pool| {
            if let Some(true) = self.is_nsexception() {
                // SAFETY: Just checked that object is an NSException
                let reason = unsafe { self.reason() };

                if let Some(reason) = &reason {
                    // SAFETY: Same as above in `Debug`.
                    let reason = unsafe { nsstring_to_str(reason, pool) };
                    return write!(f, "{reason}");
                }
            }

            write!(f, "unknown exception")
        })
    }
}

impl Error for Exception {}

impl UnwindSafe for Exception {}
impl RefUnwindSafe for Exception {}

/// Throws an Objective-C exception.
///
/// This is the Objective-C equivalent of Rust's [`panic!`].
///
/// This unwinds from Objective-C, and the exception should be caught using an
/// Objective-C exception handler like [`catch`]. It _may_ be caught by
/// [`catch_unwind`], though the error message is unlikely to be great.
///
/// [`catch_unwind`]: std::panic::catch_unwind
#[inline]
#[cfg(feature = "exception")] // For consistency, not strictly required
pub fn throw(exception: Retained<Exception>) -> ! {
    // We consume the exception object since we can't make any guarantees
    // about its mutability.
    let ptr: *const AnyObject = &exception.0;
    let ptr = ptr as *mut AnyObject;
    // SAFETY: The object is valid and non-null (nil exceptions are not valid
    // in the old runtime).
    unsafe { ffi::objc_exception_throw(ptr) }
}

#[cfg(feature = "exception")]
unsafe fn try_no_ret<F: FnOnce()>(closure: F) -> Result<(), Option<Retained<Exception>>> {
    let f = {
        extern "C-unwind" fn try_objc_execute_closure<F>(closure: &mut Option<F>)
        where
            F: FnOnce(),
        {
            // This is always passed Some, so it's safe to unwrap
            let closure = closure.take().unwrap();
            closure();
        }

        let f: extern "C-unwind" fn(&mut Option<F>) = try_objc_execute_closure;
        let f: extern "C-unwind" fn(*mut c_void) = unsafe { mem::transmute(f) };
        f
    };

    // Wrap the closure in an Option so it can be taken
    let mut closure = Some(closure);
    let context: *mut Option<F> = &mut closure;
    let context = context.cast();

    let mut exception = ptr::null_mut();
    let success = unsafe { objc2_exception_helper::try_catch(f, context, &mut exception) };

    if success == 0 {
        Ok(())
    } else {
        // SAFETY:
        // The exception is always a valid object or NULL.
        //
        // Since we do a retain inside `extern/exception.m`, the object has
        // +1 retain count.
        //
        // Code throwing an exception know that they don't hold sole access to
        // that object any more, so even if the type was originally mutable,
        // it is okay to create a new `Retained` to it here.
        Err(unsafe { Retained::from_raw(exception.cast()) })
    }
}

/// Tries to execute the given closure and catches an Objective-C exception
/// if one is thrown.
///
/// This is the Objective-C equivalent of Rust's [`catch_unwind`].
/// Accordingly, if your Rust code is compiled with `panic=abort` this cannot
/// catch the exception.
///
/// [`catch_unwind`]: std::panic::catch_unwind
///
///
/// # Errors
///
/// Returns a `Result` that is either `Ok` if the closure succeeded without an
/// exception being thrown, or an `Err` with the exception. The exception is
/// automatically released.
///
/// The exception is `None` in the extremely exceptional case that the
/// exception object is `nil`. This should basically never happen, but is
/// technically possible on some systems with `@throw nil`, or in OOM
/// situations.
///
///
/// # Safety
///
/// The given closure must not panic (e.g. normal Rust unwinding into this
/// causes undefined behaviour).
#[cfg(feature = "exception")]
pub unsafe fn catch<R>(
    closure: impl FnOnce() -> R + UnwindSafe,
) -> Result<R, Option<Retained<Exception>>> {
    let mut value = None;
    let value_ref = &mut value;
    let closure = move || {
        *value_ref = Some(closure());
    };
    let result = unsafe { try_no_ret(closure) };
    // If the try succeeded, value was set so it's safe to unwrap
    result.map(|()| value.unwrap_or_else(|| unreachable!()))
}

#[cfg(test)]
#[cfg(feature = "exception")]
mod tests {
    use alloc::format;
    use alloc::string::ToString;
    use core::panic::AssertUnwindSafe;
    use std::panic::catch_unwind;

    use super::*;
    use crate::msg_send_id;

    #[test]
    fn test_catch() {
        let mut s = "Hello".to_string();
        let result = unsafe {
            catch(move || {
                s.push_str(", World!");
                s
            })
        };
        assert_eq!(result.unwrap(), "Hello, World!");
    }

    #[test]
    #[cfg_attr(
        all(target_vendor = "apple", target_os = "macos", target_arch = "x86"),
        ignore = "`NULL` exceptions are invalid on 32-bit / w. fragile runtime"
    )]
    fn test_catch_null() {
        let s = "Hello".to_string();
        let result = unsafe {
            catch(move || {
                if !s.is_empty() {
                    ffi::objc_exception_throw(ptr::null_mut())
                }
                s.len()
            })
        };
        assert!(result.unwrap_err().is_none());
    }

    #[test]
    #[cfg_attr(
        feature = "catch-all",
        ignore = "Panics inside `catch` when catch-all is enabled"
    )]
    fn test_catch_unknown_selector() {
        let obj = AssertUnwindSafe(NSObject::new());
        let ptr = Retained::as_ptr(&obj);
        let result = unsafe {
            catch(|| {
                let _: Retained<NSObject> = msg_send_id![&*obj, copy];
            })
        };
        let err = result.unwrap_err().unwrap();

        assert_eq!(
            format!("{err}"),
            format!("-[NSObject copyWithZone:]: unrecognized selector sent to instance {ptr:?}"),
        );
    }

    #[test]
    fn test_throw_catch_object() {
        let obj = NSObject::new();
        // TODO: Investigate why this is required on GNUStep!
        let _obj2 = obj.clone();
        let obj: Retained<Exception> = unsafe { Retained::cast(obj) };
        let ptr: *const Exception = &*obj;

        let result = unsafe { catch(|| throw(obj)) };
        let obj = result.unwrap_err().unwrap();

        assert_eq!(format!("{obj:?}"), format!("exception <NSObject: {ptr:p}>"));

        assert!(ptr::eq(&*obj, ptr));
    }

    #[test]
    #[ignore = "currently aborts"]
    fn throw_catch_unwind() {
        let obj = NSObject::new();
        let obj: Retained<Exception> = unsafe { Retained::cast(obj) };

        let result = catch_unwind(|| throw(obj));
        let _ = result.unwrap_err();
    }
}
