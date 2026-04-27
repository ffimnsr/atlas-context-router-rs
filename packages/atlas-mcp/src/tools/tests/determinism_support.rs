pub fn assert_text_deterministic<F>(label: &str, mut produce: F)
where
    F: FnMut() -> String,
{
    let first = produce();
    let second = produce();

    assert_eq!(
        first, second,
        "{label} text must be byte-identical across repeated runs\nfirst:\n{first}\nsecond:\n{second}"
    );
}
