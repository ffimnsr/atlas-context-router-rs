#[test]
fn integration_calls_lib() {
    assert_eq!(fixture_shared(), 1);
}
