Successfully compiled 'tests/test_memory_functions.wr' to 'tests/test_memory_functions.asm'
pped TEST_DEST (read-write)
TEST_BUF = $C200
    ; Memory-mapped TEST_BUF (read-write)
test_memset:
    LDA #$00
    STA $40
    ; Call memset with 3 args
    LDA $40
    STA $50
    LDA #$42
    STA $51
    LDA #$0A
    STA $52
    JSR memset
    LDA $C200
    PHA
    LDA #$42
    STA $20
    PLA
    CMP $20
    BEQ eq_true_3
    LDA #$00
    JMP eq_end_4
eq_true_3:
    LDA #$01
eq_end_4:
    CMP #$00
    BEQ else_1
    LDA #$FF
    STA $C100
    JMP end_2
else_1:
end_2:
    RTS
test_memcpy:
    LDA #$11
    STA $C000
    LDA #$00
    STA $41
    LDA #$00
    STA $42
    ; Call memcpy with 3 args
    LDA $42
    STA $50
    LDA $41
    STA $51
    LDA #$01
    STA $52
    JSR memcpy
    LDA $C100
    PHA
    LDA #$11
    STA $20
    PLA
    CMP $20
    BEQ eq_true_7
    LDA #$00
    JMP eq_end_8
eq_true_7:
    LDA #$01
eq_end_8:
    CMP #$00
    BEQ else_5
    LDA #$AA
    STA $C200
    JMP end_6
else_5:
end_6:
    RTS
test_memcmp:
    LDA #$55
    STA $C000
    LDA #$55
    STA $C100
    LDA #$00
    STA $43
    LDA #$00
    STA $44
    ; Call memcmp with 3 args
    LDA $43
    STA $50
    LDA $44
    STA $51
    LDA #$01
    STA $52
    JSR memcmp
    STA $45
    LDA $45
    PHA
    LDA #$01
    STA $20
    PLA
    CMP $20
    BEQ eq_true_11
    LDA #$00
    JMP eq_end_12
eq_true_11:
    LDA #$01
eq_end_12:
    CMP #$00
    BEQ else_9
    LDA #$BB
    STA $C200
    JMP end_10
else_9:
end_10:
    RTS
main:
    ; Call test_memset with 0 args
    JSR test_memset
    ; Call test_memcpy with 0 args
    JSR test_memcpy
    ; Call test_memcmp with 0 args
    JSR test_memcmp
    RTS
