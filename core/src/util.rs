//! Small shared helpers.

use std::time::{Duration, Instant};

/// Defines a fieldless enum together with a stable string name per variant,
/// plus `ALL`, `COUNT`, `name()` and `from_index()`, so the variant list and
/// its names can never drift apart.
///
/// ```
/// matrix_hub_core::named_enum! {
///     pub enum Color { Red => "red", Green => "green" }
/// }
/// assert_eq!(Color::Green.name(), "green");
/// assert_eq!(Color::from_index(0), Some(Color::Red));
/// assert_eq!(Color::COUNT, 2);
/// ```
#[macro_export]
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

/// A non-blocking periodic timer for polling loops: `tick()` returns true at
/// most once per `period`, starting with the first call.
#[derive(Debug)]
pub struct Interval {
    period: Duration,
    next: Option<Instant>,
}

impl Interval {
    pub const fn new(period: Duration) -> Self {
        Self { period, next: None }
    }

    /// Like [`Interval::tick_at`] with the current time.
    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    /// True if a period has elapsed since the last tick (or this is the
    /// first call); the next period then starts at `now`.
    pub fn tick_at(&mut self, now: Instant) -> bool {
        if self.next.is_some_and(|next| now < next) {
            return false;
        }
        self.next = Some(now + self.period);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    named_enum! {
        enum Fruit { Apple => "apple", Pear => "pear" }
    }

    #[gtest]
    fn named_enum_exposes_variants_names_and_indices() {
        expect_that!(
            Fruit::ALL,
            elements_are![eq(&Fruit::Apple), eq(&Fruit::Pear)]
        );
        expect_that!(Fruit::COUNT, eq(2));
        expect_that!(Fruit::Pear.name(), eq("pear"));
        expect_that!(Fruit::from_index(0), some(eq(Fruit::Apple)));
        expect_that!(Fruit::from_index(2), none());
    }

    #[gtest]
    fn interval_ticks_immediately_then_once_per_period() {
        let start = Instant::now();
        let period = Duration::from_secs(1);
        let mut interval = Interval::new(period);

        expect_true!(interval.tick_at(start));
        expect_false!(interval.tick_at(start + Duration::from_millis(999)));
        expect_true!(interval.tick_at(start + period));
        expect_false!(interval.tick_at(start + period));
    }

    #[gtest]
    fn interval_restarts_the_period_from_a_late_tick() {
        let start = Instant::now();
        let mut interval = Interval::new(Duration::from_secs(1));
        interval.tick_at(start);

        // Ticking 5s late doesn't produce a burst of catch-up ticks
        expect_true!(interval.tick_at(start + Duration::from_secs(5)));
        expect_false!(interval.tick_at(start + Duration::from_millis(5500)));
    }

    #[gtest]
    fn interval_tick_uses_the_current_time() {
        let mut interval = Interval::new(Duration::from_secs(3600));
        expect_true!(interval.tick());
        expect_false!(interval.tick());
    }
}
