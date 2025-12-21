Successfully compiled 'tests/test_hex_literals.wr' to 'tests/test_hex_literals.asm'
    STA $40
    LDA #$FF
    STA $41
    LDA #$00
    STA $42
    LDA #$00
    STA $43
    LDA #$34
    STA $44
    LDA #$CD
    STA $45
    LDA #$D2
    STA $46
    LDA $40
    PHA
    LDA #$40
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
    STA $C000
    JMP end_2
else_1:
end_2:
    RTS
