//! Inline module nesting and ignored out-of-line modules.

pub mod outer {
    pub mod inner {
        pub struct Deep {
            pub value: u32,
        }

        impl Deep {
            pub fn value(&self) -> u32 {
                0
            }
        }
    }

    pub enum Level {
        Top,
    }
}

mod empty {
    fn hidden() {}
}

mod out_of_line;
