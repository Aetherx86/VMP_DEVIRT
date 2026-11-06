//! The abstract values: integer intervals, and pointers with a nullness and an allocation size.

// `Interval::div` and `rem` return an `Option`, so the operator traits would not fit all of these.
#![allow(clippy::should_implement_trait)]

/// Stands for an unbounded end. Real bounds never get near this, because arithmetic clamps to it.
pub const INF: i128 = i128::MAX / 4;

fn clamp(x: i128) -> i128 {
    x.clamp(-INF, INF)
}

/// A non-empty set of integers from `lo` to `hi`. Emptiness is an unreachable state, which is
/// represented by having no state at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub lo: i128,
    pub hi: i128,
}

impl Interval {
    pub const TOP: Interval = Interval { lo: -INF, hi: INF };

    pub fn new(lo: i128, hi: i128) -> Self {
        debug_assert!(lo <= hi);
        Interval {
            lo: clamp(lo),
            hi: clamp(hi),
        }
    }

    pub fn exact(v: i128) -> Self {
        Interval::new(v, v)
    }

    pub fn unsigned_bits(bits: u32) -> Self {
        Interval::new(0, (1i128 << bits) - 1)
    }

    pub fn contains(self, v: i128) -> bool {
        // A value beyond the clamp can only be inside an unbounded end.
        (self.lo <= v || self.lo == -INF) && (v <= self.hi || self.hi == INF)
    }

    pub fn includes(self, other: Interval) -> bool {
        self.lo <= other.lo && other.hi <= self.hi
    }

    pub fn join(self, other: Interval) -> Interval {
        Interval {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn meet(self, other: Interval) -> Option<Interval> {
        let (lo, hi) = (self.lo.max(other.lo), self.hi.min(other.hi));
        (lo <= hi).then_some(Interval { lo, hi })
    }

    /// Jumps any growing bound out to the next threshold, which is what makes loops terminate.
    pub fn widen(self, next: Interval, thresholds: &[i128]) -> Interval {
        let lo = if next.lo < self.lo {
            thresholds
                .iter()
                .rev()
                .find(|t| **t <= next.lo)
                .copied()
                .unwrap_or(-INF)
        } else {
            self.lo
        };
        let hi = if next.hi > self.hi {
            thresholds
                .iter()
                .find(|t| **t >= next.hi)
                .copied()
                .unwrap_or(INF)
        } else {
            self.hi
        };
        Interval { lo, hi }
    }

    pub fn add(self, o: Interval) -> Interval {
        Interval::new(self.lo.saturating_add(o.lo), self.hi.saturating_add(o.hi))
    }

    pub fn sub(self, o: Interval) -> Interval {
        Interval::new(self.lo.saturating_sub(o.hi), self.hi.saturating_sub(o.lo))
    }

    pub fn mul(self, o: Interval) -> Interval {
        let corners = [
            self.lo.saturating_mul(o.lo),
            self.lo.saturating_mul(o.hi),
            self.hi.saturating_mul(o.lo),
            self.hi.saturating_mul(o.hi),
        ];
        Interval::new(
            *corners.iter().min().unwrap(),
            *corners.iter().max().unwrap(),
        )
    }

    /// Truncating division. The caller reports a divisor that may be zero, and past that point the
    /// divisor can be taken not to be.
    pub fn div(self, o: Interval) -> Option<Interval> {
        let mut out: Option<Interval> = None;
        let mut part = |d: Interval| {
            let corners = [
                self.lo / d.lo,
                self.lo / d.hi,
                self.hi / d.lo,
                self.hi / d.hi,
            ];
            // A dividend spanning 0 has corners of both signs, so the range between them includes 0.
            let r = Interval::new(
                *corners.iter().min().unwrap(),
                *corners.iter().max().unwrap(),
            );
            out = Some(out.map_or(r, |o: Interval| o.join(r)));
        };
        if o.hi >= 1 {
            part(Interval {
                lo: o.lo.max(1),
                hi: o.hi,
            });
        }
        if o.lo <= -1 {
            part(Interval {
                lo: o.lo,
                hi: o.hi.min(-1),
            });
        }
        out
    }

    /// The remainder takes the sign of the dividend and is smaller than the divisor.
    pub fn rem(self, o: Interval) -> Option<Interval> {
        if o == Interval::exact(0) {
            return None;
        }
        let magnitude =
            o.lo.saturating_abs()
                .max(o.hi.saturating_abs())
                .saturating_sub(1);
        let lo = if self.lo < 0 {
            (-magnitude).max(self.lo)
        } else {
            0
        };
        let hi = if self.hi > 0 {
            magnitude.min(self.hi)
        } else {
            0
        };
        Some(Interval::new(lo, hi))
    }

    fn fill(x: i128) -> i128 {
        if x >= INF {
            INF
        } else if x <= 0 {
            0
        } else {
            i128::MAX >> (x.leading_zeros() - 1)
        }
    }

    pub fn bit_and(self, o: Interval) -> Interval {
        match (self.lo >= 0, o.lo >= 0) {
            (true, true) => Interval::new(0, self.hi.min(o.hi)),
            (true, false) => Interval::new(0, self.hi),
            (false, true) => Interval::new(0, o.hi),
            _ => Interval::TOP,
        }
    }

    pub fn bit_or(self, o: Interval) -> Interval {
        if self.lo >= 0 && o.lo >= 0 {
            Interval::new(self.lo.max(o.lo), Self::fill(self.hi.max(o.hi)))
        } else {
            Interval::TOP
        }
    }

    pub fn bit_xor(self, o: Interval) -> Interval {
        if self.lo >= 0 && o.lo >= 0 {
            Interval::new(0, Self::fill(self.hi.max(o.hi)))
        } else {
            Interval::TOP
        }
    }

    pub fn shl(self, o: Interval) -> Interval {
        if o.lo < 0 || o.hi > 100 {
            return Interval::TOP;
        }
        self.mul(Interval::new(1i128 << o.lo, 1i128 << o.hi))
    }

    /// An arithmetic right shift, which is monotone in the value being shifted.
    pub fn shr(self, o: Interval) -> Interval {
        if o.lo < 0 || o.hi > 100 {
            return Interval::TOP;
        }
        let at = |v: i128, s: i128| if v.abs() >= INF { v } else { v >> s };
        Interval::new(
            at(self.lo, o.lo).min(at(self.lo, o.hi)),
            at(self.hi, o.lo).max(at(self.hi, o.hi)),
        )
    }

    /// Reduces modulo `2^bits` into the unsigned range.
    pub fn wrap(self, bits: u32) -> Interval {
        let modulus = 1i128 << bits;
        if self.lo > -INF
            && self.hi < INF
            && self.lo.div_euclid(modulus) == self.hi.div_euclid(modulus)
        {
            let shift = self.lo.div_euclid(modulus) * modulus;
            Interval {
                lo: self.lo - shift,
                hi: self.hi - shift,
            }
        } else {
            Interval::unsigned_bits(bits)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nullness {
    Null,
    NonNull,
    MaybeNull,
}

impl Nullness {
    fn join(self, o: Nullness) -> Nullness {
        if self == o { self } else { Nullness::MaybeNull }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Int(Interval),
    /// A pointer, and the sizes its allocation can have. It says nothing about which allocation.
    Ptr {
        null: Nullness,
        size: Interval,
    },
    /// Could be an integer or a pointer.
    Top,
}

impl Value {
    pub fn int_or_top(self) -> Interval {
        match self {
            Value::Int(i) => i,
            _ => Interval::TOP,
        }
    }

    pub fn join(self, o: Value) -> Value {
        match (self, o) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a.join(b)),
            (Value::Ptr { null: a, size: s }, Value::Ptr { null: b, size: t }) => Value::Ptr {
                null: a.join(b),
                size: s.join(t),
            },
            _ => Value::Top,
        }
    }

    pub fn widen(self, next: Value, thresholds: &[i128]) -> Value {
        match (self, next) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a.widen(b, thresholds)),
            (Value::Ptr { null: a, size: s }, Value::Ptr { null: b, size: t }) => Value::Ptr {
                null: a.join(b),
                size: s.widen(t, thresholds),
            },
            _ => Value::Top,
        }
    }
}
