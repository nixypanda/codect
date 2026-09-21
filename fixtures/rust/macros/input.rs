//! Macro definitions and invocations must not produce items.

macro_rules! define_thing {
    ($name:ident) => {
        pub struct $name;
    };
}

define_thing!(Generated);

#[derive(Clone)]
#[my_attribute(option = "value")]
pub struct Real;

pub fn uses_macro() {
    println!("hello");
    other_macro!(1, 2, 3);
}
