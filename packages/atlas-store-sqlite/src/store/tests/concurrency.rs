// Compile-time enforcement: `Store` must not implement `Send` or `Sync`.
//
// `rusqlite::Connection` is `!Send` because it wraps a raw pointer to the
// SQLite connection object, which must be confined to the thread that opened it.
// `Store` owns a `Connection` directly, so it inherits `!Send` and `!Sync`.
//
// These assertions will fail to compile if anyone accidentally adds
// `unsafe impl Send for Store` or wraps `Store` in a cross-thread helper.
// They are the enforced form of the thread-confinement contract documented
// in the `Store` struct.

use static_assertions::assert_not_impl_any;

use crate::Store;

assert_not_impl_any!(Store: Send);
assert_not_impl_any!(Store: Sync);
