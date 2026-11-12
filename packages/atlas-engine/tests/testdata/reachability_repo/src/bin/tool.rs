fn main() {
    // `orphan_helper` lives in `src/orphan.rs`, which is not declared as a
    // module of this package. Same-package fallback still resolves this call,
    // producing the wrong `same_package` edge the fixture guards against.
    orphan_helper();
}
