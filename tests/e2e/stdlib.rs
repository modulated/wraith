//! Runtime (emulator) tests for standard-library primitives that require real
//! execution: block memory copy/fill (std/mem.wr) and the math helpers
//! widening-multiply, divmod, and the PRNG (std/math.wr).

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// std/mem.wr — memcpy / memcpy16 / memset / memset16
// ---------------------------------------------------------------------------

#[test]
fn memcpy_copies_bytes_indirect() {
    // Fill a source region, copy it elsewhere, and confirm the destination
    // holds the same bytes — i.e. the copy dereferences the pointers.
    let mut e = run(r#"
        import { memcpy } from "std/mem.wr";
        const S0: addr = 0x0500;
        const S1: addr = 0x0501;
        const S2: addr = 0x0502;
        const S3: addr = 0x0503;
        #[reset]
        fn main() {
            S0 = 0x11; S1 = 0x22; S2 = 0x33; S3 = 0x44;
            memcpy(0x0600 as &u8, 0x0500 as &u8, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0600), 0x11);
    assert_eq!(e.mem(0x0601), 0x22);
    assert_eq!(e.mem(0x0602), 0x33);
    assert_eq!(e.mem(0x0603), 0x44);
}

#[test]
fn memcpy16_copies_across_page_boundary() {
    // Copy a run that straddles a page boundary (0x05FE..0x0601 -> 0x06FE..),
    // which only works if the 16-bit pointer/counter logic carries correctly.
    let mut e = run(r#"
        import { memcpy16 } from "std/mem.wr";
        const P0: addr = 0x05FE;
        const P1: addr = 0x05FF;
        const P2: addr = 0x0600;
        const P3: addr = 0x0601;
        #[reset]
        fn main() {
            P0 = 0xDE; P1 = 0xAD; P2 = 0xBE; P3 = 0xEF;
            memcpy16(0x06FE as &u8, 0x05FE as &u8, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x06FE), 0xDE, "byte before page boundary");
    assert_eq!(e.mem(0x06FF), 0xAD, "last byte of page");
    assert_eq!(e.mem(0x0700), 0xBE, "first byte of next page");
    assert_eq!(e.mem(0x0701), 0xEF, "second byte of next page");
}

#[test]
fn memset_fills_region() {
    let mut e = run(r#"
        import { memset } from "std/mem.wr";
        #[reset]
        fn main() {
            memset(0x0600 as &u8, 0x5A, 5);
            loop {}
        }
    "#);
    for i in 0..5u16 {
        assert_eq!(e.mem(0x0600 + i), 0x5A, "byte {i}");
    }
    // One past the end must be untouched (still 0).
    assert_eq!(e.mem(0x0605), 0x00, "one past end untouched");
}

#[test]
fn memset16_fills_across_page_boundary() {
    let mut e = run(r#"
        import { memset16 } from "std/mem.wr";
        #[reset]
        fn main() {
            memset16(0x05FE as &u8, 0x99, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x05FE), 0x99);
    assert_eq!(e.mem(0x05FF), 0x99);
    assert_eq!(e.mem(0x0600), 0x99);
    assert_eq!(e.mem(0x0601), 0x99);
    assert_eq!(e.mem(0x0602), 0x00, "one past end untouched");
}

#[test]
fn str_copy_writes_string_bytes_to_buffer() {
    // str_copy must skip the 1-byte length prefix and write the character bytes
    // (dereferencing the destination pointer), returning the count copied.
    let mut e = run(r#"
        import { str_copy } from "std/mem.wr";
        const N: addr = 0x0410;
        #[reset]
        fn main() {
            let s: str = "ABC";
            let n: u16 = str_copy(0x0600 as &u8, 0x0010, s);
            N = n.low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0600), 0x41, "'A' at dest[0]");
    assert_eq!(e.mem(0x0601), 0x42, "'B' at dest[1]");
    assert_eq!(e.mem(0x0602), 0x43, "'C' at dest[2]");
    assert_eq!(e.mem(0x0410), 3, "returned copied count = 3");
}

#[test]
fn memcpy_accepts_a_local_buffer_by_reference() {
    // The point of the `&u8` signatures. Before this, a caller with a local
    // buffer had nothing to hand memcpy: an array variable is not a `u16`, so
    // the buffer had to be a `static` or the address had to be smuggled
    // through as a bare number the compiler could not check.
    let mut e = run(r#"
        import { memcpy } from "std/mem.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[reset]
        fn main() {
            let src: [u8; 4] = [0; 4];
            let dst: [u8; 4] = [0; 4];
            src[0] = 0x11; src[1] = 0x22; src[2] = 0x33;
            memcpy(&dst, &src, 3);
            R0 = dst[0];
            R1 = dst[2];
            R2 = dst[3];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x11, 0x33));
    assert_eq!(e.mem(0x0902), 0x00, "the fourth byte was not copied");
}

#[test]
fn memset_accepts_a_local_buffer_by_reference() {
    let mut e = run(r#"
        import { memset } from "std/mem.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            memset(&buf, 0x7E, 2);
            R0 = buf[1];
            R1 = buf[2];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x7E, 0x00));
}

#[test]
fn str_copy_writes_into_a_local_buffer() {
    let mut e = run(r#"
        import { str_copy } from "std/mem.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            let s: str = "Hi";
            let n: u16 = str_copy(&buf, 0x0008, s);
            R0 = buf[0];
            R1 = n.low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x48, "'H'");
    assert_eq!(e.mem(0x0901), 2);
}

// ---------------------------------------------------------------------------
// std/math.wr — mul_wide / divmod / rand
// ---------------------------------------------------------------------------

#[test]
fn mul_wide_produces_full_u16_product() {
    let prod = |a: u8, b: u8| {
        let src = format!(
            r#"
            import {{ mul_wide }} from "std/math.wr";
            const LO: addr = 0x0400;
            const HI: addr = 0x0401;
            #[reset]
            fn main() {{
                let p: u16 = mul_wide({a}, {b});
                LO = p.low;
                HI = p.high;
                loop {{}}
            }}
        "#
        );
        run(&src).mem16(0x0400)
    };
    assert_eq!(prod(200, 3), 600, "200*3 = 600 (exceeds u8)");
    assert_eq!(prod(255, 255), 65025, "max u8 product");
    assert_eq!(prod(0, 42), 0, "zero product");
    assert_eq!(prod(16, 16), 256, "16*16 = 256 (just over u8)");
}

#[test]
fn divmod_returns_quotient_and_remainder() {
    let qr = |a: u8, b: u8| {
        let src = format!(
            r#"
            import {{ divmod }} from "std/math.wr";
            const Q: addr = 0x0400;
            const R: addr = 0x0401;
            #[reset]
            fn main() {{
                let d: u16 = divmod({a}, {b});
                Q = d.low;
                R = d.high;
                loop {{}}
            }}
        "#
        );
        let mut e = run(&src);
        (e.mem(0x0400), e.mem(0x0401))
    };
    assert_eq!(qr(23, 5), (4, 3), "23 / 5 = 4 rem 3");
    assert_eq!(qr(100, 10), (10, 0), "100 / 10 = 10 rem 0");
    assert_eq!(qr(7, 8), (0, 7), "7 / 8 = 0 rem 7");
    assert_eq!(qr(255, 16), (15, 15), "255 / 16 = 15 rem 15");
    assert_eq!(qr(42, 0), (0xFF, 0xFF), "divide by zero -> 0xFFFF");
}

#[test]
fn rand_is_deterministic_and_varies() {
    // With a fixed seed the sequence is reproducible; consecutive draws differ
    // (the LFSR advances) and stay in range. Emit four draws to four addresses.
    let mut e = run(r#"
        import { rand, srand } from "std/math.wr";
        const R0: addr = 0x0400;
        const R1: addr = 0x0401;
        const R2: addr = 0x0402;
        const R3: addr = 0x0403;
        #[reset]
        fn main() {
            srand(0x1234);
            R0 = rand();
            R1 = rand();
            R2 = rand();
            R3 = rand();
            loop {}
        }
    "#);
    let seq = [e.mem(0x0400), e.mem(0x0401), e.mem(0x0402), e.mem(0x0403)];
    // Not all equal (the generator actually advances).
    assert!(
        !(seq[0] == seq[1] && seq[1] == seq[2] && seq[2] == seq[3]),
        "rand() must advance, got {:?}",
        seq
    );

    // Same seed -> identical sequence (determinism).
    let mut e2 = run(r#"
        import { rand, srand } from "std/math.wr";
        const R0: addr = 0x0400;
        const R1: addr = 0x0401;
        #[reset]
        fn main() {
            srand(0x1234);
            R0 = rand();
            R1 = rand();
            loop {}
        }
    "#);
    assert_eq!(e2.mem(0x0400), seq[0], "deterministic first draw");
    assert_eq!(e2.mem(0x0401), seq[1], "deterministic second draw");
}

#[test]
fn xorshift32_matches_the_reference_for_a_known_seed() {
    // Marsaglia xorshift32 with seed 1: x ^= x<<13 -> 0x2001; x ^= x>>17 ->
    // 0x2001; x ^= x<<5 -> 0x00042021. The low 16 bits are 0x2021, so the
    // first byte is 0x21 and the first 16-bit low/high are 0x21/0x20.
    let mut e = run(r#"
        import { xrand16, xsrand } from "std/math.wr";
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            xsrand(1 as u16, 0 as u16);
            let v: u16 = xrand16();
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 0x21, "low byte of xorshift32(1)");
    assert_eq!(e.mem(0x0401), 0x20, "high byte of xorshift32(1)");
}

#[test]
fn xorshift32_is_deterministic_and_varies() {
    // A fixed seed gives a reproducible sequence; consecutive draws differ.
    let draw = || {
        run(r#"
            import { xrand, xsrand } from "std/math.wr";
            const R0: addr = 0x0400;
            const R1: addr = 0x0401;
            const R2: addr = 0x0402;
            const R3: addr = 0x0403;
            #[reset]
            fn main() {
                xsrand(0x1234 as u16, 0x5678 as u16);
                R0 = xrand();
                R1 = xrand();
                R2 = xrand();
                R3 = xrand();
                loop {}
            }
        "#)
    };
    let mut a = draw();
    let seq = [a.mem(0x0400), a.mem(0x0401), a.mem(0x0402), a.mem(0x0403)];
    assert!(
        !(seq[0] == seq[1] && seq[1] == seq[2] && seq[2] == seq[3]),
        "xrand() must advance, got {:?}",
        seq
    );
    // Same seed -> identical sequence.
    let mut b = draw();
    assert_eq!(
        [b.mem(0x0400), b.mem(0x0401), b.mem(0x0402), b.mem(0x0403)],
        seq,
        "xorshift32 must be deterministic for a fixed seed"
    );
}

#[test]
fn xorshift32_self_seeds_without_an_explicit_seed() {
    // Never calling xsrand still yields a varying, nonzero stream (the state
    // self-seeds to a fixed nonzero constant on first use).
    let mut e = run(r#"
        import { xrand } from "std/math.wr";
        const R0: addr = 0x0400;
        const R1: addr = 0x0401;
        #[reset]
        fn main() {
            R0 = xrand();
            R1 = xrand();
            loop {}
        }
    "#);
    let (r0, r1) = (e.mem(0x0400), e.mem(0x0401));
    assert!(
        r0 != 0 || r1 != 0,
        "self-seeded stream must not be all zero"
    );
    assert_ne!(r0, r1, "consecutive self-seeded draws should differ");
}

#[test]
fn xorshift32_zero_seed_is_replaced() {
    // xorshift32 is stuck at zero; xsrand(0, 0) must substitute a nonzero state
    // so the generator still advances.
    let mut e = run(r#"
        import { xrand16, xsrand } from "std/math.wr";
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            xsrand(0 as u16, 0 as u16);
            let v: u16 = xrand16();
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    // A stuck-at-zero generator would return 0; a replaced seed advances.
    assert!(
        e.mem(0x0400) != 0 || e.mem(0x0401) != 0,
        "zero seed must be replaced, not left stuck at zero"
    );
}

#[test]
fn memcmp_compares_the_buffers_not_the_pointers() {
    // memcmp used to substitute `{a}` — the *frame slot* holding the pointer —
    // straight into `LDA {a},Y`, so it compared two zero-page frame bytes and
    // never touched the buffers at all. Equal buffers that differ only in their
    // last byte are the case that exposes it: a comparison that never reads
    // them returns the same answer either way.
    let cmp = |b3: u8| {
        run(&format!(
            r#"
            import {{ memcmp }} from "std/mem.wr";
            const A0: addr = 0x0500;
            const A1: addr = 0x0501;
            const A2: addr = 0x0502;
            const A3: addr = 0x0503;
            const B0: addr = 0x0600;
            const B1: addr = 0x0601;
            const B2: addr = 0x0602;
            const B3: addr = 0x0603;
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                A0 = 0x11; A1 = 0x22; A2 = 0x33; A3 = 0x44;
                B0 = 0x11; B1 = 0x22; B2 = 0x33; B3 = {b3};
                OUT = memcmp(0x0500 as &u8, 0x0600 as &u8, 4);
                loop {{}}
            }}
        "#
        ))
        .mem(0x0900)
    };
    assert_eq!(cmp(0x44), 1, "identical buffers compare equal");
    assert_eq!(cmp(0x99), 0, "a difference in the last byte is found");
}

#[test]
fn memcmp_finds_a_difference_in_the_first_byte() {
    let mut e = run(r#"
        import { memcmp } from "std/mem.wr";
        const A0: addr = 0x0500;
        const B0: addr = 0x0600;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            A0 = 0x11;
            B0 = 0x22;
            OUT = memcmp(0x0500 as &u8, 0x0600 as &u8, 1);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0);
}

#[test]
fn memcmp_of_zero_length_is_equal() {
    let mut e = run(r#"
        import { memcmp } from "std/mem.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { OUT = memcmp(0x0500 as &u8, 0x0600 as &u8, 0); loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 1, "nothing to compare, so equal");
}

// ---------------------------------------------------------------------------
// std/math.wr — bit helpers (NMOS-legal, no scratch-pool clobber)
// ---------------------------------------------------------------------------

#[test]
fn set_bit_sets_only_the_named_bit() {
    // The 65C02 SMB version could not even assemble on the NMOS target and
    // staged through zp $20 — the compiler's own scratch pool.
    let mut e = run(r#"
        import { set_bit } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = set_bit(0, 3);
            R1 = set_bit(0xF0, 1);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x08, 0xF2));
}

#[test]
fn clear_bit_clears_only_the_named_bit() {
    let mut e = run(r#"
        import { clear_bit } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = clear_bit(0xFF, 7);
            R1 = clear_bit(0xFF, 0);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x7F, 0xFE));
}

#[test]
fn test_bit_reports_set_and_clear() {
    let mut e = run(r#"
        import { test_bit } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = test_bit(0x10, 4);
            R1 = test_bit(0x10, 5);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 0));
}

// ---------------------------------------------------------------------------
// std/math.wr — saturating arithmetic and bit rotation
// ---------------------------------------------------------------------------

#[test]
fn saturating_add_caps_at_255() {
    let mut e = run(r#"
        import { saturating_add } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = saturating_add(200, 100);
            R1 = saturating_add(10, 20);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (255, 30));
}

#[test]
fn saturating_sub_floors_at_0() {
    let mut e = run(r#"
        import { saturating_sub } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = saturating_sub(10, 20);
            R1 = saturating_sub(50, 20);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0, 30));
}

#[test]
fn count_bits_counts_set_bits() {
    let mut e = run(r#"
        import { count_bits } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[reset]
        fn main() {
            R0 = count_bits(0xB4);
            R1 = count_bits(0);
            R2 = count_bits(0xFF);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (4, 0, 8));
}

#[test]
fn reverse_bits_mirrors_the_byte() {
    let mut e = run(r#"
        import { reverse_bits } from "std/math.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            R0 = reverse_bits(0x80);
            R1 = reverse_bits(0xB4);
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x01, 0x2D));
}

#[test]
fn swap_nibbles_exchanges_halves() {
    let mut e = run(r#"
        import { swap_nibbles } from "std/math.wr";
        const R0: addr = 0x0900;
        #[reset]
        fn main() {
            R0 = swap_nibbles(0xAB);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0xBA);
}

// ---------------------------------------------------------------------------
// std/mem.wr — absolute read/write/jump
// ---------------------------------------------------------------------------

#[test]
fn mem_write_then_mem_read_round_trips() {
    let mut e = run(r#"
        import { mem_read, mem_write } from "std/mem.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            mem_write(0x0950, 42);
            OUT = mem_read(0x0950);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn mem_jump_transfers_control_and_the_target_returns() {
    // mem_jump JMPs; the target's own RTS lands back at mem_jump's call
    // site. Both writes must happen, in order.
    let mut e = run(r#"
        import { mem_jump } from "std/mem.wr";
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        fn target() {
            R0 = 42;
        }
        #[reset]
        fn main() {
            mem_jump(target as u16);
            R1 = 99;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (42, 99));
}

// ---------------------------------------------------------------------------
// std/intrinsics.wr
// ---------------------------------------------------------------------------

#[test]
fn intrinsics_disable_interrupts_masks_irq_and_enable_rearms_it() {
    // SEI then CLI, both from the library rather than hand-written asm.
    let mut e = run(r#"
        import { disable_interrupts, enable_interrupts } from "std/intrinsics.wr";
        const OUT: addr = 0x0900;
        #[irq]
        fn on_irq() {
            OUT = OUT + 1;
        }
        #[reset]
        fn main() {
            OUT = 0;
            disable_interrupts();
            loop {}
        }
    "#);
    assert!(
        e.irq_stays_masked(),
        "disable_interrupts() must mask the IRQ line"
    );

    let mut e = run(r#"
        import { enable_interrupts } from "std/intrinsics.wr";
        const OUT: addr = 0x0900;
        #[irq]
        fn on_irq() {
            OUT = OUT + 1;
        }
        #[reset]
        fn main() {
            OUT = 0;
            enable_interrupts();
            loop {}
        }
    "#);
    e.pulse_irq();
    assert_eq!(e.mem(0x0900), 1, "enable_interrupts() must unmask");
}

#[test]
fn intrinsics_set_and_clear_carry_are_observable() {
    let mut e = run(r#"
        import { set_carry, clear_carry } from "std/intrinsics.wr";
        const OUT: addr = 0x0900;
        const OUT2: addr = 0x0901;
        #[reset]
        fn main() {
            let r1: u8 = 0;
            let r2: u8 = 0;
            set_carry();
            asm {
                "LDA #$00",
                "ADC #$00",
                "STA {r1}"
            }
            clear_carry();
            asm {
                "LDA #$00",
                "ADC #$00",
                "STA {r2}"
            }
            OUT = r1;
            OUT2 = r2;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 0));
}

#[test]
fn intrinsics_nop_is_a_no_op() {
    let mut e = run(r#"
        import { nop } from "std/intrinsics.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = 7;
            nop();
            nop();
            OUT = OUT + 1;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 8);
}

// ---------------------------------------------------------------------------
// std/char.wr — ASCII classification and conversion. A string is an array of
// `char`, so these are exercised over `str` indexing.
// ---------------------------------------------------------------------------

#[test]
fn char_classification_over_a_string() {
    // s = "a5 " : letter, digit, space.
    let mut e = run(r#"
        import { is_digit, is_alpha, is_upper, is_lower, is_alnum, is_whitespace } from "std/char.wr";
        const D_A: addr = 0x0500; const D_5: addr = 0x0501; const D_SP: addr = 0x0502;
        const AL_A: addr = 0x0503;
        const UP_A: addr = 0x0504; const LO_A: addr = 0x0505;
        const AN_5: addr = 0x0506; const AN_SP: addr = 0x0507;
        const WS_SP: addr = 0x0508; const WS_A: addr = 0x0509;
        #[reset]
        fn main() {
            let s: str = "a5 ";
            D_A  = is_digit(s[0]) as u8;      // 'a' -> 0
            D_5  = is_digit(s[1]) as u8;      // '5' -> 1
            D_SP = is_digit(s[2]) as u8;      // ' ' -> 0
            AL_A = is_alpha(s[0]) as u8;      // 'a' -> 1
            UP_A = is_upper(s[0]) as u8;      // 'a' -> 0
            LO_A = is_lower(s[0]) as u8;      // 'a' -> 1
            AN_5 = is_alnum(s[1]) as u8;      // '5' -> 1
            AN_SP= is_alnum(s[2]) as u8;      // ' ' -> 0
            WS_SP= is_whitespace(s[2]) as u8; // ' ' -> 1
            WS_A = is_whitespace(s[0]) as u8; // 'a' -> 0
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 0, "is_digit('a')");
    assert_eq!(e.mem(0x0501), 1, "is_digit('5')");
    assert_eq!(e.mem(0x0502), 0, "is_digit(' ')");
    assert_eq!(e.mem(0x0503), 1, "is_alpha('a')");
    assert_eq!(e.mem(0x0504), 0, "is_upper('a')");
    assert_eq!(e.mem(0x0505), 1, "is_lower('a')");
    assert_eq!(e.mem(0x0506), 1, "is_alnum('5')");
    assert_eq!(e.mem(0x0507), 0, "is_alnum(' ')");
    assert_eq!(e.mem(0x0508), 1, "is_whitespace(' ')");
    assert_eq!(e.mem(0x0509), 0, "is_whitespace('a')");
}

#[test]
fn char_case_conversion_is_idempotent_on_non_letters() {
    let mut e = run(r#"
        import { to_upper, to_lower } from "std/char.wr";
        const U_a: addr = 0x0500;  // to_upper('a') = 'A'
        const U_A: addr = 0x0501;  // to_upper('A') = 'A' (unchanged)
        const U_5: addr = 0x0502;  // to_upper('5') = '5' (unchanged)
        const L_Z: addr = 0x0503;  // to_lower('Z') = 'z'
        const L_z: addr = 0x0504;  // to_lower('z') = 'z' (unchanged)
        #[reset]
        fn main() {
            U_a = to_upper('a') as u8;
            U_A = to_upper('A') as u8;
            U_5 = to_upper('5') as u8;
            L_Z = to_lower('Z') as u8;
            L_z = to_lower('z') as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), b'A');
    assert_eq!(e.mem(0x0501), b'A');
    assert_eq!(e.mem(0x0502), b'5');
    assert_eq!(e.mem(0x0503), b'z');
    assert_eq!(e.mem(0x0504), b'z');
}

#[test]
fn digit_value_parses_and_validates() {
    let mut e = run(r#"
        import { digit_value } from "std/char.wr";
        const V0: addr = 0x0500; const V9: addr = 0x0501; const VX: addr = 0x0502;
        #[reset]
        fn main() {
            V0 = digit_value('0');   // 0
            V9 = digit_value('9');   // 9
            VX = digit_value('x');   // 0xFF (not a digit)
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 0);
    assert_eq!(e.mem(0x0501), 9);
    assert_eq!(e.mem(0x0502), 0xFF);
}

#[test]
fn digit_value_folds_a_string_into_a_number() {
    // "123" -> 123 via the natural accumulate-in-a-loop idiom. The body (inlined
    // digit_value plus a multiply) is large enough that the foreach exit branch
    // is far; the invert-over-JMP loop exit keeps it in range.
    let mut e = run(r#"
        import { digit_value } from "std/char.wr";
        const OUT: addr = 0x0500;
        #[reset]
        fn main() {
            let s: str = "123";
            let acc: u8 = 0;
            for c in s {
                acc = acc * 10 + digit_value(c);
            }
            OUT = acc;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 123);
}

// ---------------------------------------------------------------------------
// std/math.wr — abs / abs16
// ---------------------------------------------------------------------------

#[test]
fn abs_of_i8() {
    let mut e = run(r#"
        import { abs } from "std/math.wr";
        const NEG: addr = 0x0500; const POS: addr = 0x0501; const ZERO: addr = 0x0502;
        #[reset]
        fn main() {
            let n: i8 = -42;
            NEG  = abs(n) as u8;      // 42
            let p: i8 = 42;
            POS  = abs(p) as u8;      // 42
            let z: i8 = 0;
            ZERO = abs(z) as u8;      // 0
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 42, "abs(-42)");
    assert_eq!(e.mem(0x0501), 42, "abs(42)");
    assert_eq!(e.mem(0x0502), 0, "abs(0)");
}

#[test]
fn abs16_of_i16() {
    // -300 = 0xFED4; abs is 300 = 0x012C.
    let mut e = run(r#"
        import { abs16 } from "std/math.wr";
        const LO: addr = 0x0500; const HI: addr = 0x0501;
        #[reset]
        fn main() {
            let n: i16 = 0 as i16 - 300 as i16;
            let a: u16 = abs16(n) as u16;
            LO = a as u8;             // 0x2C
            HI = (a / 256 as u16) as u8;  // 0x01
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 0x2C, "low byte of |−300| = 300");
    assert_eq!(e.mem(0x0501), 0x01, "high byte of 300");
}

// ---------------------------------------------------------------------------
// std/string.wr — strcmp (ordering; `==`/`!=` already handle equality)
// ---------------------------------------------------------------------------

#[test]
fn strcmp_orders_strings_lexicographically() {
    // Returned i8 is read back as its byte: -1 is 0xFF.
    let cmp = |a: &str, b: &str| {
        let src = format!(
            r#"
            import {{ strcmp }} from "std/string.wr";
            const OUT: addr = 0x0500;
            #[reset]
            fn main() {{
                OUT = strcmp("{a}", "{b}") as u8;
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0500)
    };
    assert_eq!(cmp("abc", "abc"), 0, "equal strings");
    assert_eq!(cmp("abc", "abd"), 0xFF, "a < b at last char -> -1");
    assert_eq!(cmp("abd", "abc"), 1, "a > b at last char -> 1");
    assert_eq!(cmp("ab", "abc"), 0xFF, "a is a proper prefix -> -1");
    assert_eq!(cmp("abc", "ab"), 1, "b is a proper prefix -> 1");
    assert_eq!(cmp("", ""), 0, "two empty strings");
    assert_eq!(cmp("", "x"), 0xFF, "empty sorts before non-empty");
    assert_eq!(
        cmp("Z", "a"),
        0xFF,
        "uppercase 'Z'(0x5A) < lowercase 'a'(0x61)"
    );
}

// ---------------------------------------------------------------------------
// std/string.wr — BCD <-> string
// ---------------------------------------------------------------------------

/// Format `bcd_lit` (a BCD literal like "42 as b8") and return (len, buf bytes).
fn bcd8_str(bcd_lit: &str) -> (u8, [u8; 3]) {
    let src = format!(
        r#"
        import {{ bcd_to_string }} from "std/string.wr";
        const L: addr = 0x0500;
        const B0: addr = 0x0501; const B1: addr = 0x0502; const B2: addr = 0x0503;
        #[reset]
        fn main() {{
            let buf: [u8; 3] = [0; 3];
            let n: u16 = bcd_to_string({bcd_lit}, &buf);
            L = n.low; B0 = buf[0]; B1 = buf[1]; B2 = buf[2];
            loop {{}}
        }}
    "#
    );
    let mut e = run(&src);
    (e.mem(0x0500), [e.mem(0x0501), e.mem(0x0502), e.mem(0x0503)])
}

#[test]
fn bcd_to_string_formats_a_bcd_byte() {
    // buf[0] is the length prefix; buf[1..] are ASCII digits.
    assert_eq!(bcd8_str("42 as b8"), (2, [2, b'4', b'2']));
    assert_eq!(
        bcd8_str("5 as b8"),
        (1, [1, b'5', 0]),
        "single digit, no leading 0"
    );
    assert_eq!(bcd8_str("0 as b8"), (1, [1, b'0', 0]), "zero yields \"0\"");
    assert_eq!(bcd8_str("99 as b8"), (2, [2, b'9', b'9']));
    assert_eq!(
        bcd8_str("10 as b8"),
        (2, [2, b'1', b'0']),
        "trailing zero kept"
    );
}

/// Format `bcd_lit` (a b16 literal) and return (len, first 5 buf bytes).
fn bcd16_str(bcd_lit: &str) -> (u8, [u8; 5]) {
    let src = format!(
        r#"
        import {{ bcd16_to_string }} from "std/string.wr";
        const L: addr = 0x0500;
        const B0: addr = 0x0501; const B1: addr = 0x0502; const B2: addr = 0x0503;
        const B3: addr = 0x0504; const B4: addr = 0x0505;
        #[reset]
        fn main() {{
            let buf: [u8; 5] = [0; 5];
            let n: u16 = bcd16_to_string({bcd_lit}, &buf);
            L = n.low;
            B0 = buf[0]; B1 = buf[1]; B2 = buf[2]; B3 = buf[3]; B4 = buf[4];
            loop {{}}
        }}
    "#
    );
    let mut e = run(&src);
    (
        e.mem(0x0500),
        [
            e.mem(0x0501),
            e.mem(0x0502),
            e.mem(0x0503),
            e.mem(0x0504),
            e.mem(0x0505),
        ],
    )
}

#[test]
fn bcd16_to_string_formats_a_bcd_word() {
    assert_eq!(bcd16_str("1234 as b16"), (4, [4, b'1', b'2', b'3', b'4']));
    assert_eq!(
        bcd16_str("1024 as b16"),
        (4, [4, b'1', b'0', b'2', b'4']),
        "interior zero is kept"
    );
    assert_eq!(
        bcd16_str("7 as b16"),
        (1, [1, b'7', 0, 0, 0]),
        "leading zeros suppressed"
    );
    assert_eq!(bcd16_str("50 as b16"), (2, [2, b'5', b'0', 0, 0]));
    assert_eq!(
        bcd16_str("0 as b16"),
        (1, [1, b'0', 0, 0, 0]),
        "zero yields \"0\""
    );
    assert_eq!(bcd16_str("9999 as b16"), (4, [4, b'9', b'9', b'9', b'9']));
}

#[test]
fn string_to_bcd_round_trips_through_bcd16_to_string() {
    // Parse a decimal string to BCD, format it back, and confirm the digits.
    let src = r#"
        import { string_to_bcd, bcd16_to_string } from "std/string.wr";
        const L: addr = 0x0500;
        const B1: addr = 0x0501; const B2: addr = 0x0502;
        const B3: addr = 0x0503; const B4: addr = 0x0504;
        #[reset]
        fn main() {
            let v: b16 = string_to_bcd("2048");
            let buf: [u8; 5] = [0; 5];
            let n: u16 = bcd16_to_string(v, &buf);
            L = n.low;
            B1 = buf[1]; B2 = buf[2]; B3 = buf[3]; B4 = buf[4];
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(e.mem(0x0500), 4, "\"2048\" is four digits");
    assert_eq!(
        [e.mem(0x0501), e.mem(0x0502), e.mem(0x0503), e.mem(0x0504)],
        [b'2', b'0', b'4', b'8']
    );
}
