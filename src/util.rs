//! Small shared helpers.

use std::time::{Duration, Instant};

/// Defines a fieldless enum together with a stable string name per variant,
/// plus `ALL`, `COUNT`, `name()` and `from_index()`, so the variant list and
/// its names can never drift apart.
///
/// ```ignore
/// named_enum! {
///     pub enum Color { Red => "red", Green => "green" }
/// }
/// ```
macro_rules! named_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $str:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        #[allow(dead_code, reason = "Not every named_enum uses every helper")]
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const COUNT: usize = Self::ALL.len();

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $str),+
                }
            }

            pub fn from_index(index: u32) -> Option<Self> {
                Self::ALL.get(index as usize).copied()
            }
        }
    };
}
pub(crate) use named_enum;

/// A non-blocking periodic timer for polling loops: `tick()` returns true at
/// most once per `period`, starting with the first call.
pub struct Interval {
    period: Duration,
    next: Instant,
}

impl Interval {
    pub fn new(period: Duration) -> Self {
        Self {
            period,
            next: Instant::now(),
        }
    }

    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next {
            return false;
        }
        self.next = now + self.period;
        true
    }
}
