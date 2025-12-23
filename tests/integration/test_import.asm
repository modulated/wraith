    ; Imported from addresses.wr
BUTTON = $1001
    ; Imported from addresses.wr
RW_REG = $1002
    ; Imported from addresses.wr
LED = $1000
    * = $8000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $8000
main:
    LDA BUTTON
    STA $40
    LDA #$01
    STA LED
    LDA $40
    STA RW_REG
    LDA #$01
    STA $20
    LDA $40
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
    STA LED
    JMP end_2
else_1:
    LDA #$7D
    STA LED
end_2:
    RTS
