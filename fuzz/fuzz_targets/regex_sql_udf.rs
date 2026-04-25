#![no_main]

use atlas_fuzz::RegexCase;
use atlas_store_sqlite::Store;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|case: RegexCase| {
    let _ = Store::eval_regexp_udf(&case.pattern, &case.value);
});
