    * = $8000
main:
    LDA $C000
    STA $40
    LDA #$01
    STA $C001
    LDA $40
    STA $C002
    LDA $40
    PHA
    LDA #$01
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
    LDA #$5A
    STA $C001
    JMP end_2
else_1:
    LDA #$7D
    STA $C001
end_2:
    RTS
