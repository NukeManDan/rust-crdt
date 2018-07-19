//! The `vclock` crate provides a generic vector clock implementation.
//!
//! # Examples
//!
//! ```
//! use crdts::VClock;
//! let (mut a, mut b) = (VClock::new(), VClock::new());
//! a.witness("A".to_string(), 2);
//! b.witness("A".to_string(), 1);
//! assert!(a > b);
//! ```

// TODO: we have a mixture of language here with witness and actor. Clean this up

use super::*;

use std::cmp::{self, Ordering};
use std::collections::{BTreeMap, btree_map};

/// A counter is used to track causality at a particular actor.
pub type Counter = u64;

/// Common Actor type, Actors are unique identifier for every `thing` mutating a VClock.
/// VClock based CRDT's will need to expose this Actor type to the user.
pub trait Actor: Ord + Clone + Send + Serialize + DeserializeOwned {}
impl<A: Ord + Clone + Send + Serialize + DeserializeOwned> Actor for A {}

/// A dot represents the current counter of an actor
#[serde(bound(deserialize = ""))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Dot<A: Actor> {
    pub actor: A,
    pub counter: Counter
}

/// A `VClock` is a standard vector clock.
/// It contains a set of "actors" and associated counters.
/// When a particular actor witnesses a mutation, their associated
/// counter in a `VClock` is incremented. `VClock` is typically used
/// as metadata for associated application data, rather than as the
/// container for application data. `VClock` just tracks causality.
/// It can tell you if something causally descends something else,
/// or if different replicas are "concurrent" (were mutated in
/// isolation, and need to be resolved externally).
#[serde(bound(deserialize = ""))]
#[derive(Debug, Clone, Ord, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VClock<A: Actor> {
    /// dots is the mapping from actors to their associated counters
    pub dots: BTreeMap<A, Counter>,
}

impl<A: Actor> PartialOrd for VClock<A> {
    fn partial_cmp(&self, other: &VClock<A>) -> Option<Ordering> {
        if self == other {
            Some(Ordering::Equal)
        } else if other.dots.iter().all(|(w, c)| {
            self.contains_descendent_element(w, c)
        })
        {
            Some(Ordering::Greater)
        } else if self.dots.iter().all(|(w, c)| {
            other.contains_descendent_element(w, c)
        })
        {
            Some(Ordering::Less)
        } else {
            None
        }
    }
}

impl<A: Actor> VClock<A> {
    /// Returns a new `VClock` instance.
    pub fn new() -> VClock<A> {
        VClock { dots: BTreeMap::new() }
    }

    /// Returns the greatest lower bound of given clocks
    ///
    /// # Examples
    ///
    /// ``` rust
    /// use crdts::VClock;
    /// let (mut a, mut b) = (VClock::new(), VClock::new());
    /// a.witness("A".to_string(), 3);
    /// a.witness("B".to_string(), 6);
    /// b.witness("A".to_string(), 2);
    ///
    /// let glb = VClock::glb(&a, &b);
    ///
    /// assert_eq!(glb.get(&"A".to_string()), 2);
    /// assert_eq!(glb.get(&"B".to_string()), 0);
    /// assert!(a >= glb);
    /// assert!(b >= glb);
    /// ```
    pub fn glb(a: &VClock<A>, b: &VClock<A>) -> VClock<A> {
        let mut glb_vclock = VClock::new();
        for (actor, a_cntr) in a.dots.iter() {
            let min_cntr = cmp::min(b.get(actor), *a_cntr);
            if min_cntr > 0 {
                // 0 is the implied counter if an actor is not in dots, so we don't
                // need to waste memory by storing it
                glb_vclock.dots.insert(actor.clone(), min_cntr);
            }
        }
        glb_vclock
    }

    /// Truncates the VClock to the greatest-lower-bound of the passed
    /// in VClock and it's self
    /// (essentially a mutable version of VClock::glb)
    /// ``` rust
    /// use crdts::VClock;
    /// let mut c = VClock::new();
    /// c.witness(23, 6);
    /// c.witness(89, 14);
    /// let c2 = c.clone();
    ///
    /// c.truncate(&c2); // should be a no-op
    /// assert_eq!(c, c2);
    ///
    /// c.witness(43, 1);
    /// assert_eq!(c.get(&43), 1);
    /// c.truncate(&c2); // should remove the 43 => 1 entry
    /// assert_eq!(c.get(&43), 0);
    /// ```
    pub fn truncate(&mut self, other: &VClock<A>) {
        let mut actors_to_remove: Vec<A> = Vec::new();
        for (actor, count) in self.dots.iter_mut() {
            let min_count = cmp::min(*count, other.get(actor));
            if min_count > 0 {
                *count = min_count
            } else {
                // Since an actor missing from the dots map has an implied counter of 0
                // we can save some memory, and remove the actor.
                actors_to_remove.push(actor.clone())
            }
        }

        // finally, remove all the zero counter actor
        for actor in actors_to_remove {
            self.dots.remove(&actor);
        }
    }

    /// For a particular actor, possibly store a new counter
    /// if it dominates.
    ///
    /// # Examples
    ///
    /// ```
    /// use crdts::VClock;
    /// let (mut a, mut b) = (VClock::new(), VClock::new());
    /// a.witness("A".to_string(), 2);
    /// a.witness("A".to_string(), 0); // ignored because 2 dominates 0
    /// b.witness("A".to_string(), 1);
    /// assert!(a > b);
    /// ```
    ///
    pub fn witness(&mut self, actor: A, counter: Counter) -> Result<()> {
        if !self.contains_descendent_element(&actor, &counter) {
            self.dots.insert(actor, counter);
            Ok(())
        } else {
            Err(Error::ConflictingDot)
        }
    }

    /// For a particular actor, increment the associated counter.
    ///
    /// # Examples
    ///
    /// ```
    /// use crdts::VClock;
    /// let (mut a, mut b) = (VClock::new(), VClock::new());
    /// a.increment("A".to_string());
    /// a.increment("A".to_string());
    /// a.witness("A".to_string(), 0); // ignored because 2 dominates 0
    /// b.increment("A".to_string());
    /// assert!(a > b);
    /// ```
    ///
    pub fn increment(&mut self, actor: A) -> Counter {
        let next = self.get(&actor) + 1;
        self.dots.insert(actor, next);
        next
    }

    /// Merge another vector clock into this one, without
    /// regard to dominance.
    ///
    /// # Examples
    ///
    /// ```
    /// use crdts::VClock;
    /// let (mut a, mut b, mut c) = (VClock::new(), VClock::new(), VClock::new());
    /// a.increment("A".to_string());
    /// b.increment("B".to_string());
    /// c.increment("A".to_string());
    /// c.increment("B".to_string());
    /// a.merge(&b);
    /// assert_eq!(a, c);
    /// ```
    ///
    #[allow(unused_must_use)]
    pub fn merge(&mut self, other: &VClock<A>) {
        for (actor, counter) in other.dots.iter() {
            self.witness(actor.clone(), *counter);
        }
    }

    /// Determine if a single element is present and descendent.
    /// Generally prefer using the higher-level comparison operators
    /// between vclocks over this specific method.
    #[inline]
    pub fn contains_descendent_element(
        &self,
        actor: &A,
        counter: &Counter,
    ) -> bool {
        self.dots
            .get(actor)
            .map(|our_counter| our_counter >= counter)
            .unwrap_or(false)
    }

    /// True if two vector clocks have diverged.
    ///
    /// # Examples
    ///
    /// ```
    /// use crdts::VClock;
    /// let (mut a, mut b) = (VClock::new(), VClock::new());
    /// a.increment("A".to_string());
    /// b.increment("B".to_string());
    /// assert!(a.concurrent(&b));
    /// ```
    pub fn concurrent(&self, other: &VClock<A>) -> bool {
        self.partial_cmp(other).is_none()
    }

    /// Return the associated counter for this actor.
    /// All actors not in the vclock have an implied count of 0
    pub fn get(&self, actor: &A) -> Counter {
        self.dots.get(actor)
            .map(|counter| *counter)
            .unwrap_or(0)
    }

    /// Returns `true` if this vector clock contains nothing.
    pub fn is_empty(&self) -> bool {
        self.dots.is_empty()
    }

    /// Return the dots that self dominates compared to another clock.
    pub fn dominating_dots(
        &self,
        dots: &BTreeMap<A, Counter>,
    ) -> BTreeMap<A, Counter> {
        let mut ret = BTreeMap::new();
        for (actor, counter) in self.dots.iter() {
            let other = dots.get(actor).map(|c| *c).unwrap_or(0);
            if *counter > other {
                ret.insert(actor.clone(), *counter);
            }
        }
        ret
    }

    /// Return a new `VClock` that contains the entries for which we have
    /// a counter that dominates another `VClock`.
    ///
    /// # Examples
    ///
    /// ```
    /// use crdts::VClock;
    /// let (mut a, mut b) = (VClock::new(), VClock::new());
    /// a.witness("A".to_string(), 3);
    /// a.witness("B".to_string(), 2);
    /// a.witness("D".to_string(), 14);
    /// a.witness("G".to_string(), 22);
    ///
    /// b.witness("A".to_string(), 4);
    /// b.witness("B".to_string(), 1);
    /// b.witness("C".to_string(), 1);
    /// b.witness("D".to_string(), 14);
    /// b.witness("E".to_string(), 5);
    /// b.witness("F".to_string(), 2);
    ///
    /// let dom = a.dominating_vclock(&b);
    /// assert_eq!(dom.get(&"B".to_string()), 2);
    /// assert_eq!(dom.get(&"G".to_string()), 22);
    /// ```
    pub fn dominating_vclock(&self, other: &VClock<A>) -> VClock<A> {
        let dots = self.dominating_dots(&other.dots);
        VClock { dots: dots }
    }

    /// Returns the common elements (same actor and counter)
    /// for two `VClock` instances.
    pub fn intersection(&self, other: &VClock<A>) -> VClock<A> {
        let mut dots = BTreeMap::new();
        for (actor, counter) in self.dots.iter() {
            let other_counter = other.get(actor);
            if other_counter == *counter {
                dots.insert(actor.clone(), *counter);
            }
        }
        VClock { dots: dots }
    }

    /// Returns an iterator over the dots in this vclock
    pub fn iter(&self) -> impl Iterator<Item=(&A, &u64)> {
        self.dots.iter()
    }

    // /// Consumes the vclock and returns an iterator over dots in the clock
    // fn into_iter(self) -> impl Iterator<Item=(A, u64)> {
    //     self.dots.into_iter()
    // }

    /// Remove's actors with descendent dots in the given VClock
    pub fn subtract(&mut self, other: &VClock<A>) {
        for (actor, counter) in other.iter() {
            if counter >= &self.get(&actor) {
                self.dots.remove(&actor);
            }
        }
    }
}

impl<A: Actor> std::iter::IntoIterator for VClock<A> {
    type Item = (A, u64);
    type IntoIter = btree_map::IntoIter<A, u64>;
    
    /// Consumes the vclock and returns an iterator over dots in the clock
    fn into_iter(self) -> btree_map::IntoIter<A, u64> {
        self.dots.into_iter()
    }
}

impl<A: Actor> From<Dot<A>> for VClock<A> {
    fn from(dot: Dot<A>) -> VClock<A> {
        let mut clock = VClock::new();
        clock.witness(dot.actor, dot.counter).unwrap(); // this should not fail!
        clock
    }
}

impl<A: Actor> std::iter::FromIterator<(A, u64)> for VClock<A> {
    fn from_iter<I: IntoIterator<Item=(A, u64)>>(iter: I) -> Self {
        let mut clock = Self::new();

        for (actor, counter) in iter {
            clock.witness(actor, counter);
        }

        clock
    }
}

#[cfg(test)]
mod tests {
    extern crate rand;
    extern crate quickcheck;

    use self::quickcheck::{Arbitrary, Gen};
    use super::*;

    impl<A: Actor + Arbitrary> Arbitrary for VClock<A> {
        fn arbitrary<G: Gen>(g: &mut G) -> Self {
            let mut v = VClock::new();
            for _ in 0..g.gen_range(0, 7) {
                let witness = A::arbitrary(g);
                v.witness(witness, g.gen_range(1, 7));
            }
            v
        }

        fn shrink(&self) -> Box<Iterator<Item = VClock<A>>> {
            let mut smaller = vec![];
            for k in self.dots.keys() {
                let mut vc = self.clone();
                vc.dots.remove(k);
                smaller.push(vc)
            }
            Box::new(smaller.into_iter())
        }
    }

    quickcheck! {
        fn prop_from_iter_of_iter_is_nop(clock: VClock<u8>) -> bool {
            clock == clock.clone().into_iter().collect()
        }

        fn prop_from_iter_order_of_dots_should_not_matter(dots: Vec<(u8, u64)>) -> bool {
            // TODO: is there a better way to check comutativity of dots?
            let reverse: VClock<u8> = dots.clone()
                .into_iter()
                .rev()
                .collect();
            let forward: VClock<u8> = dots
                .into_iter()
                .collect();

            reverse == forward
        }

        fn prop_from_iter_dots_should_be_idempotent(dots: Vec<(u8, u64)>) -> bool {
            let single: VClock<u8> = dots.clone()
                .into_iter()
                .collect();

            let double: VClock<u8> = dots.clone()
                .into_iter()
                .chain(dots.into_iter())
                .collect();

            single == double
        }

        fn prop_truncate_self_is_nop(clock: VClock<u8>) -> bool {
            let mut clock_truncated = clock.clone();
            clock_truncated.truncate(&clock);

            clock_truncated == clock
        }

        fn prop_subtract_with_empty_is_nop(clock: VClock<u8>) -> bool {
            let mut subbed  = clock.clone();
            subbed.subtract(&VClock::new());
            subbed == clock
        }

        fn prop_subtract_self_is_empty(clock: VClock<u8>) -> bool {
            let mut subbed  = clock.clone();
            subbed.subtract(&clock);
            subbed == VClock::new()
        }
    }

    #[test]
    fn test_subtract() {
        let mut a: VClock<u8> = vec![(1, 4), (2, 3), (5, 9)].into_iter().collect();
        let     b: VClock<u8> = vec![(1, 5), (2, 3), (5, 8)].into_iter().collect();
        let expected: VClock<u8> = vec![(5, 9)].into_iter().collect();

        a.subtract(&b);

        assert_eq!(a, expected);
    }

    #[test]
    fn test_merge() {
        let mut a: VClock<u8> = vec![(1, 1), (2, 2), (4, 4)].into_iter().collect();
        let b: VClock<u8> = vec![(3, 3), (4, 3)].into_iter().collect();
        a.merge(&b);
        
        let c: VClock<u8> = vec![(1, 1), (2, 2), (3, 3), (4, 4)].into_iter().collect();
        assert_eq!(a, c);
    }

    #[test]
    fn test_merge_less_left() {
        let (mut a, mut b) = (VClock::new(), VClock::new());
        a.witness(5, 5).unwrap();

        b.witness(6, 6).unwrap();
        b.witness(7, 7).unwrap();

        a.merge(&b);
        assert_eq!(a.get(&5), 5);
        assert_eq!(a.get(&6), 6);
        assert_eq!(a.get(&7), 7);
    }

    #[test]
    fn test_merge_less_right() {
        let (mut a, mut b) = (VClock::new(), VClock::new());
        a.witness(6, 6).unwrap();
        a.witness(7, 7).unwrap();

        b.witness(5, 5).unwrap();

        a.merge(&b);
        assert_eq!(a.get(&5), 5);
        assert_eq!(a.get(&6), 6);
        assert_eq!(a.get(&7), 7);
    }

    #[test]
    fn test_merge_same_id() {
        let (mut a, mut b) = (VClock::new(), VClock::new());
        a.witness(1, 1).unwrap();
        a.witness(2, 1).unwrap();

        b.witness(1, 1).unwrap();
        b.witness(3, 1).unwrap();

        a.merge(&b);
        assert_eq!(a.get(&1), 1);
        assert_eq!(a.get(&2), 1);
        assert_eq!(a.get(&3), 1);
    }

    #[test]
    fn test_vclock_ordering() {
        assert_eq!(VClock::<i8>::new(), VClock::new());

        let (mut a, mut b) = (VClock::new(), VClock::new());
        a.witness("A".to_string(), 1).unwrap();
        a.witness("A".to_string(), 2).unwrap();
        assert!(a.witness("A".to_string(), 0).is_err());
        b.witness("A".to_string(), 1).unwrap();

        // a {A:2}
        // b {A:1}
        // expect: a dominates
        assert!(a > b);
        assert!(b < a);
        assert!(a != b);

        b.witness("A".to_string(), 3).unwrap();
        // a {A:2}
        // b {A:3}
        // expect: b dominates
        assert!(b > a);
        assert!(a < b);
        assert!(a != b);

        a.witness("B".to_string(), 1).unwrap();
        // a {A:2, B:1}
        // b {A:3}
        // expect: concurrent
        assert!(a != b);
        assert!(!(a > b));
        assert!(!(b > a));

        a.witness("A".to_string(), 3).unwrap();
        // a {A:3, B:1}
        // b {A:3}
        // expect: a dominates
        assert!(a > b);
        assert!(b < a);
        assert!(a != b);

        b.witness("B".to_string(), 2).unwrap();
        // a {A:3, B:1}
        // b {A:3, B:2}
        // expect: b dominates
        assert!(b > a);
        assert!(a < b);
        assert!(a != b);

        a.witness("B".to_string(), 2).unwrap();
        // a {A:3, B:2}
        // b {A:3, B:2}
        // expect: equal
        assert!(!(b > a));
        assert!(!(a > b));
        assert_eq!(a, b);
    }
}
