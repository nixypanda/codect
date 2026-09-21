//! Crate documentation that must be removed.

/// A documented struct.
#[derive(Debug, Clone)]
#[repr(C)]
#[cfg(feature = "serde")]
pub struct Documented {
    /// Field documentation removed.
    #[serde(rename = "id")]
    pub id: u32,

    #[doc = "inline doc attribute must be removed"]
    #[cfg(test)]
    hidden: String,
}

#[non_exhaustive]
pub enum Flag {
    /// Variant documentation removed.
    #[deprecated]
    On,
    Off,
}

#[doc(hidden)]
pub fn documented_function() {}
