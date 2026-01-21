# Wraith Compiler Optimization Roadmap

## Peephole Optimizations

### Implemented
- [x] Redundant consecutive loads (`LDA $40; LDA $40` → `LDA $40`)
- [x] Redundant consecutive stores (`STA $40; STA $40` → `STA $40`)
- [x] Load after store elimination (`STA $40; LDA $40` → `STA $40`)
- [x] Dead store elimination (`STA $40; LDA #$05; STA $40` → `LDA #$05; STA $40`)
- [x] NOP operation elimination (`ORA #$00`, `AND #$FF`, `EOR #$00`)
- [x] Redundant transfer elimination (`TAX; TXA` → nothing)
- [x] Unreachable code after terminator (`RTS; JMP label` → `RTS`)

### Planned
- [x] Redundant `LDX` after `STX` to same location (in eliminate_load_after_store)
- [x] Redundant `LDY #$00` when Y is known to be 0 (eliminate_redundant_ldy_zero)
- [x] Remove redundant `CMP #$00` after LDA/AND/ORA/EOR (eliminate_redundant_cmp_zero)
- [ ] Combine address loading (`LDA #<label; LDX #>label` patterns)
- [ ] Strength reduction (`ASL` for multiply by 2, etc.)
- [ ] Branch optimization (invert condition to avoid JMP)
- [ ] Tail call optimization for non-recursive calls

## 65C02 Target Support

The 65C02 processor has additional instructions that can improve code density and performance. These optimizations require a CLI flag to enable.

### CLI Flag
```
--target 6502      # Default: Classic 6502 (NMOS)
--target 65c02     # WDC 65C02 (CMOS)
```

### 65C02-Specific Optimizations

#### Addressing Modes
- [ ] `JMP (addr,X)` - Indexed indirect jump
  - Eliminates need for temporary storage in jump tables
  - Current: `LDA table,X; STA $30; LDA table+1,X; STA $31; JMP ($30)`
  - 65C02: `JMP (table,X)`

#### New Instructions
- [ ] `STZ addr` - Store zero directly
  - Current: `LDA #$00; STA addr`
  - 65C02: `STZ addr`

- [ ] `BRA rel` - Branch always (unconditional relative branch)
  - Saves 1 byte vs `JMP` for short distances (-128 to +127)
  - Current: `JMP label` (3 bytes)
  - 65C02: `BRA label` (2 bytes, if in range)

- [ ] `PHX/PLX`, `PHY/PLY` - Push/pull X and Y directly
  - Current: `TXA; PHA` / `PLA; TAX`
  - 65C02: `PHX` / `PLX`

- [ ] `INC A`, `DEC A` - Increment/decrement accumulator
  - Current: `CLC; ADC #$01` or `SEC; SBC #$01`
  - 65C02: `INC A` or `DEC A`

- [ ] `TSB/TRB addr` - Test and set/reset bits
  - Useful for bit manipulation without affecting other bits

- [ ] `SMB/RMB` - Set/reset memory bit (65C02 variants only)
- [ ] `BBR/BBS` - Branch on bit reset/set (65C02 variants only)

## Code Size Optimizations

- [ ] Consolidate duplicate enum variant data
- [ ] Move inline data to a data section (avoid JMP over data)
- [ ] Dead function elimination
- [ ] Inline small functions automatically

## Future Considerations

- [ ] 65816 target support (16-bit mode)
- [ ] Optimization level flags (`-O0`, `-O1`, `-O2`)
- [ ] Size vs speed trade-off options (`-Os` for size)
