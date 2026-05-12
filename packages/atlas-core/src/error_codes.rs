pub const ERROR_CODE_CATALOG_PATH: &str = "docs/error_codes.md";

pub fn error_code_docs_ref(error_code: &str) -> String {
    format!("{ERROR_CODE_CATALOG_PATH}#{error_code}")
}
