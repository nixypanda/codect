use std::collections::HashMap;

pub struct Registry {
    entries: HashMap<String, u32>,
}

pub trait Lookup {
    fn get(&self, key: &str) -> Option<u32>;
}

pub const LIMIT: u32 = 8;

pub fn helper(value: u32) -> u32 {
    value
}

#[test]
fn plain_attribute_is_a_test() {
    assert_eq!(helper(1), 1);
}

#[tokio::test]
async fn qualified_attribute_is_a_test() {
    helper(2);
}

#[rstest]
#[case(1, 2)]
fn rstest_is_a_test(input: u32, expected: u32) {
    assert_eq!(input, expected);
}

#[test]
#[should_panic]
fn companion_attributes_do_not_change_the_verdict() {
    panic!("expected");
}

#[derive(Debug)]
pub struct NotATest {
    pub field: u32,
}

#[derive(Debug)]
pub enum AlsoNotATest {
    Variant,
}

#[allow(dead_code)]
pub fn not_a_test() {}

#[test]
fn free_standing_test_has_no_module() {
    helper(3);
}

pub mod empty {
    pub fn no_tests_here() {}
}

#[cfg(test)]
mod tests {
    use super::helper;

    fn setup_helper() -> u32 {
        helper(4)
    }

    #[test]
    fn inside_a_test_module() {
        assert_eq!(setup_helper(), 4);
    }

    #[tokio::test]
    async fn async_inside_a_test_module() {
        setup_helper();
    }

    pub fn not_a_test_helper() {}
}

pub mod nested {
    pub mod deeper {
        #[test]
        fn deeply_nested_test() {}
    }
}
#[cfg(test)]
mod test_case_suite {
    use test_case::test_case;

    #[test_case(-2, -4 ; "both negative")]
    #[test_case(2, 4 ; "both positive")]
    fn multiplication_tests(x: i8, y: i8) {
        assert_eq!(x * y, 8);
    }

    pub fn not_a_test() {}
}

#[cfg(test)]
mod test_matrix_suite {
    use test_case::test_matrix;

    #[test_matrix([-2, 2], [-4, 4])]
    fn cartesian(x: i8, y: i8) {
        assert_eq!((x * y).abs(), 8);
    }
}
