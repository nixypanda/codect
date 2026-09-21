//! Every enum variant form.

pub enum Status {
    Active,
    Suspended { reason: String },
    Point(i32, i32),
    #[cfg(feature = "legacy")]
    Legacy,
}

enum Discriminants {
    Zero = 0,
    One = 1,
    Ten = 1 + 9,
}

enum Empty {}
