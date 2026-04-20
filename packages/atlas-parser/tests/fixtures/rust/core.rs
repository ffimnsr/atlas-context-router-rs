use crate::support::Helper;

pub fn helper(value: i32) -> i32 {
    value + 1
}

pub fn caller() -> i32 {
    helper(1)
}

pub struct Greeter;

impl Greeter {
    pub fn greet(&self) -> i32 {
        caller()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        helper(1);
    }
}
