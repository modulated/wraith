Successfully compiled 'tests/test_new_syntax.wr' to 'tests/test_new_syntax.asm'
$0A
    STA $40
    LDA #$00
    STA $41
    JMP arr_skip_2
arr_1:
    .byte $01
    .byte $02
    .byte $03
    .byte $04
arr_skip_2:
    ; Load address of array (4 elements)
    LDA #<arr_1
    LDX #>arr_1
    STA $42
    LDA $40
    PHA
    LDA #$0A
    STA $20
    PLA
    CMP $20
    BEQ eq_true_5
    LDA #$00
    JMP eq_end_6
eq_true_5:
    LDA #$01
eq_end_6:
    CMP #$00
    BEQ else_3
    LDA #$FF
    STA $C000
    JMP end_4
else_3:
end_4:
    RTS
