//! Pointer and aggregate arguments through an indirect call.
//!
//! An indirect call stages its arguments at a fixed address, because the
//! callee — and therefore the callee's frame — is not known until run time.
//! That staging was restricted to primitives, so a driver vtable could not be
//! handed a `&State` and a peripheral could not carry per-instance state: the
//! only thing that fit through the interface was a unit number, with the
//! driver keeping parallel arrays indexed by it.
//!
//! Nothing was behind the restriction. A pointer, a string, an enum and a
//! struct are each two bytes with a settled register convention — the pointer
//! convention, high byte in X — and the callee's prologue copies the staging
//! block into its frame by byte count, so it never cared what the bytes meant.
//!
//! Sema's rule moved with it. It read "only a pointer to global storage may
//! cross an indirect call", justified by frame colouring not seeing indirect
//! calls; colouring now edges an indirect caller to every address-taken
//! function, so that reason is spent. What is left is the hazard a *direct*
//! call has — a callee that stores the pointer somewhere outliving the call —
//! asked of every function the call could reach.

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// The shape this was for: a peripheral with its own state, behind a vtable.
// ---------------------------------------------------------------------------

#[test]
fn each_peripheral_carries_a_pointer_to_its_own_state() {
    // Two peripherals of the same kind on one bus. They share a driver — one
    // set of functions — and are told apart only by the state they are handed.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;

        struct State { count: u8, last: u8 }
        static S0: State = State { count: 0, last: 0 };
        static S1: State = State { count: 0, last: 0 };

        fn sensor_poll(s: &State) -> u8 {
            s.count = s.count + 1;
            s.last = s.count * 10;
            return s.count;
        }

        struct Peripheral { state: &State, poll: fn(&State) -> u8 }
        static PERIPHS: [Peripheral; 2] = [
            Peripheral { state: &S0, poll: sensor_poll },
            Peripheral { state: &S1, poll: sensor_poll },
        ];

        #[reset]
        fn main() {
            let i: u8 = 0;
            PERIPHS[i].poll(PERIPHS[i].state);
            PERIPHS[i].poll(PERIPHS[i].state);
            let j: u8 = 1;
            PERIPHS[j].poll(PERIPHS[j].state);
            OUT0 = S0.count;
            OUT1 = S1.count;
            OUT2 = S0.last;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (2, 1, 20),
        "each instance kept its own state through one shared driver"
    );
}

#[test]
fn a_write_through_the_pointer_reaches_the_callers_storage() {
    // The pointer is an address, not a copy: what the callee writes is what
    // the caller sees.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        struct State { count: u8 }
        static S: State = State { count: 5 };
        fn bump(s: &State) { s.count = s.count + 37; }
        struct Dev { poke: fn(&State) }
        static D: Dev = Dev { poke: bump };
        #[reset]
        fn main() { D.poke(&S); OUT0 = S.count; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

// ---------------------------------------------------------------------------
// The other argument kinds that are two bytes with a settled convention.
// ---------------------------------------------------------------------------

#[test]
fn a_struct_argument_through_an_indirect_call() {
    // Structs go by reference, indirectly as well as directly.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct P { x: u8, y: u8 }
        static PS: [P; 2] = [P { x: 1, y: 2 }, P { x: 30, y: 12 }];
        fn total(p: P) -> u8 { return p.x + p.y; }
        struct Dev { sum: fn(P) -> u8 }
        static D: Dev = Dev { sum: total };
        #[reset]
        fn main() {
            let i: u8 = 1;
            OUT0 = D.sum(PS[i]);
            OUT1 = D.sum(PS[0]);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (42, 3));
}

#[test]
fn a_string_argument_through_an_indirect_call() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const MSG: str = "Hello";
        static N: u8 = 0;
        fn count(s: str) -> u8 {
            let n: u8 = 0;
            for c in s { n = n + 1; }
            return n;
        }
        struct Dev { measure: fn(str) -> u8 }
        static D: Dev = Dev { measure: count };
        #[reset]
        fn main() { N = D.measure(MSG); OUT0 = N; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 5);
}

#[test]
fn mixed_scalar_and_pointer_arguments_stay_in_order() {
    // Staging is packed by size, so a one-byte argument between two-byte ones
    // is where the offsets can drift.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        struct State { v: u8 }
        static S: State = State { v: 7 };
        static A: u8 = 0;
        static B: u8 = 0;
        static C: u8 = 0;
        fn take(first: u8, s: &State, third: u16) {
            A = first;
            B = s.v;
            C = third.low;
        }
        struct Dev { go: fn(u8, &State, u16) }
        static D: Dev = Dev { go: take };
        #[reset]
        fn main() {
            D.go(11, &S, 0x0122);
            OUT0 = A; OUT1 = B; OUT2 = C;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (11, 7, 0x22));
}

// ---------------------------------------------------------------------------
// What the relaxed escape rule must still catch.
// ---------------------------------------------------------------------------

#[test]
fn a_pointer_to_a_local_may_now_cross_an_indirect_call() {
    // The relaxation itself. `sensor_poll` only reads and writes through its
    // parameter, so a pointer to a local is fine — colouring keeps the
    // callee's frame off this one.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        struct State { count: u8 }
        fn bump(s: &State) { s.count = s.count + 1; }
        struct Dev { poke: fn(&State) }
        static D: Dev = Dev { poke: bump };
        #[reset]
        fn main() {
            let local: State = State { count: 40 };
            D.poke(&local);
            D.poke(&local);
            OUT0 = local.count;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42, "the callee wrote the caller's local");
}

#[test]
fn a_pointer_to_a_local_is_still_rejected_when_a_candidate_stores_it() {
    // The hazard that survives: one of the functions this call could reach
    // parks the pointer in a global, which outlives the frame it names. The
    // callee is unknown, so any candidate storing it condemns the call.
    crate::common::assert_error_contains(
        r#"
        struct State { count: u8 }
        static KEPT: &State = 0 as &State;
        fn harmless(s: &State) { s.count = s.count + 1; }
        fn hoards(s: &State) { KEPT = s; }
        struct Dev { poke: fn(&State) }
        static D: Dev = Dev { poke: harmless };
        #[reset]
        fn main() {
            let local: State = State { count: 1 };
            D.poke(&local);
            D.poke = hoards;
            loop {}
        }
    "#,
        "this indirect call could reach 'hoards'",
    );
}

#[test]
fn a_pointer_to_a_global_is_always_fine() {
    // Unchanged by the relaxation: a `static` keeps its address for the life
    // of the program, so no candidate can outlive it.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        struct State { count: u8 }
        static S: State = State { count: 1 };
        static KEPT: &State = 0 as &State;
        fn hoards(s: &State) { KEPT = s; s.count = s.count + 41; }
        struct Dev { poke: fn(&State) }
        static D: Dev = Dev { poke: hoards };
        #[reset]
        fn main() { D.poke(&S); OUT0 = S.count; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn an_array_argument_is_still_rejected_with_a_reason() {
    // Not every type is two bytes with a settled convention. The message says
    // what to do instead rather than naming a category.
    crate::common::assert_error_contains(
        r#"
        fn take(a: [u8; 4]) -> u8 { return a[0]; }
        struct Dev { go: fn([u8; 4]) -> u8 }
        static D: Dev = Dev { go: take };
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let v: [u8; 4] = [1, 2, 3, 4]; OUT = D.go(v); loop {} }
    "#,
        "indirect call cannot take",
    );
}
