#![no_main]

use atlas_fuzz::regex_case_from_bytes;
use atlas_store_sqlite::Store;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(case) = regex_case_from_bytes(data) else {
        return;
    };

    let _ = Store::eval_regexp_udf(&case.pattern, &case.value);
});
